//! Confluence → RAG ingestion pipeline (D48).
//!
//! Recursively collects a page tree (or one space, flat) over the Confluence
//! Server/DC REST API and hands each page's raw `body.storage` HTML to the
//! user's RAG service (`rag.rs` — which owns summarization + embedding).
//! Progress streams to the settings UI over a `tauri::ipc::Channel`
//! (`IngestEvent`), mirroring the run engine's transport; cancellation uses a
//! flag registry (`IngestRegistry`, a child-process-free `RunRegistry`).
//!
//! Auth is a Bearer PAT (Server/DC). Atlassian Cloud's Basic `email:api_token`
//! auth is out of scope for v1. The crawl is an iterative BFS with visited-set
//! dedupe and hard page/depth caps; the cancel flag is checked between HTTP
//! calls, so cancellation latency is at most one request timeout.
//!
//! Testability: the JSON parsers are pure `fn(&str)`, and the crawl loop is
//! generic over the `ConfluenceApi` trait + an ingest sink closure, so unit
//! tests drive it with an in-memory fake (the `root: &Path` injection pattern,
//! adapted to HTTP).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::Manager;

use crate::rag::{IngestPage, RagClient};
use crate::settings::ConfluenceConfig;

/// Hard caps so a runaway wiki can't crawl forever (a stop is reported in `End`).
const MAX_PAGES: usize = 2000;
const MAX_DEPTH: usize = 10;
/// Child-listing page size (Confluence caps at ~100 for content APIs).
const PAGE_LIMIT: usize = 100;

/// Progress events streamed to the settings UI. Serialized as
/// `{ "type": "...", ...camelCase }`, like `RunEvent`.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum IngestEvent {
    Started { root_id: String },
    /// One page fetched from Confluence (`fetched` = running count).
    PageFetched { page_id: String, title: String, fetched: u64 },
    /// One page accepted by the RAG service (`ingested` = running count).
    PageIngested { page_id: String, title: String, ingested: u64 },
    /// One page failed (fetch or RAG ingest); the crawl continues.
    PageFailed { page_id: String, title: String, message: String },
    /// Fatal failure (auth, unreachable root) — followed by `End{failed}`.
    Error { message: String },
    /// Terminal: "succeeded" | "failed" | "canceled".
    End { status: String, ingested: u64, failed: u64 },
}

/// A child listing entry (id + title, body fetched separately).
#[derive(Clone, Debug, PartialEq)]
pub struct PageStub {
    pub id: String,
    pub title: String,
}

/// One fetched page with its raw storage body.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfluencePage {
    pub id: String,
    pub title: String,
    pub body_html: String,
    /// Site-relative webui link (`_links.webui`), may be empty.
    pub webui: String,
}

/// Parse `GET /rest/api/content/{id}?expand=body.storage`.
pub fn parse_page(json: &str) -> Result<ConfluencePage, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .ok_or("confluence page response has no id")?
        .to_string();
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
    // Tolerate a missing body (permissions, drafts) — ingest an empty page
    // rather than failing the whole crawl.
    let body_html = v
        .get("body")
        .and_then(|b| b.get("storage"))
        .and_then(|s| s.get("value"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let webui = v
        .get("_links")
        .and_then(|l| l.get("webui"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Ok(ConfluencePage { id, title, body_html, webui })
}

/// Parse a paginated content listing (`child/page` or a space listing):
/// returns the stubs plus whether another page of results should be fetched.
pub fn parse_child_list(json: &str) -> Result<(Vec<PageStub>, bool), String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let results = v
        .get("results")
        .and_then(|x| x.as_array())
        .ok_or("confluence listing has no results array")?;
    let stubs = results
        .iter()
        .filter_map(|r| {
            let id = r.get("id").and_then(|x| x.as_str())?.to_string();
            let title = r.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
            Some(PageStub { id, title })
        })
        .collect::<Vec<_>>();
    let limit = v.get("limit").and_then(|x| x.as_u64()).unwrap_or(PAGE_LIMIT as u64) as usize;
    let has_more = results.len() >= limit && limit > 0;
    Ok((stubs, has_more))
}

/// The HTTP surface the crawl needs — a trait so tests can drive the loop with
/// an in-memory fake.
pub trait ConfluenceApi {
    fn fetch_page(&self, id: &str) -> Result<ConfluencePage, String>;
    fn fetch_children(&self, id: &str, start: usize) -> Result<(Vec<PageStub>, bool), String>;
    fn fetch_space_pages(&self, space_key: &str, start: usize)
        -> Result<(Vec<PageStub>, bool), String>;
}

/// Per-request timeout. Generous (D53): slow corporate Confluence/RAG backends
/// were hitting the previous 30s limit; the cancel flag still bounds
/// cancellation latency to one request.
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Production implementation over `reqwest::blocking` (worker threads only).
struct HttpConfluence {
    base: String,
    token: Option<String>,
    http: reqwest::blocking::Client,
}

impl HttpConfluence {
    fn new(cfg: &ConfluenceConfig) -> Result<Self, String> {
        let base = cfg.base_url.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return Err("Confluence base URL이 설정되지 않았습니다 — 지식 화면에서 등록해 주세요".into());
        }
        let http = reqwest::blocking::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .danger_accept_invalid_certs(cfg.allow_invalid_certs)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { base, token: cfg.token.clone(), http })
    }

    fn get(&self, url: &str) -> Result<String, String> {
        let mut req = self.http.get(url);
        if let Some(token) = self.token.as_deref() {
            req = req.bearer_auth(token);
        }
        let res = req.send().map_err(|e| e.to_string())?;
        let status = res.status();
        if !status.is_success() {
            return Err(format!("HTTP {status} — {url}"));
        }
        res.text().map_err(|e| e.to_string())
    }
}

impl ConfluenceApi for HttpConfluence {
    fn fetch_page(&self, id: &str) -> Result<ConfluencePage, String> {
        let url = format!("{}/rest/api/content/{}?expand=body.storage", self.base, id);
        parse_page(&self.get(&url)?)
    }

    fn fetch_children(&self, id: &str, start: usize) -> Result<(Vec<PageStub>, bool), String> {
        let url = format!(
            "{}/rest/api/content/{}/child/page?limit={PAGE_LIMIT}&start={start}",
            self.base, id
        );
        parse_child_list(&self.get(&url)?)
    }

    fn fetch_space_pages(
        &self,
        space_key: &str,
        start: usize,
    ) -> Result<(Vec<PageStub>, bool), String> {
        let url = format!(
            "{}/rest/api/content?spaceKey={space_key}&type=page&limit={PAGE_LIMIT}&start={start}",
            self.base
        );
        parse_child_list(&self.get(&url)?)
    }
}

/// Crawl outcome — the numbers behind the terminal `End` event.
#[derive(Debug, PartialEq)]
pub struct IngestOutcome {
    /// "succeeded" | "failed" | "canceled".
    pub status: String,
    pub ingested: u64,
    pub failed: u64,
}

/// Collect every result page of a listing call, checking the cancel flag
/// between requests. Returns `None` when canceled.
fn collect_all(
    canceled: &AtomicBool,
    mut fetch: impl FnMut(usize) -> Result<(Vec<PageStub>, bool), String>,
) -> Result<Option<Vec<PageStub>>, String> {
    let mut out = Vec::new();
    let mut start = 0;
    loop {
        if canceled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let (stubs, has_more) = fetch(start)?;
        let n = stubs.len();
        out.extend(stubs);
        if !has_more || n == 0 {
            return Ok(Some(out));
        }
        start += n;
    }
}

/// The crawl loop: iterative BFS from `root_page_id` (or a flat space listing
/// when `space_key` is used instead). `on_page` is the ingest sink (→ RAG); a
/// per-page failure emits `PageFailed` and continues. Fatal failures (root
/// unreachable) emit `Error` and end with status "failed".
pub fn crawl(
    api: &dyn ConfluenceApi,
    root_page_id: Option<&str>,
    space_key: Option<&str>,
    canceled: &AtomicBool,
    mut on_page: impl FnMut(&ConfluencePage) -> Result<(), String>,
    mut emit: impl FnMut(IngestEvent),
) -> IngestOutcome {
    let mut fetched: u64 = 0;
    let mut ingested: u64 = 0;
    let mut failed: u64 = 0;
    let mut visited: HashSet<String> = HashSet::new();
    // (page id, display title, depth) — titles from listings label failures
    // even when the page fetch itself fails.
    let mut queue: VecDeque<(String, String, usize)> = VecDeque::new();

    let end = |status: &str, ingested: u64, failed: u64| IngestOutcome {
        status: status.to_string(),
        ingested,
        failed,
    };

    // Seed the queue.
    if let Some(root) = root_page_id.filter(|r| !r.trim().is_empty()) {
        queue.push_back((root.trim().to_string(), String::new(), 0));
    } else if let Some(space) = space_key.filter(|s| !s.trim().is_empty()) {
        match collect_all(canceled, |start| api.fetch_space_pages(space.trim(), start)) {
            Ok(Some(stubs)) => {
                for s in stubs {
                    queue.push_back((s.id, s.title, 0));
                }
            }
            Ok(None) => return end("canceled", 0, 0),
            Err(e) => {
                emit(IngestEvent::Error { message: format!("스페이스 목록 조회 실패: {e}") });
                return end("failed", 0, 0);
            }
        }
    } else {
        emit(IngestEvent::Error {
            message: "수집 시작점이 없습니다 — 루트 페이지 ID 또는 스페이스 키를 설정해 주세요".into(),
        });
        return end("failed", 0, 0);
    }

    let space_mode = root_page_id.map(|r| r.trim().is_empty()).unwrap_or(true);
    let mut root_failed = false;

    while let Some((id, listed_title, depth)) = queue.pop_front() {
        if canceled.load(Ordering::Relaxed) {
            return end("canceled", ingested, failed);
        }
        if !visited.insert(id.clone()) {
            continue;
        }
        if visited.len() > MAX_PAGES {
            emit(IngestEvent::Error {
                message: format!("페이지 상한({MAX_PAGES})에 도달해 수집을 중단합니다"),
            });
            break;
        }

        // Fetch the page body.
        let page = match api.fetch_page(&id) {
            Ok(p) => p,
            Err(e) => {
                // The tree root being unreachable is fatal (nothing to crawl);
                // any other page failure is recorded and skipped.
                if depth == 0 && !space_mode && fetched == 0 {
                    emit(IngestEvent::Error { message: format!("루트 페이지 조회 실패: {e}") });
                    root_failed = true;
                    break;
                }
                failed += 1;
                emit(IngestEvent::PageFailed { page_id: id, title: listed_title, message: e });
                continue;
            }
        };
        fetched += 1;
        emit(IngestEvent::PageFetched {
            page_id: page.id.clone(),
            title: page.title.clone(),
            fetched,
        });

        // Hand the raw page to the RAG service.
        match on_page(&page) {
            Ok(()) => {
                ingested += 1;
                emit(IngestEvent::PageIngested {
                    page_id: page.id.clone(),
                    title: page.title.clone(),
                    ingested,
                });
            }
            Err(e) => {
                failed += 1;
                emit(IngestEvent::PageFailed {
                    page_id: page.id.clone(),
                    title: page.title.clone(),
                    message: e,
                });
            }
        }

        // Descend (tree mode only; space mode is a flat listing).
        if !space_mode && depth < MAX_DEPTH {
            match collect_all(canceled, |start| api.fetch_children(&page.id, start)) {
                Ok(Some(children)) => {
                    for c in children {
                        if !visited.contains(&c.id) {
                            queue.push_back((c.id, c.title, depth + 1));
                        }
                    }
                }
                Ok(None) => return end("canceled", ingested, failed),
                Err(e) => {
                    failed += 1;
                    emit(IngestEvent::PageFailed {
                        page_id: page.id.clone(),
                        title: page.title.clone(),
                        message: format!("하위 페이지 목록 조회 실패: {e}"),
                    });
                }
            }
        }
    }

    if root_failed {
        end("failed", ingested, failed)
    } else {
        end("succeeded", ingested, failed)
    }
}

/// Like `RunRegistry` but with no child process — just cancel flags, keyed by
/// ingest id. Managed Tauri state.
#[derive(Default)]
pub struct IngestRegistry {
    counter: AtomicU64,
    flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

// ── Tauri commands ────────────────────────────────────────────────────────────

/// Start a crawl+ingest on a worker thread; returns the ingest id immediately.
/// Progress arrives on `on_event`; cancel with `cancel_ingest`.
#[tauri::command]
pub fn start_confluence_ingest(
    app: tauri::AppHandle,
    on_event: Channel<IngestEvent>,
) -> Result<String, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings = crate::settings::load(&config_dir);
    let conf = settings
        .confluence
        .clone()
        .ok_or_else(|| "Confluence가 설정되지 않았습니다 — 지식 화면에서 등록해 주세요".to_string())?;
    let rag_cfg = settings
        .rag
        .clone()
        .ok_or_else(|| "RAG endpoint가 설정되지 않았습니다 — 지식 화면에서 등록해 주세요".to_string())?;

    // Build both clients up front so config errors surface before the thread.
    let api = HttpConfluence::new(&conf)?;
    let rag = RagClient::new(&rag_cfg, conf.allow_invalid_certs)?;
    let base = api.base.clone();

    let registry = app.state::<IngestRegistry>();
    let n = registry.counter.fetch_add(1, Ordering::Relaxed) + 1;
    let ingest_id = format!("ingest-{n}");
    let canceled = Arc::new(AtomicBool::new(false));
    registry
        .flags
        .lock()
        .map_err(|_| "ingest registry poisoned".to_string())?
        .insert(ingest_id.clone(), canceled.clone());

    let root_id = conf.root_page_id.clone().unwrap_or_default();
    let space_key = conf.space_key.clone().unwrap_or_default();
    let app_handle = app.clone();
    let id_for_thread = ingest_id.clone();

    std::thread::spawn(move || {
        let emit = |ev: IngestEvent| {
            let _ = on_event.send(ev);
        };
        emit(IngestEvent::Started {
            root_id: if root_id.is_empty() { space_key.clone() } else { root_id.clone() },
        });
        let outcome = crawl(
            &api,
            Some(root_id.as_str()).filter(|s| !s.is_empty()),
            Some(space_key.as_str()).filter(|s| !s.is_empty()),
            &canceled,
            |page| {
                rag.ingest_page(&IngestPage {
                    id: page.id.clone(),
                    title: page.title.clone(),
                    url: if page.webui.is_empty() {
                        String::new()
                    } else {
                        format!("{base}{}", page.webui)
                    },
                    content_html: page.body_html.clone(),
                })
            },
            emit,
        );
        let _ = on_event.send(IngestEvent::End {
            status: outcome.status,
            ingested: outcome.ingested,
            failed: outcome.failed,
        });
        if let Ok(mut flags) = app_handle.state::<IngestRegistry>().flags.lock() {
            flags.remove(&id_for_thread);
        }
    });

    Ok(ingest_id)
}

/// Cancel a running ingest. The worker notices between HTTP calls and emits
/// `End{status:"canceled"}` (latency ≤ one request timeout).
#[tauri::command]
pub fn cancel_ingest(
    registry: tauri::State<IngestRegistry>,
    ingest_id: String,
) -> Result<(), String> {
    let flags = registry.flags.lock().map_err(|_| "ingest registry poisoned".to_string())?;
    match flags.get(&ingest_id) {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            Ok(())
        }
        None => Err(format!("no such ingest: {ingest_id}")),
    }
}

/// Settings-screen connection test: fetch the configured root page (or the
/// space's first page) and return its title.
#[tauri::command]
pub async fn probe_confluence(app: tauri::AppHandle) -> Result<String, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let conf = crate::settings::load(&config_dir)
        .confluence
        .ok_or_else(|| "Confluence가 설정되지 않았습니다 — 지식 화면에서 등록해 주세요".to_string())?;

    tauri::async_runtime::spawn_blocking(move || {
        let api = HttpConfluence::new(&conf)?;
        if let Some(root) = conf.root_page_id.as_deref().filter(|s| !s.trim().is_empty()) {
            return Ok(api.fetch_page(root.trim())?.title);
        }
        if let Some(space) = conf.space_key.as_deref().filter(|s| !s.trim().is_empty()) {
            let (stubs, _) = api.fetch_space_pages(space.trim(), 0)?;
            return stubs
                .first()
                .map(|s| s.title.clone())
                .ok_or_else(|| "스페이스에 페이지가 없습니다".to_string());
        }
        Err("루트 페이지 ID 또는 스페이스 키를 설정해 주세요".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn parses_page_with_and_without_body() {
        let full = r#"{
            "id": "123", "title": "설계서",
            "body": { "storage": { "value": "<p>본문</p>" } },
            "_links": { "webui": "/pages/viewpage.action?pageId=123" }
        }"#;
        let p = parse_page(full).unwrap();
        assert_eq!(p.id, "123");
        assert_eq!(p.title, "설계서");
        assert_eq!(p.body_html, "<p>본문</p>");
        assert_eq!(p.webui, "/pages/viewpage.action?pageId=123");

        // Missing body/links are tolerated (empty), missing id is not.
        let bare = parse_page(r#"{ "id": "9" }"#).unwrap();
        assert_eq!(bare.body_html, "");
        assert!(parse_page(r#"{ "title": "no id" }"#).is_err());
        assert!(parse_page("not json").is_err());
    }

    #[test]
    fn parses_child_list_and_pagination() {
        let page1 = r#"{
            "results": [ { "id": "1", "title": "a" }, { "id": "2", "title": "b" } ],
            "limit": 2, "size": 2
        }"#;
        let (stubs, more) = parse_child_list(page1).unwrap();
        assert_eq!(stubs.len(), 2);
        assert_eq!(stubs[0], PageStub { id: "1".into(), title: "a".into() });
        assert!(more, "a full page implies another fetch");

        let last = r#"{ "results": [ { "id": "3", "title": "c" } ], "limit": 2 }"#;
        let (stubs, more) = parse_child_list(last).unwrap();
        assert_eq!(stubs.len(), 1);
        assert!(!more);

        assert!(parse_child_list(r#"{ "nope": true }"#).is_err());
    }

    /// In-memory fake: a page tree + optional per-page ingest failures.
    struct FakeApi {
        children: HashMap<&'static str, Vec<&'static str>>,
        broken_pages: HashSet<&'static str>,
        fetch_log: RefCell<Vec<String>>,
    }

    impl FakeApi {
        fn tree(children: &[(&'static str, &[&'static str])]) -> Self {
            let mut map = HashMap::new();
            for (parent, kids) in children {
                map.insert(*parent, kids.to_vec());
            }
            Self { children: map, broken_pages: HashSet::new(), fetch_log: RefCell::new(vec![]) }
        }
    }

    impl ConfluenceApi for FakeApi {
        fn fetch_page(&self, id: &str) -> Result<ConfluencePage, String> {
            self.fetch_log.borrow_mut().push(id.to_string());
            if self.broken_pages.contains(id) {
                return Err(format!("HTTP 500 — {id}"));
            }
            Ok(ConfluencePage {
                id: id.to_string(),
                title: format!("page {id}"),
                body_html: format!("<p>{id}</p>"),
                webui: format!("/pages/{id}"),
            })
        }

        fn fetch_children(&self, id: &str, _start: usize) -> Result<(Vec<PageStub>, bool), String> {
            let kids = self
                .children
                .get(id)
                .map(|v| {
                    v.iter()
                        .map(|k| PageStub { id: (*k).into(), title: format!("page {k}") })
                        .collect()
                })
                .unwrap_or_default();
            Ok((kids, false))
        }

        fn fetch_space_pages(
            &self,
            _space: &str,
            _start: usize,
        ) -> Result<(Vec<PageStub>, bool), String> {
            Ok((
                vec![
                    PageStub { id: "s1".into(), title: "space 1".into() },
                    PageStub { id: "s2".into(), title: "space 2".into() },
                ],
                false,
            ))
        }
    }

    fn run_crawl(
        api: &dyn ConfluenceApi,
        root: Option<&str>,
        space: Option<&str>,
        canceled: &AtomicBool,
        fail_ingest_for: &[&str],
    ) -> (IngestOutcome, Vec<IngestEvent>) {
        let mut events = Vec::new();
        let outcome = crawl(
            api,
            root,
            space,
            canceled,
            |p| {
                if fail_ingest_for.contains(&p.id.as_str()) {
                    Err("RAG ingest 미구현".into())
                } else {
                    Ok(())
                }
            },
            |ev| events.push(ev),
        );
        (outcome, events)
    }

    #[test]
    fn bfs_order_and_dedupe() {
        // root → a, b; a → c, a(cycle); b → c(duplicate).
        let api = FakeApi::tree(&[("root", &["a", "b"]), ("a", &["c", "a"]), ("b", &["c"])]);
        let canceled = AtomicBool::new(false);
        let (outcome, _) = run_crawl(&api, Some("root"), None, &canceled, &[]);
        assert_eq!(outcome.status, "succeeded");
        assert_eq!(outcome.ingested, 4);
        assert_eq!(outcome.failed, 0);
        // BFS order, each page fetched exactly once despite the cycle/duplicate.
        assert_eq!(*api.fetch_log.borrow(), vec!["root", "a", "b", "c"]);
    }

    #[test]
    fn per_page_ingest_failure_continues() {
        let api = FakeApi::tree(&[("root", &["a", "b"])]);
        let canceled = AtomicBool::new(false);
        let (outcome, events) = run_crawl(&api, Some("root"), None, &canceled, &["a"]);
        assert_eq!(outcome.status, "succeeded");
        assert_eq!(outcome.ingested, 2); // root + b
        assert_eq!(outcome.failed, 1); // a
        assert!(events.iter().any(
            |e| matches!(e, IngestEvent::PageFailed { page_id, .. } if page_id == "a")
        ));
    }

    #[test]
    fn broken_child_page_is_skipped_but_broken_root_is_fatal() {
        let mut api = FakeApi::tree(&[("root", &["a"])]);
        api.broken_pages.insert("a");
        let canceled = AtomicBool::new(false);
        let (outcome, _) = run_crawl(&api, Some("root"), None, &canceled, &[]);
        assert_eq!(outcome.status, "succeeded");
        assert_eq!((outcome.ingested, outcome.failed), (1, 1));

        let mut api = FakeApi::tree(&[]);
        api.broken_pages.insert("root");
        let (outcome, events) = run_crawl(&api, Some("root"), None, &canceled, &[]);
        assert_eq!(outcome.status, "failed");
        assert!(events.iter().any(|e| matches!(e, IngestEvent::Error { .. })));
    }

    #[test]
    fn cancel_stops_the_crawl() {
        let api = FakeApi::tree(&[("root", &["a", "b"])]);
        let canceled = AtomicBool::new(true); // pre-canceled
        let (outcome, _) = run_crawl(&api, Some("root"), None, &canceled, &[]);
        assert_eq!(outcome.status, "canceled");
        assert_eq!(outcome.ingested, 0);
    }

    #[test]
    fn space_mode_is_flat() {
        let api = FakeApi::tree(&[("s1", &["never-descended"])]);
        let canceled = AtomicBool::new(false);
        let (outcome, _) = run_crawl(&api, None, Some("OPS"), &canceled, &[]);
        assert_eq!(outcome.status, "succeeded");
        assert_eq!(outcome.ingested, 2); // s1 + s2 only, no descent
        assert_eq!(*api.fetch_log.borrow(), vec!["s1", "s2"]);
    }

    #[test]
    fn no_start_point_is_fatal() {
        let api = FakeApi::tree(&[]);
        let canceled = AtomicBool::new(false);
        let (outcome, events) = run_crawl(&api, None, None, &canceled, &[]);
        assert_eq!(outcome.status, "failed");
        assert!(events.iter().any(|e| matches!(e, IngestEvent::Error { .. })));
    }

    #[test]
    fn ingest_event_serializes_camel_case() {
        let ev = IngestEvent::PageIngested {
            page_id: "1".into(),
            title: "t".into(),
            ingested: 3,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"type\":\"pageIngested\""));
        assert!(json.contains("\"pageId\":\"1\""));
        assert!(json.contains("\"ingested\":3"));
    }
}

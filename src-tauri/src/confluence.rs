//! Confluence collection via the official MCP server (D82).
//!
//! Recursively collects a page tree (or a flat search result) through the
//! in-house Confluence **MCP** server (`mcp.rs`, JSON-RPC over streamable HTTP)
//! and saves the pages into the **local knowledge base** as one artifact entry
//! (`knowledge.rs`) — which the workflow's `knowledge` foundation step then
//! injects. This replaces the old REST crawl + WebView spike, both of which the
//! corporate WAF kept 403-ing (D75/D77), and the dead `RagClient::ingest_page`
//! sink (rag-chat has no ingest endpoint — D65).
//!
//! Progress streams to the settings UI over a `tauri::ipc::Channel`
//! (`IngestEvent`), mirroring the run engine's transport; cancellation uses a
//! flag registry (`IngestRegistry`, a child-process-free `RunRegistry`). The
//! cancel flag is checked between MCP tool calls, so cancellation latency is at
//! most one tool call.
//!
//! Testability: the BFS `crawl` loop is generic over the `ConfluenceApi` trait +
//! an ingest sink closure (driven by an in-memory fake in tests), and the JSON
//! parsers are pure `fn(&str)`. Only the production `ConfluenceApi` impl
//! (`McpConfluence`) and the sink changed — the engine and its tests are intact.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::Manager;

use crate::knowledge::KnowledgeEntry;
use crate::mcp;

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
    /// One page collected into the buffer (`ingested` = running count).
    PageIngested { page_id: String, title: String, ingested: u64 },
    /// One page failed to fetch; the crawl continues.
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

// ── MCP transport (D82) ─────────────────────────────────────────────────────
// Confluence is reached through the official MCP server (`mcp.rs`) instead of
// the REST API (403'd by the WAF — D75/D77). The BFS `crawl` engine and its
// `ConfluenceApi` trait are unchanged; only the production impl + ingest sink
// changed. Exact MCP tool I/O shapes are not known ahead of time, so the parsers
// are tolerant (try several candidate keys; non-JSON text → the page body).

/// Collection target from the settings UI (replaces the removed rootPageId /
/// spaceKey config fields — D82). `root_page_id` drives a recursive getChild
/// crawl; `search_query` seeds a flat searchContent listing.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConfluenceTarget {
    #[serde(default)]
    pub root_page_id: Option<String>,
    #[serde(default)]
    pub search_query: Option<String>,
}

/// First non-empty string among `keys`.
fn pick_str(obj: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = obj.get(*k).and_then(|x| x.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Best-effort page body: Confluence storage/view HTML first, then simple
/// `body`/`content`/`value`/`text`/`html` string fields.
fn pick_body(obj: &Value) -> Option<String> {
    for path in [["body", "storage", "value"], ["body", "view", "value"]] {
        let mut cur = obj;
        let mut ok = true;
        for k in path {
            match cur.get(k) {
                Some(v) => cur = v,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            if let Some(s) = cur.as_str() {
                if !s.trim().is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    for k in ["body", "content", "value", "text", "html"] {
        if let Some(s) = obj.get(k).and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Unwrap a single-page envelope (`{page|content|result|data: {...}}`) or the
/// first array element — only when the inner value looks like a page (has an id
/// or title), so a page whose own `content` field holds the body is not
/// mistaken for an envelope.
fn mcp_unwrap(v: &Value) -> &Value {
    for k in ["page", "content", "result", "data"] {
        if let Some(inner) = v.get(k) {
            if inner.is_object() && (inner.get("id").is_some() || inner.get("title").is_some()) {
                return inner;
            }
        }
    }
    if let Some(first) = v.as_array().and_then(|a| a.first()) {
        return first;
    }
    v
}

/// Parse an MCP `getPageById`/`getPage` result text into a `ConfluencePage`.
/// Reuses the tested REST parser for Confluence-shaped objects; otherwise
/// tolerant; non-JSON text becomes the page body verbatim.
fn parse_mcp_page(text: &str, requested_id: &str) -> Result<ConfluencePage, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("MCP 페이지 응답이 비어 있습니다".to_string());
    }
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        let obj = mcp_unwrap(&v);
        let rest_shaped = obj.get("id").and_then(|x| x.as_str()).is_some()
            && obj
                .get("body")
                .and_then(|b| b.get("storage"))
                .and_then(|s| s.get("value"))
                .is_some();
        if rest_shaped {
            if let Ok(p) = parse_page(&obj.to_string()) {
                return Ok(p);
            }
        }
        let id = pick_str(obj, &["id", "pageId", "contentId", "content_id"])
            .unwrap_or_else(|| requested_id.to_string());
        let title = pick_str(obj, &["title", "name"]).unwrap_or_default();
        let body_html = pick_body(obj).unwrap_or_default();
        let webui = obj
            .get("_links")
            .and_then(|l| l.get("webui"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .or_else(|| pick_str(obj, &["webui", "url", "link"]))
            .unwrap_or_default();
        return Ok(ConfluencePage { id, title, body_html, webui });
    }
    Ok(ConfluencePage {
        id: requested_id.to_string(),
        title: String::new(),
        body_html: t.to_string(),
        webui: String::new(),
    })
}

/// Parse an MCP `getChild`/`searchContent` result text into page stubs. Tolerant
/// of several container shapes; unparseable/empty → no stubs (not fatal).
fn parse_mcp_stubs(text: &str) -> Vec<PageStub> {
    let t = text.trim();
    if t.is_empty() {
        return vec![];
    }
    let v: Value = match serde_json::from_str(t) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    if v.get("results").and_then(|x| x.as_array()).is_some() {
        if let Ok((stubs, _)) = parse_child_list(t) {
            if !stubs.is_empty() {
                return stubs;
            }
        }
    }
    let arr: Vec<Value> = if let Some(a) = v.as_array() {
        a.clone()
    } else {
        ["results", "children", "pages", "content", "items", "value", "data"]
            .iter()
            .find_map(|k| v.get(*k).and_then(|x| x.as_array()).cloned())
            .unwrap_or_default()
    };
    arr.iter()
        .filter_map(|item| {
            let id = pick_str(item, &["id", "pageId", "contentId", "content_id"])?;
            let title = pick_str(item, &["title", "name"]).unwrap_or_default();
            Some(PageStub { id, title })
        })
        .collect()
}

/// `ConfluenceApi` over the MCP session. Trait methods are `&self`, so the
/// session lives in a `RefCell` (single-threaded crawl → no contention). Tool
/// names + argument keys are resolved from the connected server's `tools/list`
/// (with fallbacks) so the wire contract isn't hard-coded.
struct McpConfluence {
    sess: RefCell<mcp::McpSession>,
    page_tool: String,
    child_tool: String,
    search_tool: String,
    page_arg: String,
    child_arg: String,
    search_arg: String,
}

impl McpConfluence {
    fn new(sess: mcp::McpSession) -> Self {
        let page = sess
            .tools
            .iter()
            .find(|t| t.name == "getPageById")
            .cloned()
            .or_else(|| sess.tools.iter().find(|t| t.name == "getPage").cloned());
        let child = sess.tools.iter().find(|t| t.name == "getChild").cloned();
        let search = sess.tools.iter().find(|t| t.name == "searchContent").cloned();
        let page_arg =
            page.as_ref().map(|t| mcp::arg_key_for(t, "id")).unwrap_or_else(|| "id".to_string());
        let child_arg =
            child.as_ref().map(|t| mcp::arg_key_for(t, "id")).unwrap_or_else(|| "id".to_string());
        let search_arg = search
            .as_ref()
            .map(|t| mcp::arg_key_for(t, "query"))
            .unwrap_or_else(|| "query".to_string());
        McpConfluence {
            page_tool: page.map(|t| t.name).unwrap_or_else(|| "getPageById".to_string()),
            child_tool: child.map(|t| t.name).unwrap_or_else(|| "getChild".to_string()),
            search_tool: search.map(|t| t.name).unwrap_or_else(|| "searchContent".to_string()),
            page_arg,
            child_arg,
            search_arg,
            sess: RefCell::new(sess),
        }
    }

    fn call(&self, tool: &str, arg_key: &str, arg_val: &str) -> Result<String, String> {
        let mut m = serde_json::Map::new();
        m.insert(arg_key.to_string(), Value::String(arg_val.to_string()));
        self.sess.borrow_mut().call_tool(tool, Value::Object(m))
    }
}

impl ConfluenceApi for McpConfluence {
    fn fetch_page(&self, id: &str) -> Result<ConfluencePage, String> {
        let text = self.call(&self.page_tool, &self.page_arg, id)?;
        parse_mcp_page(&text, id)
    }

    fn fetch_children(&self, id: &str, start: usize) -> Result<(Vec<PageStub>, bool), String> {
        // MCP getChild returns all children at once — no REST-style pagination.
        if start > 0 {
            return Ok((vec![], false));
        }
        let text = self.call(&self.child_tool, &self.child_arg, id)?;
        Ok((parse_mcp_stubs(&text), false))
    }

    fn fetch_space_pages(&self, query: &str, start: usize) -> Result<(Vec<PageStub>, bool), String> {
        if start > 0 {
            return Ok((vec![], false));
        }
        let text = self.call(&self.search_tool, &self.search_arg, query)?;
        Ok((parse_mcp_stubs(&text), false))
    }
}

/// Escape the few chars that break out of an HTML text context.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Safe, readable file-name stem from a page title (illegal chars → space,
/// collapsed, capped). The knowledge writer sanitizes again + dedupes.
fn sanitize_title(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { ' ' } else { c })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim_matches([' ', '.']).to_string();
    let base = if cleaned.is_empty() { "page" } else { &cleaned };
    base.chars().take(60).collect()
}

/// Wrap a page's storage body as a self-contained HTML document for the
/// knowledge artifact folder (read by agents via extraDirs; previewable).
fn page_doc_html(page: &ConfluencePage) -> String {
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{t}</title></head>\n<body>\n<h1>{t}</h1>\n<!-- Confluence page id={id} -->\n{body}\n</body></html>\n",
        t = esc(&page.title),
        id = page.id,
        body = page.body_html,
    )
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

/// Build the artifact knowledge entry that stores a crawl's pages. `body` is the
/// injected summary (a titles index); files are written by `save_knowledge_docs`.
fn build_entry(label: &str, pages: &[(String, String, String)]) -> KnowledgeEntry {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut body = format!(
        "출처: Confluence MCP\n대상: {label}\n수집 페이지: {}건\n\n포함 문서:\n",
        pages.len()
    );
    for (_, _, title) in pages.iter().take(80) {
        body.push_str("- ");
        body.push_str(if title.trim().is_empty() { "(제목 없음)" } else { title });
        body.push('\n');
    }
    if pages.len() > 80 {
        body.push_str(&format!("… 외 {}건\n", pages.len() - 80));
    }
    KnowledgeEntry {
        id: format!("confluence-{millis}"),
        title: format!("Confluence 수집 — {label} ({}건)", pages.len()),
        body,
        kind: String::new(), // save_knowledge_docs sets "artifact"
        files: Vec::new(),
        source_project_id: None,
        source_category: None,
        source_title: Some(format!("Confluence MCP — {label}")),
        created_at: 0,
        updated_at: 0,
    }
}

/// Start an MCP crawl on a worker thread; returns the ingest id immediately.
/// The collected pages become one knowledge-base artifact entry (D82). Progress
/// arrives on `on_event`; cancel with `cancel_ingest`.
#[tauri::command]
pub fn start_confluence_ingest(
    app: tauri::AppHandle,
    target: ConfluenceTarget,
    on_event: Channel<IngestEvent>,
) -> Result<String, String> {
    let config_dir = crate::ow_home()?;
    let conf = crate::settings::load(&config_dir)
        .confluence
        .filter(|c| !c.url.trim().is_empty())
        .ok_or_else(|| "Confluence MCP가 설정되지 않았습니다 — 지식 화면에서 등록해 주세요".to_string())?;

    let root_id = target.root_page_id.unwrap_or_default().trim().to_string();
    let search = target.search_query.unwrap_or_default().trim().to_string();
    if root_id.is_empty() && search.is_empty() {
        return Err("수집 대상을 입력해 주세요 — 루트 페이지 ID 또는 검색어".to_string());
    }

    let registry = app.state::<IngestRegistry>();
    let n = registry.counter.fetch_add(1, Ordering::Relaxed) + 1;
    let ingest_id = format!("ingest-{n}");
    let canceled = Arc::new(AtomicBool::new(false));
    registry
        .flags
        .lock()
        .map_err(|_| "ingest registry poisoned".to_string())?
        .insert(ingest_id.clone(), canceled.clone());

    let url = conf.url.clone();
    let auth = conf.auth_key.clone();
    let app_handle = app.clone();
    let id_for_thread = ingest_id.clone();

    std::thread::spawn(move || {
        let emit = |ev: IngestEvent| {
            let _ = on_event.send(ev);
        };
        let unregister = || {
            if let Ok(mut flags) = app_handle.state::<IngestRegistry>().flags.lock() {
                flags.remove(&id_for_thread);
            }
        };
        let label = if !root_id.is_empty() {
            format!("루트 {root_id}")
        } else {
            format!("검색 \"{search}\"")
        };
        emit(IngestEvent::Started { root_id: label.clone() });

        // Handshake with the MCP server (network) on this worker thread.
        let session = match mcp::McpSession::connect(&url, auth.as_deref(), false) {
            Ok(s) => s,
            Err(e) => {
                emit(IngestEvent::Error { message: format!("Confluence MCP 연결 실패: {e}") });
                emit(IngestEvent::End { status: "failed".to_string(), ingested: 0, failed: 0 });
                unregister();
                return;
            }
        };
        let api = McpConfluence::new(session);

        // Buffer each fetched page (file name, html, title). The sink never
        // fails (buffering), so `ingested` == `fetched`; `failed` counts fetch
        // failures inside `crawl`.
        let buf: RefCell<Vec<(String, String, String)>> = RefCell::new(Vec::new());
        let outcome = crawl(
            &api,
            Some(root_id.as_str()).filter(|s| !s.is_empty()),
            Some(search.as_str()).filter(|s| !s.is_empty()),
            &canceled,
            |page| {
                let name = format!("{}-{}.html", sanitize_title(&page.title), page.id);
                buf.borrow_mut().push((name, page_doc_html(page), page.title.clone()));
                Ok(())
            },
            emit,
        );

        // Save whatever was collected (even a partial/canceled crawl) as one
        // artifact knowledge entry — the workflow's knowledge step injects it.
        let pages = buf.into_inner();
        let mut status = outcome.status.clone();
        if !pages.is_empty() {
            let docs: Vec<(String, String)> =
                pages.iter().map(|(n, h, _)| (n.clone(), h.clone())).collect();
            let entry = build_entry(&label, &pages);
            if let Err(e) = crate::knowledge::save_knowledge_docs(entry, docs) {
                let _ = on_event
                    .send(IngestEvent::Error { message: format!("지식 베이스 저장 실패: {e}") });
                status = "failed".to_string();
            }
        }
        let _ = on_event.send(IngestEvent::End {
            status,
            ingested: outcome.ingested,
            failed: outcome.failed,
        });
        unregister();
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

/// Settings-screen connection test (D82): handshake with the MCP server and
/// report the tool count/names. Verifies the URL + `x-auth` key without needing
/// a target page. `async` + `spawn_blocking` (same pattern as `probe_rag`).
#[tauri::command]
pub async fn probe_confluence() -> Result<String, String> {
    let config_dir = crate::ow_home()?;
    let conf = crate::settings::load(&config_dir)
        .confluence
        .filter(|c| !c.url.trim().is_empty())
        .ok_or_else(|| "Confluence MCP가 설정되지 않았습니다 — 지식 화면에서 등록해 주세요".to_string())?;

    tauri::async_runtime::spawn_blocking(move || {
        let session = mcp::McpSession::connect(&conf.url, conf.auth_key.as_deref(), false)?;
        if session.tools.is_empty() {
            return Ok::<String, String>("연결됨 — 사용 가능한 도구가 없습니다".to_string());
        }
        let names: Vec<&str> = session.tools.iter().map(|t| t.name.as_str()).take(10).collect();
        Ok(format!("연결됨 — {}개 도구 ({})", session.tools.len(), names.join(", ")))
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
    fn parses_mcp_page_shapes() {
        // REST-shaped Confluence content object → reuses the tested parser.
        let rest = r#"{"id":"123","title":"설계","body":{"storage":{"value":"<p>본문</p>"}}}"#;
        let p = parse_mcp_page(rest, "123").unwrap();
        assert_eq!(p.id, "123");
        assert_eq!(p.body_html, "<p>본문</p>");

        // Simplified shape: body under `content`, id under `pageId`, title `name`.
        let simple = r#"{"pageId":"9","name":"제목","content":"<p>x</p>"}"#;
        let p = parse_mcp_page(simple, "req").unwrap();
        assert_eq!(p.id, "9");
        assert_eq!(p.title, "제목");
        assert_eq!(p.body_html, "<p>x</p>");

        // Envelope ({page:{...}}) + view body; then non-JSON text fallback.
        let env = r#"{"page":{"id":"5","body":{"view":{"value":"<b>v</b>"}}}}"#;
        assert_eq!(parse_mcp_page(env, "5").unwrap().body_html, "<b>v</b>");
        let plain = parse_mcp_page("just some text", "42").unwrap();
        assert_eq!(plain.id, "42");
        assert_eq!(plain.body_html, "just some text");
    }

    #[test]
    fn parses_mcp_stub_shapes() {
        let rest = r#"{"results":[{"id":"1","title":"a"},{"id":"2","title":"b"}]}"#;
        assert_eq!(parse_mcp_stubs(rest).len(), 2);

        let children = r#"{"children":[{"pageId":"3","name":"c"}]}"#;
        assert_eq!(parse_mcp_stubs(children), vec![PageStub { id: "3".into(), title: "c".into() }]);

        // Bare array parsed; junk → empty (not fatal).
        assert_eq!(parse_mcp_stubs(r#"[{"id":"7"}]"#).len(), 1);
        assert!(parse_mcp_stubs("not json").is_empty());
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

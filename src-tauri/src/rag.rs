//! RAG API adapter (D48/D50). The rag workflow step queries the user's RAG
//! service; this app also hands over raw crawled Confluence pages for ingestion.
//!
//! `search` is implemented against the Samsung SDS Fabrix `rag-chat` API
//! (`/openapi/rag-chat/v1/{models,messages}`); the two auth header values
//! (`x-fabrix-client` / `x-openapi-token`) come from `RagConfig.secret_key` /
//! `pass_key`, mirroring `fabrix.rs` (D64). `ingest_page` remains a `TODO(user)`
//! stub — the rag-chat API has no ingestion endpoint (knowledge assets are
//! managed on the Fabrix platform), so Confluence ingestion reports per-page
//! failures (never a crash) until a real ingestion backend is wired.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::detect::ModelOption;
use crate::settings::RagConfig;

/// RAG chat model — GLM 5.2. The app always uses this model for the rag step;
/// the `/models` list is fetched only for the connection test. Model ids come
/// from the Fabrix rag-chat `/models` endpoint.
const GLM_5_2_MODEL_ID: &str = "019f23a1-46aa-7fa5-a6ab-391127fea7e6";

/// Knowledge asset used when `RagConfig.knowledge_asset_id` is unset — lets the
/// rag step work out of the box (the user can override it in the 지식 screen).
const SAMPLE_KNOWLEDGE_ASSET_ID: &str = "019f5a11-e701-7315-b5b5-91912381b4f7";

/// One page handed to the RAG API verbatim (this app does NOT summarize — the
/// RAG service would own summarization + embedding). Retained as the RAG-ingest
/// extension point; it has no caller since Confluence moved to MCP → local
/// knowledge base (D82), and rag-chat has no ingest endpoint (D65).
#[allow(dead_code)]
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IngestPage {
    pub id: String,
    pub title: String,
    /// Absolute webui link (source attribution for later search hits).
    pub url: String,
    /// Raw `body.storage` HTML.
    pub content_html: String,
}

/// One search hit: rendered in the canvas "검색 결과" HTML tab and injected
/// into the agent prompt by the rag workflow step.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RagHit {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    pub snippet: String,
    #[serde(default)]
    pub score: Option<f64>,
}

/// Per-request timeout. Generous (D53): the search runs mid-turn as a workflow
/// preflight and a slow corporate RAG backend timing out reads as a silently
/// skipped step.
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Blocking HTTP client over the user's RAG service. Construct on a worker
/// thread only — `reqwest::blocking` must never run on the IPC thread.
pub struct RagClient {
    cfg: RagConfig,
    http: reqwest::blocking::Client,
}

impl RagClient {
    /// `allow_invalid_certs` is currently always false (callers pass `false`,
    /// TLS on — D75); prefer installing the corporate proxy CA into the Windows
    /// store. Kept as a parameter for symmetry with the other HTTP clients.
    pub fn new(cfg: &RagConfig, allow_invalid_certs: bool) -> Result<Self, String> {
        if cfg.endpoint.trim().is_empty() {
            return Err("RAG endpoint가 설정되지 않았습니다 — 지식 화면에서 등록해 주세요".into());
        }
        let http = reqwest::blocking::Client::builder()
            // The RAG service is a directly-reachable corporate endpoint —
            // connect straight to it, ignoring any HTTP(S)_PROXY/ALL_PROXY (D66).
            .no_proxy()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(HTTP_TIMEOUT)
            .danger_accept_invalid_certs(allow_invalid_certs)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { cfg: cfg.clone(), http })
    }

    /// TODO(user): the Fabrix rag-chat API has no ingestion endpoint — knowledge
    /// assets are indexed on the Fabrix platform — so Confluence ingestion is a
    /// separate concern. If a real ingestion backend is added, POST `page` here
    /// (attach the `x-fabrix-client` / `x-openapi-token` headers via
    /// [`attach_headers`]). Until then this returns an actionable Korean `Err`,
    /// surfaced by the frontend as a per-page failure (never a crash).
    #[allow(dead_code)] // retained extension point; no caller since D82 (see IngestPage)
    pub fn ingest_page(&self, page: &IngestPage) -> Result<(), String> {
        let _ = page;
        Err("RAG ingest 미구현 — src-tauri/src/rag.rs의 RagClient::ingest_page를 채워 주세요".into())
    }

    /// Query the Fabrix rag-chat `/messages` endpoint (non-stream) and map the
    /// response into `RagHit`s (synthesized answer + source chunks). GLM 5.2 is
    /// the fixed model; the knowledge asset comes from settings (or the sample).
    pub fn search(&self, query: &str, top_k: u32) -> Result<Vec<RagHit>, String> {
        let asset = self
            .cfg
            .knowledge_asset_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(SAMPLE_KNOWLEDGE_ASSET_ID);
        let url = format!("{}/openapi/rag-chat/v1/messages", self.cfg.endpoint);
        let body = serde_json::json!({
            "modelIds": [GLM_5_2_MODEL_ID],
            "contents": [query],
            "isStream": false,
            "llmConfig": {},
            "systemPrompt": "",
            "knowledgeAssetId": asset,
        });
        let req = attach_headers(self.http.post(&url).json(&body), &self.cfg);
        let resp = req.send().map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("RAG 검색 실패: HTTP {status} — {}", text.trim()));
        }
        parse_rag_response(&text, top_k as usize)
    }

    /// Connection-test helper: GET the rag-chat model list. Verifies credentials
    /// + reachability without needing a knowledge asset. The list is cached in
    /// settings and shown in the 지식 screen (cache-first, D66); the RAG search
    /// itself always uses the fixed GLM 5.2 model.
    pub fn fetch_models(&self) -> Result<Vec<ModelOption>, String> {
        let url = format!("{}/openapi/rag-chat/v1/models", self.cfg.endpoint);
        let req = attach_headers(self.http.get(&url), &self.cfg);
        let resp = req.send().map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("모델 목록 조회 실패: HTTP {status} — {}", text.trim()));
        }
        // Response shape matches Fabrix's `/all-models` — reuse its parser.
        crate::fabrix::parse_models_json(&text)
    }
}

/// Attach the Fabrix auth headers (values from `RagConfig`; sent only when set).
/// Same header names as the Fabrix agent (D64) — the rag-chat API shares them.
fn attach_headers(
    mut req: reqwest::blocking::RequestBuilder,
    cfg: &RagConfig,
) -> reqwest::blocking::RequestBuilder {
    if let Some(k) = cfg.secret_key.as_deref() {
        req = req.header("x-fabrix-client", k);
    }
    if let Some(k) = cfg.pass_key.as_deref() {
        req = req.header("x-openapi-token", k);
    }
    req
}

/// Map a (non-stream) rag-chat `/messages` response into `RagHit`s: the
/// synthesized answer (`content`) as the leading hit, then the source chunks
/// (`contentReferences[].references[]`, falling back to top-level `references`).
/// Pure (no network) → unit-tested against captured sample JSON.
fn parse_rag_response(body: &str, top_k: usize) -> Result<Vec<RagHit>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("RAG 응답 JSON 파싱 실패: {e}"))?;

    // A service-side failure surfaces as an error (frontend → "step skipped").
    let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("");
    let up = status.to_ascii_uppercase();
    if up.contains("FAIL") || up.contains("ERROR") {
        let msg = v
            .get("message")
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| v.get("content").and_then(|x| x.as_str()).filter(|s| !s.trim().is_empty()))
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("RAG 오류 (status={status})"));
        return Err(format!("RAG 검색 실패: {msg}"));
    }

    let mut hits = Vec::new();

    // Leading hit: the RAG-synthesized answer.
    if let Some(ans) = v
        .get("content")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        hits.push(RagHit {
            title: Some("RAG 요약 답변".into()),
            url: None,
            snippet: ans.to_string(),
            score: None,
        });
    }

    // Source chunks: contentReferences[].references[] carries the source link.
    let mut sources: Vec<RagHit> = Vec::new();
    if let Some(crefs) = v.get("contentReferences").and_then(|x| x.as_array()) {
        for cr in crefs {
            if let Some(refs) = cr.get("references").and_then(|x| x.as_array()) {
                for r in refs {
                    sources.push(ref_to_hit(r, "link"));
                }
            }
        }
    }
    // Fallback: top-level references[] (url key is "url", often null).
    if sources.is_empty() {
        if let Some(refs) = v.get("references").and_then(|x| x.as_array()) {
            for r in refs {
                sources.push(ref_to_hit(r, "url"));
            }
        }
    }

    sources.truncate(top_k);
    hits.extend(sources);
    Ok(hits)
}

/// One reference object → a `RagHit`. `url_key` differs by array
/// (`contentReferences[].references[]` uses "link"; top-level uses "url").
fn ref_to_hit(r: &Value, url_key: &str) -> RagHit {
    let field = |k: &str| {
        r.get(k)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let title = field("title").or_else(|| field("filename")).map(|s| s.to_string());
    let url = field(url_key).map(|s| s.to_string());
    let snippet = field("content").unwrap_or("").to_string();
    RagHit { title, url, snippet, score: None }
}

/// Runtime search for the rag workflow step. Blocking HTTP → `spawn_blocking`
/// (same pattern as `detect_agent`).
#[tauri::command]
pub async fn rag_search(query: String, top_k: Option<u32>) -> Result<Vec<RagHit>, String> {
    let config_dir = crate::ow_home()?;
    let settings = crate::settings::load(&config_dir);
    let cfg = settings
        .rag
        .clone()
        .ok_or_else(|| "RAG endpoint가 설정되지 않았습니다 — 지식 화면에서 등록해 주세요".to_string())?;
    // TLS verification is always on (D75); no per-config opt-out.
    let allow_invalid = false;
    let k = top_k.or(cfg.top_k).unwrap_or(5);

    tauri::async_runtime::spawn_blocking(move || {
        RagClient::new(&cfg, allow_invalid)?.search(&query, k)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Connection test for the RAG service (지식 화면): fetch the model list to
/// verify credentials + reachability without needing a knowledge asset. Mirrors
/// `probe_fabrix` (D64).
#[tauri::command]
pub async fn probe_rag() -> Result<String, String> {
    let config_dir = crate::ow_home()?;
    let settings = crate::settings::load(&config_dir);
    let cfg = settings
        .rag
        .clone()
        .filter(|c| !c.endpoint.trim().is_empty())
        .ok_or_else(|| "RAG endpoint가 설정되지 않았습니다 — 지식 화면에서 등록해 주세요".to_string())?;
    // TLS verification is always on (D75); no per-config opt-out.
    let allow_invalid = false;

    tauri::async_runtime::spawn_blocking(move || {
        let models = RagClient::new(&cfg, allow_invalid)?.fetch_models()?;
        // Cache the fresh list so the 지식 screen shows it without a network
        // call on the next load (cache-first, D66).
        let mut s = crate::settings::load(&config_dir);
        if let Some(r) = s.rag.as_mut() {
            r.models = models.clone();
            let _ = crate::settings::save(&config_dir, &s);
        }
        Ok::<String, String>(format!("연결됨 ({}개 모델)", models.len()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RagConfig {
        RagConfig {
            endpoint: "https://rag.example.com".into(),
            secret_key: None,
            pass_key: None,
            top_k: None,
            knowledge_asset_id: None,
            models: vec![],
        }
    }

    // The ingest stub's Korean "미구현" message is load-bearing: the Confluence
    // ingest path surfaces it as a per-page failure.
    #[test]
    fn ingest_stub_returns_actionable_unimplemented_error() {
        let client = RagClient::new(&cfg(), false).unwrap();
        let page = IngestPage {
            id: "1".into(),
            title: "t".into(),
            url: "https://w/1".into(),
            content_html: "<p>x</p>".into(),
        };
        assert!(client.ingest_page(&page).unwrap_err().contains("미구현"));
    }

    #[test]
    fn empty_endpoint_is_rejected() {
        let empty = RagConfig::default();
        assert!(RagClient::new(&empty, false).is_err());
    }

    #[test]
    fn rag_hit_serde_camel_case() {
        let hit = RagHit {
            title: Some("설계서".into()),
            url: Some("https://w/2".into()),
            snippet: "발췌".into(),
            score: Some(0.87),
        };
        let json = serde_json::to_string(&hit).unwrap();
        assert!(json.contains("\"snippet\":\"발췌\""));
        let back: RagHit = serde_json::from_str(&json).unwrap();
        assert_eq!(back, hit);
        // Minimal hit (snippet only) still parses.
        let min: RagHit = serde_json::from_str(r#"{"snippet":"s"}"#).unwrap();
        assert_eq!(min.snippet, "s");
        assert!(min.title.is_none() && min.url.is_none() && min.score.is_none());
    }

    #[test]
    fn parses_answer_and_source_chunks() {
        // Shaped like the rag-chat `/messages` sample: content + both arrays.
        let body = r#"{
            "content": "요약 답변입니다.",
            "status": "SUCCESS",
            "responseCode": "R20000",
            "contentReferences": [
                { "references": [
                    { "title": "chunk1", "content": "청크 본문", "link": "https://wiki/1", "filename": "chunk1.txt", "rankScore": 0.0001 }
                ]}
            ],
            "references": [
                { "type": "search", "title": "chunk1", "url": null, "content": "청크 본문" }
            ]
        }"#;
        let hits = parse_rag_response(body, 5).unwrap();
        assert_eq!(hits.len(), 2);
        // Answer leads.
        assert_eq!(hits[0].title.as_deref(), Some("RAG 요약 답변"));
        assert_eq!(hits[0].snippet, "요약 답변입니다.");
        assert!(hits[0].url.is_none());
        // Source chunk comes from contentReferences (has the link).
        assert_eq!(hits[1].title.as_deref(), Some("chunk1"));
        assert_eq!(hits[1].url.as_deref(), Some("https://wiki/1"));
        assert_eq!(hits[1].snippet, "청크 본문");
    }

    #[test]
    fn falls_back_to_top_level_references() {
        let body = r#"{
            "content": "",
            "references": [
                { "title": "doc-a", "url": "https://wiki/a", "content": "본문 A" }
            ]
        }"#;
        let hits = parse_rag_response(body, 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title.as_deref(), Some("doc-a"));
        assert_eq!(hits[0].url.as_deref(), Some("https://wiki/a"));
        assert_eq!(hits[0].snippet, "본문 A");
    }

    #[test]
    fn title_falls_back_to_filename() {
        let body = r#"{ "contentReferences": [ { "references": [
            { "content": "c", "link": "https://w/1", "filename": "readme.txt" }
        ]}]}"#;
        let hits = parse_rag_response(body, 5).unwrap();
        assert_eq!(hits[0].title.as_deref(), Some("readme.txt"));
    }

    #[test]
    fn failure_status_is_error() {
        let body = r#"{ "status": "FAIL", "message": "권한 없음" }"#;
        let err = parse_rag_response(body, 5).unwrap_err();
        assert!(err.contains("권한 없음"));
    }

    #[test]
    fn top_k_caps_sources_but_keeps_answer() {
        let body = r#"{
            "content": "답변",
            "contentReferences": [ { "references": [
                { "title": "c1", "content": "1", "link": "https://w/1" },
                { "title": "c2", "content": "2", "link": "https://w/2" },
                { "title": "c3", "content": "3", "link": "https://w/3" }
            ]}]
        }"#;
        let hits = parse_rag_response(body, 2).unwrap();
        // answer + 2 sources (answer is never truncated away).
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].title.as_deref(), Some("RAG 요약 답변"));
        assert_eq!(hits[2].title.as_deref(), Some("c2"));
    }

    #[test]
    fn non_json_body_is_error() {
        assert!(parse_rag_response("not json", 5).is_err());
    }
}

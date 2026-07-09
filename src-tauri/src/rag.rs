//! RAG API adapter — the user's own extension point (D48).
//!
//! The user's RAG service does **summarization + embedding itself**; this app
//! only hands over raw crawled content (ingest, from `confluence.rs`) and
//! queries (search, from the rag workflow step). The two `TODO(user)` methods
//! below are the intended fill-in points: everything around them (config load
//! from settings, HTTP client construction, worker threading, IPC command) is
//! already wired, so implementing the integration means replacing two function
//! bodies.
//!
//! Until they are implemented, both return an actionable Korean `Err` — the
//! frontend surfaces it as a "step skipped" note (search) or per-page failure
//! (ingest), never a crash.

use serde::{Deserialize, Serialize};

use crate::settings::RagConfig;

/// One crawled Confluence page handed to the RAG API verbatim (this app does
/// NOT summarize — the RAG service owns summarization + embedding).
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

/// Blocking HTTP client over the user's RAG service. Construct on a worker
/// thread only — `reqwest::blocking` must never run on the IPC thread.
pub struct RagClient {
    #[allow(dead_code)] // consumed by the TODO(user) implementations below
    cfg: RagConfig,
    #[allow(dead_code)]
    http: reqwest::blocking::Client,
}

impl RagClient {
    /// `allow_invalid_certs` comes from `ConfluenceConfig` (one TLS knob for
    /// both backends); default false — prefer installing the corporate proxy CA
    /// into the Windows store.
    pub fn new(cfg: &RagConfig, allow_invalid_certs: bool) -> Result<Self, String> {
        if cfg.endpoint.trim().is_empty() {
            return Err("RAG endpoint가 설정되지 않았습니다 — 지식 화면에서 등록해 주세요".into());
        }
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .danger_accept_invalid_certs(allow_invalid_certs)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { cfg: cfg.clone(), http })
    }

    /// TODO(user): POST one crawled page to your RAG ingestion endpoint.
    /// `self.cfg.endpoint` / `self.cfg.api_key` are already loaded from
    /// settings. Example skeleton:
    ///
    /// ```ignore
    /// let mut req = self.http.post(format!("{}/ingest", self.cfg.endpoint)).json(page);
    /// if let Some(key) = self.cfg.api_key.as_deref() {
    ///     req = req.bearer_auth(key);
    /// }
    /// let res = req.send().map_err(|e| e.to_string())?;
    /// if !res.status().is_success() {
    ///     return Err(format!("RAG ingest 실패: HTTP {}", res.status()));
    /// }
    /// Ok(())
    /// ```
    pub fn ingest_page(&self, page: &IngestPage) -> Result<(), String> {
        let _ = page;
        Err("RAG ingest 미구현 — src-tauri/src/rag.rs의 RagClient::ingest_page를 채워 주세요".into())
    }

    /// TODO(user): query your RAG search endpoint and map the response into
    /// `RagHit`s. Example skeleton:
    ///
    /// ```ignore
    /// #[derive(serde::Deserialize)]
    /// struct SearchResponse { hits: Vec<RagHit> }
    /// let mut req = self
    ///     .http
    ///     .post(format!("{}/search", self.cfg.endpoint))
    ///     .json(&serde_json::json!({ "query": query, "topK": top_k }));
    /// if let Some(key) = self.cfg.api_key.as_deref() {
    ///     req = req.bearer_auth(key);
    /// }
    /// let res: SearchResponse = req.send().map_err(|e| e.to_string())?
    ///     .error_for_status().map_err(|e| e.to_string())?
    ///     .json().map_err(|e| e.to_string())?;
    /// Ok(res.hits)
    /// ```
    pub fn search(&self, query: &str, top_k: u32) -> Result<Vec<RagHit>, String> {
        let _ = (query, top_k);
        Err("RAG search 미구현 — src-tauri/src/rag.rs의 RagClient::search를 채워 주세요".into())
    }
}

/// Runtime search for the rag workflow step. Blocking HTTP → `spawn_blocking`
/// (same pattern as `detect_agent`).
#[tauri::command]
pub async fn rag_search(
    app: tauri::AppHandle,
    query: String,
    top_k: Option<u32>,
) -> Result<Vec<RagHit>, String> {
    use tauri::Manager;

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings = crate::settings::load(&config_dir);
    let cfg = settings
        .rag
        .clone()
        .ok_or_else(|| "RAG endpoint가 설정되지 않았습니다 — 지식 화면에서 등록해 주세요".to_string())?;
    let allow_invalid = settings.confluence.as_ref().map(|c| c.allow_invalid_certs).unwrap_or(false);
    let k = top_k.or(cfg.top_k).unwrap_or(5);

    tauri::async_runtime::spawn_blocking(move || {
        RagClient::new(&cfg, allow_invalid)?.search(&query, k)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RagConfig {
        RagConfig { endpoint: "https://rag.example.com".into(), api_key: None, top_k: None }
    }

    // Locks the actionable-error contract until the user fills in the stubs:
    // the frontend keys "step skipped" guidance off these messages.
    #[test]
    fn stubs_return_actionable_unimplemented_errors() {
        let client = RagClient::new(&cfg(), false).unwrap();
        let page = IngestPage {
            id: "1".into(),
            title: "t".into(),
            url: "https://w/1".into(),
            content_html: "<p>x</p>".into(),
        };
        assert!(client.ingest_page(&page).unwrap_err().contains("미구현"));
        assert!(client.search("q", 5).unwrap_err().contains("미구현"));
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
}

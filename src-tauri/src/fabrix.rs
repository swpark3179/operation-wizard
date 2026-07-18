//! Fabrix — the remote HTTP API agent (D64).
//!
//! Unlike the local CLI agents (resolve a binary → spawn → parse stdout),
//! Fabrix is an HTTP service:
//!   - model list: `GET  {endpoint}/openapi/chat/v1/all-models`
//!   - chat:       `POST {endpoint}/openapi/chat/v1/messages`  (SSE stream)
//!
//! Detection and run bypass `resolve.rs`/`exec.rs`/the `run.rs` process pipeline
//! and go through this module. Auth travels as the `x-fabrix-client` and
//! `x-openapi-token` request headers, sourced from `settings.fabrix`.
//!
//! The HTTP client reuses the same `reqwest` blocking + native-tls (schannel)
//! recipe as `rag.rs`/`confluence.rs` — no new crate, and the OS cert store is
//! trusted (corporate proxy CA works without `allowInvalidCerts`).

use std::io::{BufRead, BufReader};
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::Value;
use tauri::ipc::Channel;
use tauri::Manager;

use crate::detect::{DetectedAgent, ModelOption};
use crate::run::{RunArgs, RunEvent, RunRegistry};
use crate::settings::{self, FabrixConfig};

/// Connect timeout for every Fabrix request. The chat stream sets no *total*
/// timeout (a chat may legitimately run for minutes — the run engine is also
/// untimed, D53); the model-list GET adds a short total timeout on top.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Total timeout for the (short) model-list GET.
const MODELS_TIMEOUT: Duration = Duration::from_secs(30);

/// Build a blocking HTTP client. `total` is the whole-request timeout; pass
/// `None` for the streaming chat so long responses are not cut off.
fn build_client(allow_invalid_certs: bool, total: Option<Duration>) -> Result<reqwest::blocking::Client, String> {
    let mut b = reqwest::blocking::Client::builder()
        // Fabrix is a directly-reachable corporate endpoint — connect straight
        // to it and ignore any HTTP(S)_PROXY/ALL_PROXY env var (D66).
        .no_proxy()
        .connect_timeout(CONNECT_TIMEOUT)
        .danger_accept_invalid_certs(allow_invalid_certs);
    if let Some(t) = total {
        b = b.timeout(t);
    }
    b.build().map_err(|e| e.to_string())
}

fn base(cfg: &FabrixConfig) -> String {
    cfg.endpoint_url.trim().trim_end_matches('/').to_string()
}

// ---------------------------------------------------------------------------
// Model list
// ---------------------------------------------------------------------------

/// Find the Korean (`ko`) `content` in a `[{languageCode, content}]` array, and
/// fall back to the first non-empty content. `None` if the field is missing or
/// has no usable content (the caller then uses the modelId as the label).
fn pick_ko_name(name: Option<&Value>) -> Option<String> {
    let arr = name?.as_array()?;
    // Prefer the Korean entry.
    for entry in arr {
        if entry.get("languageCode").and_then(|x| x.as_str()) == Some("ko") {
            if let Some(c) = entry.get("content").and_then(|x| x.as_str()) {
                if !c.trim().is_empty() {
                    return Some(c.to_string());
                }
            }
        }
    }
    // Fallback: the first non-empty content in any language.
    for entry in arr {
        if let Some(c) = entry.get("content").and_then(|x| x.as_str()) {
            if !c.trim().is_empty() {
                return Some(c.to_string());
            }
        }
    }
    None
}

/// Parse the `all-models` response: a top-level JSON array whose items carry a
/// `modelId` (unique key → `ModelOption.id`) and a `name` array from which the
/// Korean display name is taken (→ `ModelOption.label`). No synthetic `default`
/// option is prepended — the chat API requires a real `modelId`.
pub fn parse_models_json(body: &str) -> Result<Vec<ModelOption>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("모델 목록 JSON 파싱 실패: {e}"))?;
    let arr = v
        .as_array()
        .ok_or("모델 목록 형식 오류: 최상위가 JSON 배열이 아닙니다")?;
    let mut out = Vec::new();
    for item in arr {
        let model_id = match item.get("modelId").and_then(|x| x.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => continue,
        };
        let label = pick_ko_name(item.get("name")).unwrap_or_else(|| model_id.clone());
        out.push(ModelOption { id: model_id, label });
    }
    Ok(out)
}

/// Fetch and parse the Fabrix model list. Blocking — call on a worker thread.
fn fetch_models(cfg: &FabrixConfig) -> Result<Vec<ModelOption>, String> {
    let client = build_client(cfg.allow_invalid_certs, Some(MODELS_TIMEOUT))?;
    let url = format!("{}/openapi/chat/v1/all-models", base(cfg));
    let mut req = client.get(&url);
    if let Some(c) = cfg.client.as_deref() {
        req = req.header("x-fabrix-client", c);
    }
    if let Some(t) = cfg.openapi_token.as_deref() {
        req = req.header("x-openapi-token", t);
    }
    let resp = req.send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(format!("HTTP {status} — {}", body.trim()));
    }
    let text = resp.text().map_err(|e| e.to_string())?;
    parse_models_json(&text)
}

/// Detection for Fabrix: config presence + (cache-first) model list. Blocking —
/// call via `spawn_blocking`. Mirrors the shape of `detect::detect_agent_blocking`
/// (the free-form `source`/`diagnostic` strings carry remote-specific values).
///
/// Cache-first (D66): with `force == false` and a cached model list present, the
/// stored list is returned with NO network call (app start / normal load). A
/// live fetch happens only on `force == true` (save / refresh / connection test)
/// or when there is no cache yet (first configure). On a forced fetch failure the
/// cache, if any, is still shown as a fallback so the model dropdown stays usable.
pub fn detect_fabrix(cfg: Option<FabrixConfig>, force: bool) -> DetectedAgent {
    let mut agent = DetectedAgent {
        id: "fabrix".to_string(),
        name: "Fabrix".to_string(),
        available: false,
        path: None,
        version: None,
        source: "not-found".to_string(),
        models: Vec::new(),
        models_source: "fallback".to_string(),
        diagnostic: None,
    };

    let cfg = match cfg {
        Some(c) if !c.endpoint_url.trim().is_empty() => c,
        _ => {
            agent.diagnostic = Some("not-configured".to_string());
            return agent;
        }
    };

    // Cache path: use the stored list without touching the network.
    if !force && !cfg.models.is_empty() {
        agent.available = true;
        agent.source = "remote".to_string();
        agent.models = cfg.models.clone();
        agent.models_source = "fallback".to_string();
        return agent;
    }

    match fetch_models(&cfg) {
        Ok(models) => {
            agent.available = true;
            agent.source = "remote".to_string();
            agent.models = models;
            agent.models_source = "live".to_string();
        }
        Err(_) => {
            // Configured but the endpoint didn't answer / rejected us. The
            // detailed error surfaces via the "connection test" (probe_fabrix).
            agent.source = "remote".to_string();
            agent.diagnostic = Some("unreachable".to_string());
            // Keep showing the last-known models as a fallback if we have them.
            if !cfg.models.is_empty() {
                agent.models = cfg.models.clone();
                agent.models_source = "fallback".to_string();
            }
        }
    }
    agent
}

// ---------------------------------------------------------------------------
// Chat (SSE)
// ---------------------------------------------------------------------------

/// Parse one SSE `data:` payload (already stripped of the `data:` prefix) into
/// zero or more `RunEvent`s. Mirrors the Python sample: `event_status=="CHUNK"`
/// with non-empty `content` is a text chunk; a failure `status` becomes an
/// error. The terminal `status=="SUCCESS"`/`R20000` marker yields no event —
/// the worker emits the single terminal `End` after the stream closes (same
/// convention as the CLI parsers in `run.rs`).
pub fn parse_fabrix_sse_data(data: &str) -> Vec<RunEvent> {
    let data = data.trim();
    if data.is_empty() {
        return vec![];
    }
    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("");
    let event_status = v.get("event_status").and_then(|x| x.as_str()).unwrap_or("");
    let content = v.get("content").and_then(|x| x.as_str()).unwrap_or("");

    // A chunk of assistant text.
    if event_status == "CHUNK" && !content.is_empty() {
        return vec![RunEvent::TextDelta { delta: content.to_string() }];
    }

    // An explicit failure status (heuristic — avoids treating SUCCESS/PROCESSING
    // markers as errors). Per-connection failures (auth, non-200) are handled by
    // the HTTP status check before streaming.
    let status_up = status.to_ascii_uppercase();
    if status_up.contains("FAIL") || status_up.contains("ERROR") {
        let response_code = v.get("response_code").and_then(|x| x.as_str()).unwrap_or("");
        let msg = v
            .get("message")
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .or_else(|| (!content.is_empty()).then(|| content.to_string()))
            .unwrap_or_else(|| format!("Fabrix 오류 (status={status}, code={response_code})"));
        return vec![RunEvent::Error { message: msg }];
    }

    vec![]
}

/// The chat request body. `isStream: true` (token streaming — chosen over the
/// Python sample's contradictory `False`); `llmConfig` uses the sample defaults.
fn chat_body(model: &str, prompt: &str) -> Value {
    serde_json::json!({
        "modelIds": [model],
        "contents": [prompt],
        "llmConfig": {
            // Raised from the sample's 2024 so long document-step artifacts
            // (plans/analyses) are not truncated mid-content (D67).
            "max_new_tokens": 8192,
            "seed": Value::Null,
            "top_k": 14,
            "top_p": 0.94,
            "temperature": 0.4,
            "repetition_penalty": 1.04
        },
        "isStream": true,
        "systemPrompt": "안녕하세요. 사용자 질문에 친절히 대답해주세요."
    })
}

/// Start a Fabrix chat run. Loads config, mints a run id, registers a cancel
/// flag, then streams the SSE response on a worker thread — the analog of
/// `run::run_agent` for the HTTP transport (returns the id immediately; a single
/// terminal `End` follows once the stream closes). Cancellation is cooperative:
/// the read loop polls the flag between SSE lines and drops the connection.
pub fn run_fabrix(
    app: tauri::AppHandle,
    args: RunArgs,
    on_event: Channel<RunEvent>,
) -> Result<String, String> {
    let config_dir = crate::ow_home()?;
    let cfg = settings::load(&config_dir)
        .fabrix
        .filter(|c| !c.endpoint_url.trim().is_empty())
        .ok_or("Fabrix 연결 정보가 설정되지 않았습니다. Agents 화면에서 저장하세요.")?;

    // The chat API requires a concrete modelId (no "default"); the frontend
    // seeds one, but guard defensively.
    let model = match args.model.as_deref() {
        Some(m) if !m.trim().is_empty() && m != "default" => m.to_string(),
        _ => return Err("Fabrix 모델을 선택해 주세요.".to_string()),
    };

    let registry = app.state::<RunRegistry>();
    let run_id = registry.next_id();
    let canceled = registry.register_remote(&run_id);

    let url = format!("{}/openapi/chat/v1/messages", base(&cfg));
    let body = chat_body(&model, &args.prompt);
    let app2 = app.clone();
    let run_id2 = run_id.clone();

    std::thread::spawn(move || {
        let finish = |status: &str| {
            let reg = app2.state::<RunRegistry>();
            reg.unregister(&run_id2);
            let _ = on_event.send(RunEvent::End { code: None, status: status.to_string() });
        };

        let client = match build_client(cfg.allow_invalid_certs, None) {
            Ok(c) => c,
            Err(e) => {
                let _ = on_event.send(RunEvent::Error { message: e });
                finish("failed");
                return;
            }
        };

        let mut req = client.post(&url).json(&body);
        if let Some(c) = cfg.client.as_deref() {
            req = req.header("x-fabrix-client", c);
        }
        if let Some(t) = cfg.openapi_token.as_deref() {
            req = req.header("x-openapi-token", t);
        }

        let resp = match req.send() {
            Ok(r) => r,
            Err(e) => {
                let _ = on_event.send(RunEvent::Error { message: format!("Fabrix 요청 실패: {e}") });
                finish("failed");
                return;
            }
        };

        let http_status = resp.status();
        if !http_status.is_success() {
            let text = resp.text().unwrap_or_default();
            let _ = on_event.send(RunEvent::Error {
                message: format!("Fabrix HTTP {http_status} — {}", text.trim()),
            });
            finish("failed");
            return;
        }

        // Announce the model for the UI (no session id — Fabrix is sessionless).
        let _ = on_event.send(RunEvent::Status {
            label: "streaming".to_string(),
            model: Some(model.clone()),
            session_id: None,
        });

        // Stream the SSE body line-by-line, decoding lossily (an invalid UTF-8
        // byte becomes U+FFFD rather than aborting the rest of the reply — same
        // rationale as `run::stream_lines`). Poll the cancel flag between lines.
        let mut reader = BufReader::new(resp);
        let mut buf = Vec::new();
        let mut had_error = false;
        loop {
            if canceled.load(Ordering::Relaxed) {
                break; // dropping `reader` below closes the connection
            }
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(['\r', '\n']);
                    // SSE frames: only `data:` lines carry the JSON payload.
                    let data = match line.strip_prefix("data:") {
                        Some(rest) => rest.trim_start(),
                        None => continue,
                    };
                    for ev in parse_fabrix_sse_data(data) {
                        if matches!(ev, RunEvent::Error { .. }) {
                            had_error = true;
                        }
                        let _ = on_event.send(ev);
                    }
                }
                Err(_) => {
                    had_error = true;
                    break;
                }
            }
        }
        drop(reader);

        let status = if canceled.load(Ordering::Relaxed) {
            "canceled"
        } else if had_error {
            "failed"
        } else {
            "succeeded"
        };
        finish(status);
    });

    Ok(run_id)
}

/// Connection test for the Fabrix card ("연결 테스트"): fetch the model list and
/// report the count. `async` + `spawn_blocking` (same one-shot pattern as
/// `rag_search`/`probe_confluence`).
#[tauri::command]
pub async fn probe_fabrix() -> Result<String, String> {
    let config_dir = crate::ow_home()?;
    let cfg = settings::load(&config_dir)
        .fabrix
        .filter(|c| !c.endpoint_url.trim().is_empty())
        .ok_or("Fabrix 연결 정보가 설정되지 않았습니다.")?;
    tauri::async_runtime::spawn_blocking(move || {
        let models = fetch_models(&cfg)?;
        // Cache the fresh list so the model dropdown survives restarts (D66).
        let mut s = settings::load(&config_dir);
        if let Some(f) = s.fabrix.as_mut() {
            f.models = models.clone();
            let _ = settings::save(&config_dir, &s);
        }
        Ok::<String, String>(format!("연결됨 ({}개 모델)", models.len()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_uses_cache_without_network_when_not_forced() {
        // Cache-first (D66): a config with cached models + force=false returns
        // the cached list directly — no HTTP call (the endpoint is bogus, so a
        // live fetch would fail).
        let cfg = FabrixConfig {
            endpoint_url: "http://127.0.0.1:9/never".into(),
            client: None,
            openapi_token: None,
            allow_invalid_certs: false,
            models: vec![ModelOption { id: "cached".into(), label: "캐시 모델".into() }],
        };
        let agent = detect_fabrix(Some(cfg), false);
        assert!(agent.available);
        assert_eq!(agent.source, "remote");
        assert_eq!(agent.models_source, "fallback");
        assert_eq!(agent.models.len(), 1);
        assert_eq!(agent.models[0].id, "cached");
        assert!(agent.diagnostic.is_none());
    }

    #[test]
    fn detect_reports_not_configured_without_endpoint() {
        let agent = detect_fabrix(None, false);
        assert!(!agent.available);
        assert!(agent.models.is_empty());
        assert_eq!(agent.diagnostic.as_deref(), Some("not-configured"));
    }

    #[test]
    fn parses_models_with_korean_names() {
        let body = r#"[
            { "modelId": "m-1", "name": [
                { "languageCode": "en", "content": "Model One" },
                { "languageCode": "ko", "content": "모델 하나" }
            ], "description": [], "types": ["chat"] },
            { "modelId": "m-2", "name": [
                { "languageCode": "ko", "content": "모델 둘" }
            ] }
        ]"#;
        let models = parse_models_json(body).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "m-1");
        assert_eq!(models[0].label, "모델 하나");
        assert_eq!(models[1].id, "m-2");
        assert_eq!(models[1].label, "모델 둘");
    }

    #[test]
    fn falls_back_when_no_korean_name() {
        // No ko → first non-empty content.
        let body = r#"[{ "modelId": "m-3", "name": [{ "languageCode": "en", "content": "Only English" }] }]"#;
        let models = parse_models_json(body).unwrap();
        assert_eq!(models[0].label, "Only English");

        // No usable name at all → the modelId is the label.
        let body2 = r#"[{ "modelId": "m-4", "name": [] }, { "modelId": "m-5" }]"#;
        let models2 = parse_models_json(body2).unwrap();
        assert_eq!(models2[0].label, "m-4");
        assert_eq!(models2[1].label, "m-5");
    }

    #[test]
    fn skips_items_without_model_id_and_rejects_non_array() {
        let body = r#"[{ "name": [{ "languageCode": "ko", "content": "no id" }] }, { "modelId": "ok" }]"#;
        let models = parse_models_json(body).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "ok");

        assert!(parse_models_json("not json").is_err());
        assert!(parse_models_json(r#"{"models":[]}"#).is_err(), "top-level must be an array");
    }

    #[test]
    fn sse_chunk_becomes_text_delta() {
        let evs = parse_fabrix_sse_data(
            r#"{"status":"PROCESSING","event_status":"CHUNK","content":"안녕"}"#,
        );
        assert_eq!(evs, vec![RunEvent::TextDelta { delta: "안녕".to_string() }]);
    }

    #[test]
    fn sse_success_marker_emits_nothing() {
        // Terminal marker → no event (the worker emits the single End).
        let evs = parse_fabrix_sse_data(
            r#"{"status":"SUCCESS","response_code":"R20000","event_status":"DONE","content":""}"#,
        );
        assert!(evs.is_empty());
        // Empty chunk content also emits nothing.
        let evs2 = parse_fabrix_sse_data(r#"{"event_status":"CHUNK","content":""}"#);
        assert!(evs2.is_empty());
        // Non-JSON / empty → nothing.
        assert!(parse_fabrix_sse_data("").is_empty());
        assert!(parse_fabrix_sse_data("keep-alive").is_empty());
    }

    #[test]
    fn sse_failure_status_becomes_error() {
        let evs = parse_fabrix_sse_data(
            r#"{"status":"FAIL","response_code":"R50000","message":"모델을 찾을 수 없습니다"}"#,
        );
        assert_eq!(
            evs,
            vec![RunEvent::Error { message: "모델을 찾을 수 없습니다".to_string() }]
        );
        // Without a message field, a synthetic one is built.
        let evs2 = parse_fabrix_sse_data(r#"{"status":"ERROR","response_code":"R50001"}"#);
        assert_eq!(evs2.len(), 1);
        assert!(matches!(&evs2[0], RunEvent::Error { message } if message.contains("R50001")));
    }
}

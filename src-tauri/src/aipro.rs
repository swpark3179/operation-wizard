//! AI Pro — a remote OpenAI-compatible HTTP API agent (D71).
//!
//! AI Pro was once wired as a local gemini-compatible CLI (`aipro` binary), but
//! the in-house backend is actually an OpenAI-compatible service. Like Fabrix
//! (`fabrix.rs`, D64) it is reached over HTTP, not by spawning a process — the
//! only endpoint used is the chat one:
//!   - chat: `POST {endpoint}/chat/completions`  (SSE stream) — also the
//!           connection test (a minimal non-stream call).
//!
//! The protocol is the OpenAI surface the `opencode` CLI uses for the same
//! backend: auth is a single `Authorization: Bearer <apiKey>` header (vs.
//! Fabrix's two custom headers), the chat body is
//! `{model, messages, stream, stream_options}`, and the SSE stream is a series
//! of `chat.completion.chunk` objects (`choices[0].delta.content`) ending with a
//! `data: [DONE]` marker.
//!
//! **A `User-Agent: opencode/<version>` header is mandatory** (D74): the SDS
//! gateway allowlists the opencode-based official client by UA (any other UA →
//! HTTP 406) and the backend does `ua.split("/")` (a *missing* UA → HTTP 500
//! `'NoneType'...'split'`). `reqwest` sends no UA by default, so it is set on the
//! shared `build_client`. The model list is the **static catalog** in
//! `agents.rs` (`fallback_models`) — the 3 model ids are known, so `/models` is
//! not called; reachability is verified by the connection test (a minimal chat
//! call) and by real chats (D73).
//!
//! The HTTP client reuses the same `reqwest` blocking + native-tls (schannel)
//! recipe as `fabrix.rs`/`rag.rs` — no new crate, and the OS cert store is
//! trusted. `.no_proxy()` connects straight to the (directly-reachable) internal
//! endpoint, ignoring any proxy env var (D66).

use std::io::{BufRead, BufReader};
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::Value;
use tauri::ipc::Channel;
use tauri::Manager;

use crate::detect::{DetectedAgent, ModelOption};
use crate::run::{RunArgs, RunEvent, RunRegistry};
use crate::settings::{self, AiProConfig};

/// Connect timeout for every AI Pro request. The chat stream sets no *total*
/// timeout (a chat may run for minutes — the run engine is also untimed, D53);
/// the (short) non-stream connection test adds a total timeout on top.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Total timeout for the (short) non-stream connection-test call.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Baseline system prompt. The real workflow instructions are injected into the
/// *user* prompt (D67 `remoteDocCtx`), so this is only a neutral persona.
const SYSTEM_PROMPT: &str = "사용자 질문에 정확하고 도움이 되게 답합니다.";

/// **Required** User-Agent (D74). The SDS AI Pro gateway allowlists the
/// opencode-based official client by `User-Agent` and does `ua.split("/")` on
/// the backend: any other UA is rejected with HTTP 406 ("Not Acceptable"), and a
/// *missing* UA makes the backend crash with HTTP 500
/// (`'NoneType' object has no attribute 'split'`). `reqwest` sends no UA by
/// default, which is why every request failed until this was set. A
/// `opencode/<version>` value passes the gateway and the backend parse
/// (verified live against `/chat/completions`, streaming and non-streaming).
const OPENCODE_UA: &str = "opencode/0.1.0";

/// Build a blocking HTTP client. `total` is the whole-request timeout; pass
/// `None` for the streaming chat so long responses are not cut off.
fn build_client(allow_invalid_certs: bool, total: Option<Duration>) -> Result<reqwest::blocking::Client, String> {
    let mut b = reqwest::blocking::Client::builder()
        // AI Pro is a directly-reachable corporate endpoint — connect straight
        // to it and ignore any HTTP(S)_PROXY/ALL_PROXY env var (D66).
        .no_proxy()
        // Mandatory: the gateway allowlists this UA and the backend splits it —
        // without it every request 500s/406s (D74).
        .user_agent(OPENCODE_UA)
        .connect_timeout(CONNECT_TIMEOUT)
        .danger_accept_invalid_certs(allow_invalid_certs);
    if let Some(t) = total {
        b = b.timeout(t);
    }
    b.build().map_err(|e| e.to_string())
}

fn base(cfg: &AiProConfig) -> String {
    cfg.endpoint_url.trim().trim_end_matches('/').to_string()
}

/// The agent's static fallback catalog (`agents.rs` `fallback_models`), mapped to
/// `ModelOption`s. Used when `/models` is unreachable and no cache exists — no
/// synthetic `default` is added (the chat API needs a real model id). Kept as a
/// pure helper so it can be unit-tested without a network call.
fn static_fallback_models() -> Vec<ModelOption> {
    crate::agents::find("aipro")
        .map(|d| {
            d.fallback_models
                .iter()
                .map(|(id, label)| ModelOption { id: (*id).into(), label: (*label).into() })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Connection test (minimal chat)
// ---------------------------------------------------------------------------

/// The model used for the connection test: the first entry of the static catalog
/// (`glm-5.1`) — AI Pro's default model, matching opencode.json's first model.
fn probe_model() -> String {
    static_fallback_models()
        .first()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| "glm-5.1".to_string())
}

/// Verify reachability + auth with a **minimal non-stream `POST /chat/completions`**
/// — the exact path opencode uses successfully (and the only one this SDS gateway
/// serves reliably; `GET /models` returns a 500, D73). HTTP 2xx = OK. Blocking —
/// call on a worker thread.
fn chat_probe(cfg: &AiProConfig, model: &str) -> Result<(), String> {
    let client = build_client(cfg.allow_invalid_certs, Some(PROBE_TIMEOUT))?;
    let url = format!("{}/chat/completions", base(cfg));
    let body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": "ping" }],
        "max_tokens": 1,
        "stream": false
    });
    let mut req = client.post(&url).header("Accept", "application/json").json(&body);
    if let Some(k) = cfg.api_key.as_deref() {
        req = req.header("Authorization", format!("Bearer {k}"));
    }
    let resp = req.send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {status} — {}", resp.text().unwrap_or_default().trim()))
    }
}

/// Detection for AI Pro: **no network** (D73). Configured (endpoint present) →
/// `available` with the static model catalog; unconfigured → `not-configured`.
/// The gateway has no cheap health endpoint (`GET /models` is broken), so
/// reachability is not probed on every detect (which runs at app start / refresh
/// and would cost a chat token) — it is verified by the explicit connection test
/// (`probe_aipro`) and by real chats. `force` is unused (kept for the shared
/// `detect_agent` remote-branch signature with Fabrix).
pub fn detect_aipro(cfg: Option<AiProConfig>, _force: bool) -> DetectedAgent {
    let mut agent = DetectedAgent {
        id: "aipro".to_string(),
        name: "AI Pro".to_string(),
        available: false,
        path: None,
        version: None,
        source: "not-found".to_string(),
        models: Vec::new(),
        models_source: "fallback".to_string(),
        diagnostic: None,
    };

    match cfg {
        Some(c) if !c.endpoint_url.trim().is_empty() => {
            agent.available = true;
            agent.source = "remote".to_string();
            agent.models = static_fallback_models();
            agent.models_source = "fallback".to_string();
        }
        _ => {
            agent.diagnostic = Some("not-configured".to_string());
        }
    }
    agent
}

// ---------------------------------------------------------------------------
// Chat (SSE)
// ---------------------------------------------------------------------------

/// Parse one SSE `data:` payload (already stripped of the `data:` prefix) into
/// zero or more `RunEvent`s. OpenAI streaming: each payload is a
/// `chat.completion.chunk` whose `choices[0].delta.content` is a text fragment;
/// the terminal `[DONE]` marker (and empty/role-only deltas) yield no event — the
/// worker emits the single terminal `End` after the stream closes. A final chunk
/// with `usage` (from `stream_options.include_usage`) becomes a `Usage` event; a
/// top-level `error` object becomes an `Error`.
pub fn parse_openai_sse_data(data: &str) -> Vec<RunEvent> {
    let data = data.trim();
    // `[DONE]` is the OpenAI stream terminator; it is not JSON.
    if data.is_empty() || data == "[DONE]" {
        return vec![];
    }
    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    // Some OpenAI-compatible servers stream an error object mid-stream instead of
    // (or after) a non-2xx status. Surface it as an error.
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("AI Pro 오류: {err}"));
        return vec![RunEvent::Error { message: msg }];
    }

    let mut out = Vec::new();

    let delta = v
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("delta"));

    // Reasoning delta (glm-5.1 streams `delta.reasoning` before `delta.content`)
    // → the assistant "thinking" section (D74). `reasoning_content` is the
    // alternate field some backends use.
    if let Some(reasoning) = delta
        .and_then(|d| d.get("reasoning").or_else(|| d.get("reasoning_content")))
        .and_then(|x| x.as_str())
    {
        if !reasoning.is_empty() {
            out.push(RunEvent::ThinkingDelta { delta: reasoning.to_string() });
        }
    }

    // Assistant text delta.
    if let Some(content) = delta.and_then(|d| d.get("content")).and_then(|x| x.as_str()) {
        if !content.is_empty() {
            out.push(RunEvent::TextDelta { delta: content.to_string() });
        }
    }

    // Token usage (final chunk, when include_usage is set).
    if let Some(usage) = v.get("usage").filter(|u| u.is_object()) {
        let input_tokens = usage.get("prompt_tokens").and_then(|x| x.as_u64());
        let output_tokens = usage.get("completion_tokens").and_then(|x| x.as_u64());
        if input_tokens.is_some() || output_tokens.is_some() {
            out.push(RunEvent::Usage { input_tokens, output_tokens });
        }
    }

    out
}

/// The OpenAI-compatible chat request body. `stream: true` with
/// `stream_options.include_usage` so the final chunk carries token counts.
/// `max_tokens` 8192 mirrors Fabrix's `max_new_tokens` (D67) to avoid truncating
/// long document-step artifacts.
fn chat_body(model: &str, prompt: &str) -> Value {
    serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": prompt }
        ],
        "stream": true,
        "stream_options": { "include_usage": true },
        "temperature": 0.4,
        "max_tokens": 8192
    })
}

/// Start an AI Pro chat run. Loads config, mints a run id, registers a cancel
/// flag, then streams the SSE response on a worker thread — the analog of
/// `run::run_agent` for the HTTP transport (returns the id immediately; a single
/// terminal `End` follows once the stream closes). Cancellation is cooperative:
/// the read loop polls the flag between SSE lines and drops the connection.
/// Mirrors `fabrix::run_fabrix`.
pub fn run_aipro(
    app: tauri::AppHandle,
    args: RunArgs,
    on_event: Channel<RunEvent>,
) -> Result<String, String> {
    let config_dir = crate::ow_home()?;
    let cfg = settings::load(&config_dir)
        .aipro
        .filter(|c| !c.endpoint_url.trim().is_empty())
        .ok_or("AI Pro 연결 정보가 설정되지 않았습니다. Agents 화면에서 저장하세요.")?;

    // The chat API requires a concrete model id (no "default"); the frontend
    // seeds one, but guard defensively.
    let model = match args.model.as_deref() {
        Some(m) if !m.trim().is_empty() && m != "default" => m.to_string(),
        _ => return Err("AI Pro 모델을 선택해 주세요.".to_string()),
    };

    let registry = app.state::<RunRegistry>();
    let run_id = registry.next_id();
    let canceled = registry.register_remote(&run_id);

    let url = format!("{}/chat/completions", base(&cfg));
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

        let mut req = client
            .post(&url)
            .header("Accept", "text/event-stream")
            .json(&body);
        if let Some(k) = cfg.api_key.as_deref() {
            req = req.header("Authorization", format!("Bearer {k}"));
        }

        let resp = match req.send() {
            Ok(r) => r,
            Err(e) => {
                let _ = on_event.send(RunEvent::Error { message: format!("AI Pro 요청 실패: {e}") });
                finish("failed");
                return;
            }
        };

        let http_status = resp.status();
        if !http_status.is_success() {
            let text = resp.text().unwrap_or_default();
            let _ = on_event.send(RunEvent::Error {
                message: format!("AI Pro HTTP {http_status} — {}", text.trim()),
            });
            finish("failed");
            return;
        }

        // Announce the model for the UI (no session id — AI Pro is sessionless).
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
                    for ev in parse_openai_sse_data(data) {
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

/// Connection test for the AI Pro card ("연결 테스트"): a minimal non-stream chat
/// call verifies auth + reachability on the real path (D73 — `GET /models` is
/// broken on this gateway). `async` + `spawn_blocking` (same one-shot pattern as
/// `probe_fabrix`).
#[tauri::command]
pub async fn probe_aipro() -> Result<String, String> {
    let config_dir = crate::ow_home()?;
    let cfg = settings::load(&config_dir)
        .aipro
        .filter(|c| !c.endpoint_url.trim().is_empty())
        .ok_or("AI Pro 연결 정보가 설정되지 않았습니다.")?;
    tauri::async_runtime::spawn_blocking(move || {
        chat_probe(&cfg, &probe_model())?;
        Ok::<String, String>("연결됨 — AI Pro 응답 정상".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_configured_is_available_with_static_models() {
        // D73: no network at detect — a configured endpoint is `available` with
        // the static catalog (reachability is verified by the connection test).
        let cfg = AiProConfig {
            endpoint_url: "https://aipro.example.com/open/api/v1".into(),
            api_key: Some("k".into()),
            allow_invalid_certs: false,
            models: vec![], // ignored — detect always uses the static catalog now
        };
        let agent = detect_aipro(Some(cfg), false);
        assert!(agent.available);
        assert_eq!(agent.source, "remote");
        assert_eq!(agent.models_source, "fallback");
        assert!(agent.diagnostic.is_none());
        let ids: Vec<&str> = agent.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["glm-5.1", "qwen3.6-27b", "gpt-oss-120b"]);
    }

    #[test]
    fn detect_reports_not_configured_without_endpoint() {
        let agent = detect_aipro(None, false);
        assert!(!agent.available);
        assert!(agent.models.is_empty());
        assert_eq!(agent.diagnostic.as_deref(), Some("not-configured"));
    }

    #[test]
    fn static_fallback_is_the_known_catalog_without_default() {
        // `detect_aipro` (and the connection-test model) use this catalog: the 3
        // in-house ids, in order, with NO synthetic `default` (D71/D73).
        let m = static_fallback_models();
        let ids: Vec<&str> = m.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["glm-5.1", "qwen3.6-27b", "gpt-oss-120b"]);
        assert!(!ids.contains(&"default"), "remote model list has no `default`");
        assert_eq!(probe_model(), "glm-5.1");
    }

    #[test]
    fn sse_content_delta_becomes_text_delta() {
        let evs = parse_openai_sse_data(
            r#"{"object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"안녕"},"finish_reason":null}]}"#,
        );
        assert_eq!(evs, vec![RunEvent::TextDelta { delta: "안녕".to_string() }]);
    }

    #[test]
    fn sse_reasoning_delta_becomes_thinking_delta() {
        // glm-5.1 streams `delta.reasoning` before `delta.content` (D74).
        let evs = parse_openai_sse_data(
            r#"{"choices":[{"index":0,"delta":{"reasoning":"생각"},"finish_reason":null}]}"#,
        );
        assert_eq!(evs, vec![RunEvent::ThinkingDelta { delta: "생각".to_string() }]);
    }

    #[test]
    fn sse_done_and_empty_deltas_emit_nothing() {
        // Stream terminator → no event (the worker emits the single End).
        assert!(parse_openai_sse_data("[DONE]").is_empty());
        // Role-only opening delta → no text.
        assert!(parse_openai_sse_data(
            r#"{"choices":[{"index":0,"delta":{"role":"assistant"}}]}"#
        )
        .is_empty());
        // Empty content → nothing.
        assert!(parse_openai_sse_data(r#"{"choices":[{"delta":{"content":""}}]}"#).is_empty());
        // Non-JSON / empty → nothing.
        assert!(parse_openai_sse_data("").is_empty());
        assert!(parse_openai_sse_data(": keep-alive").is_empty());
    }

    #[test]
    fn sse_usage_chunk_becomes_usage_event() {
        let evs = parse_openai_sse_data(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":34,"total_tokens":46}}"#,
        );
        assert_eq!(
            evs,
            vec![RunEvent::Usage { input_tokens: Some(12), output_tokens: Some(34) }]
        );
    }

    #[test]
    fn sse_error_object_becomes_error() {
        let evs = parse_openai_sse_data(
            r#"{"error":{"message":"모델을 찾을 수 없습니다","type":"invalid_request_error"}}"#,
        );
        assert_eq!(
            evs,
            vec![RunEvent::Error { message: "모델을 찾을 수 없습니다".to_string() }]
        );
    }
}

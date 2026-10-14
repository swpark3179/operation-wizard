//! FabriX — the remote HTTP API agent (D64 → D84).
//!
//! Talks to the **FabriX LLM Serving API**, an OpenAI-compatible service
//! (WSO2 gateway → OIDC → litellm proxy → vLLM). Ported from the verified
//! reference `Desktop/FabrixSample` (`fabrix/config.py`, `client.py`,
//! `normalize.py`, `docs/API_REFERENCE.md`):
//!   - base   = `{endpoint}/openapi/llm`
//!   - models : `GET  {base}/v1/models` — a bare array (`{base}/models` is 405)
//!   - chat   : `POST {base}/chat/completions` — SSE `chat.completion.chunk`s,
//!              reasoning models stream `delta.reasoning` before `delta.content`,
//!              a `choices: []` usage chunk always arrives last, then `[DONE]`
//!   - auth   : `x-fabrix-client: <JWT>` + `x-openapi-token: Bearer <JWT>`
//!              (the `Bearer ` prefix is added when missing)
//!   - model  : chosen ONLY by the `x-llm-model-id` header — the body `model`
//!              is ignored, so it carries the conventional `"/mnt/models"`
//!   - always no-proxy (`.no_proxy()`); TLS is verified by the OS store
//!     (native-tls = schannel), never skipped
//!
//! A chat turn runs an app-side **tool-calling loop** with read-only file tools
//! (`fabrix_tools.rs`) so the model can read the codebase; document artifacts
//! are still persisted by the frontend (`write_file`, D67). `reqwest` blocking
//! + native-tls — no new crate (SSE and the loop are hand-rolled).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::ipc::Channel;
use tauri::Manager;

use crate::detect::{DetectedAgent, ModelOption};
use crate::fabrix_rate;
use crate::fabrix_tools::{self, Sandbox, ToolOutcome};
use crate::run::{RunArgs, RunEvent, RunRegistry};
use crate::settings::{self, FabrixConfig};

/// OpenAPI path appended to the issued endpoint (sample `FABRIX_PREFIX_PATH`).
const PREFIX_PATH: &str = "openapi/llm";
/// FabriX ignores the body `model` (the header selects it); this is the
/// conventional placeholder (sample `BODY_MODEL_PLACEHOLDER`).
const BODY_MODEL_PLACEHOLDER: &str = "/mnt/models";
/// Default model when none is configured: 70 = Glm 5.2 — recommended for
/// complex analysis and tool calling (sample README §2).
pub const DEFAULT_MODEL_ID: &str = "70";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// reqwest *blocking* applies `timeout` to the wait for response headers and to
/// every body read (default 30 s — which used to silently cap slow first
/// chunks). 180 s matches the sample's `timeout=180` "no response" ceiling; a
/// steadily streaming reply is never cut off.
const READ_TIMEOUT: Duration = Duration::from_secs(180);
/// Total timeout for the (short) model-list GET.
const MODELS_TIMEOUT: Duration = Duration::from_secs(30);
const TEMPERATURE: f64 = 0.4;

/// Tool-loop limits. Every round re-sends all earlier tool output, so cost grows
/// roughly with the square of the round count — keep both bounded.
const MAX_ROUNDS: usize = 12;
/// Tool-output budget when the model's context size is unknown.
const DEFAULT_TOOL_BUDGET: usize = 64_000;
/// Tool results shown in the chat timeline (and saved in session JSON) are
/// previews; the model always gets the full (capped) text.
const UI_PREVIEW_CHARS: usize = 2_000;

const LENGTH_MSG: &str = "FabriX 응답이 출력 토큰 한도(추론 토큰 포함, 게이트웨이 기본 16384)에 도달해 잘렸습니다(finish_reason=length). 요청 범위를 나누거나 다시 시도하세요.";

const HINT_MISSING_MODEL: &str =
    "x-llm-model-id(모델 ID) 헤더가 없거나 잘못되면 이 401이 납니다 — 모델을 다시 선택하세요.";
const HINT_UNKNOWN_MODEL: &str =
    "존재하지 않는 모델 ID입니다 — Agents 화면의 FabriX 카드에서 새로고침 후 모델을 다시 선택하세요.";
const HINT_TOKEN: &str = "x-openapi-token 값을 확인하세요('Bearer ' 접두사·만료 여부).";
const HINT_CLIENT: &str =
    "x-fabrix-client 키가 잘못되었거나 만료되었습니다 — FabriX 포털에서 재발급받아 저장하세요.";
const HINT_RATE: &str = "호출 한도를 초과했습니다 — 잠시 후 다시 시도하고, Agents 화면 FabriX 카드의 '분당 호출 한도'를 라이선스 값에 맞추세요.";
const HINT_PATH: &str =
    "API 경로 오류입니다 — 엔드포인트에는 발급받은 Endpoint URL만 입력하세요(/openapi/llm은 앱이 붙입니다).";

// ---------------------------------------------------------------------------
// Connection conventions
// ---------------------------------------------------------------------------

/// Normalize a user-entered endpoint to the issued `{ENDPOINT}`: trim, then
/// tolerantly strip a pasted base path or full request URL (`/openapi/llm`,
/// `/v1`, `/chat/completions`, `/v1/models`, `/models`, trailing slashes).
pub fn normalize_endpoint(raw: &str) -> String {
    const SUFFIXES: [&str; 5] = ["/chat/completions", "/models", "/v1", "/openapi/llm", "/"];
    let mut e = raw.trim().to_string();
    loop {
        let before = e.len();
        for s in SUFFIXES {
            if e.to_ascii_lowercase().ends_with(s) {
                e.truncate(e.len() - s.len());
            }
        }
        if e.len() == before {
            return e;
        }
    }
}

fn base_url(cfg: &FabrixConfig) -> String {
    format!("{}/{PREFIX_PATH}", normalize_endpoint(&cfg.endpoint_url))
}

/// `x-openapi-token` value with the required `Bearer ` prefix (a common mistake
/// is pasting only the issued token — the sample's `load_config` fixes it too).
fn auth_token(raw: &str) -> String {
    let t = raw.trim();
    if t.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("bearer ")) {
        t.to_string()
    } else {
        format!("Bearer {t}")
    }
}

/// Attach the two FabriX auth headers (each only when set).
fn with_auth(mut req: reqwest::blocking::RequestBuilder, cfg: &FabrixConfig) -> reqwest::blocking::RequestBuilder {
    if let Some(c) = cfg.client.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        req = req.header("x-fabrix-client", c);
    }
    if let Some(t) = cfg.openapi_token.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        req = req.header("x-openapi-token", auth_token(t));
    }
    req
}

/// Blocking HTTP client: always direct (no env/system proxy — sample §0), TLS
/// verified via the OS store. `timeout` = header wait + per-read ceiling.
fn build_client(timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())
}

/// The configured default model id (`None`/blank → "70").
fn default_model_id(cfg: &FabrixConfig) -> String {
    cfg.default_model_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_MODEL_ID)
        .to_string()
}

/// FabriX model ids are numeric (`modelId`). A cache holding anything else is
/// left over from the pre-D84 chat API (UUID ids) and must be refetched.
fn cache_is_current(models: &[ModelOption]) -> bool {
    !models.is_empty() && models.iter().all(|m| !m.id.is_empty() && m.id.chars().all(|c| c.is_ascii_digit()))
}

/// Pick the model for a turn, in the backend so every caller is covered (the
/// Home composer's `"default"`, side turns that send no model, sessions saved
/// with an old/unknown id): the requested id when the cache knows it (or, with
/// no usable cache, when it looks like a FabriX id), else the configured
/// default, else "70".
pub fn resolve_model(requested: Option<&str>, cfg: &FabrixConfig) -> String {
    let req = requested.map(str::trim).filter(|m| !m.is_empty() && *m != "default");
    match req {
        Some(m) if cache_is_current(&cfg.models) => {
            if cfg.models.iter().any(|x| x.id == m) {
                m.to_string()
            } else {
                default_model_id(cfg)
            }
        }
        Some(m) if m.chars().all(|c| c.is_ascii_digit()) => m.to_string(),
        _ => default_model_id(cfg),
    }
}

/// Move the default model to the front: ChatPanel snaps an unset/invalid model
/// to `models[0]`, so this is what makes the default the initial selection.
fn order_default_first(mut models: Vec<ModelOption>, default_id: &str) -> Vec<ModelOption> {
    if let Some(pos) = models.iter().position(|m| m.id == default_id) {
        let m = models.remove(pos);
        models.insert(0, m);
    }
    models
}

// ---------------------------------------------------------------------------
// Model list
// ---------------------------------------------------------------------------

/// `[{"languageCode": "ko", "content": "…"}, …]` (or a bare string) → the ko,
/// then en, then first non-empty content (sample `_pick_lang`).
fn pick_lang(v: Option<&Value>) -> Option<String> {
    let v = v?;
    if let Some(s) = v.as_str() {
        return Some(s.trim().to_string()).filter(|s| !s.is_empty());
    }
    let arr = v.as_array()?;
    let content_for = |lang: Option<&str>| {
        arr.iter().find_map(|e| {
            let ok = lang.map_or(true, |l| e.get("languageCode").and_then(|x| x.as_str()) == Some(l));
            e.get("content")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| ok && !s.is_empty())
                .map(str::to_string)
        })
    };
    content_for(Some("ko")).or_else(|| content_for(Some("en"))).or_else(|| content_for(None))
}

/// Parse a model description's meta lines (`[Context] 128K`, `[Tools] Function
/// Calling, Knowledge`) — sample `parse_model_description`. Returns
/// `(context, supports_function_calling)`. `str::trim` also drops the `\u{a0}`
/// the portal sometimes leaves in these lines.
fn parse_description_meta(text: &str) -> (Option<String>, bool) {
    let mut context = None;
    let mut tools = false;
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('[') else { continue };
        let Some((key, value)) = rest.split_once(']') else { continue };
        match key.trim().to_ascii_lowercase().as_str() {
            "context" => context = Some(value.trim().to_string()).filter(|v| !v.is_empty()),
            "tools" => {
                tools = value
                    .split(',')
                    .any(|t| t.trim().eq_ignore_ascii_case("function calling"))
            }
            _ => {}
        }
    }
    (context, tools)
}

/// Parse `GET {base}/v1/models`: a bare array (or `{data:[…]}`) of
/// `{modelId, name[], description[]}`. `modelId` (number or string) is the
/// `x-llm-model-id` value — `modelServingId` repeats (13 and 16 share one), so
/// it is never the key. No synthetic `default` is added.
pub fn parse_models(body: &str) -> Result<Vec<ModelOption>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("모델 목록 JSON 파싱 실패: {e}"))?;
    let items = match &v {
        Value::Array(a) => a,
        Value::Object(o) => o
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or("모델 목록 형식 오류: 배열이 아닙니다")?,
        _ => return Err("모델 목록 형식 오류: 배열이 아닙니다".into()),
    };
    let mut out = Vec::new();
    for item in items {
        let id = match item.get("modelId") {
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
            _ => continue,
        };
        let label = pick_lang(item.get("name")).unwrap_or_else(|| id.clone());
        let (context, tools) = parse_description_meta(&pick_lang(item.get("description")).unwrap_or_default());
        out.push(ModelOption { id, label, context, tools: Some(tools) });
    }
    Ok(out)
}

/// Fetch the model list (`x-llm-model-id` is not needed here). Blocking.
fn fetch_models(cfg: &FabrixConfig) -> Result<Vec<ModelOption>, String> {
    let client = build_client(MODELS_TIMEOUT)?;
    let url = format!("{}/v1/models", base_url(cfg));
    // Counted (not paced): detection must not stall on the quota (D91).
    fabrix_rate::record();
    let resp = with_auth(client.get(&url), cfg)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| format!("FabriX 요청 실패: {e}"))?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(error_message(status.as_u16(), &text));
    }
    parse_models(&text)
}

/// Persist a freshly fetched list into `settings.fabrix.models` — only if the
/// saved connection still points at the same endpoint (best-effort, D66).
fn save_cache(dir: &Path, cfg: &FabrixConfig, models: &[ModelOption]) {
    let mut s = settings::load(dir);
    if let Some(f) = s.fabrix.as_mut() {
        if normalize_endpoint(&f.endpoint_url) == normalize_endpoint(&cfg.endpoint_url) {
            f.models = models.to_vec();
            let _ = settings::save(dir, &s);
        }
    }
}

/// Detection for FabriX: config presence + (cache-first, D66) model list.
/// Blocking — call via `spawn_blocking`. With `force == false` and a current
/// cache the stored list is returned with NO network call; otherwise the list
/// is fetched live and (when `persist` is given) cached. The default model is
/// listed first. On a fetch failure the current cache, if any, stays visible.
pub fn detect_fabrix(cfg: Option<FabrixConfig>, force: bool, persist: Option<&Path>) -> DetectedAgent {
    let mut agent = DetectedAgent {
        id: "fabrix".to_string(),
        name: "FabriX".to_string(),
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
    let default_id = default_model_id(&cfg);
    agent.source = "remote".to_string();

    if !force && cache_is_current(&cfg.models) {
        agent.available = true;
        agent.models = order_default_first(cfg.models.clone(), &default_id);
        return agent;
    }

    match fetch_models(&cfg) {
        Ok(models) => {
            if let Some(dir) = persist {
                save_cache(dir, &cfg, &models);
            }
            agent.available = true;
            agent.models = order_default_first(models, &default_id);
            agent.models_source = "live".to_string();
        }
        Err(_) => {
            // Configured but unreachable/rejected; the detailed error surfaces
            // via the connection test (probe_fabrix).
            agent.diagnostic = Some("unreachable".to_string());
            if cache_is_current(&cfg.models) {
                agent.models = order_default_first(cfg.models.clone(), &default_id);
            }
        }
    }
    agent
}

/// Connection test for the FabriX card ("연결 테스트"): fetch the model list,
/// cache it (D66) and report the count + the default model.
#[tauri::command]
pub async fn probe_fabrix() -> Result<String, String> {
    let config_dir = crate::ow_home()?;
    let cfg = settings::load(&config_dir)
        .fabrix
        .filter(|c| !c.endpoint_url.trim().is_empty())
        .ok_or("FabriX 연결 정보가 설정되지 않았습니다.")?;
    tauri::async_runtime::spawn_blocking(move || {
        let models = fetch_models(&cfg)?;
        save_cache(&config_dir, &cfg, &models);
        let def = default_model_id(&cfg);
        let def_label = match models.iter().find(|m| m.id == def) {
            Some(m) => format!("{} · id {}", m.label, m.id),
            None => format!("id {def} — 목록에 없음, 카드에서 기본 모델을 다시 선택하세요"),
        };
        Ok::<String, String>(format!("연결됨 — {}개 모델 (기본: {def_label})", models.len()))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// Errors (sample `to_openai_error`)
// ---------------------------------------------------------------------------

/// Turn any FabriX error body — litellm `{"error":{…}}`, OIDC
/// `{"code":"OIDC-…"}`, WSO2 `{"code":"900901","message","description"}` or
/// plain text — into one line: `FabriX HTTP {status} — {message}` (+ a Korean
/// hint). The `HTTP {status}` lead is kept so the frontend's retry filter and
/// `errorHint` still classify the failure.
pub fn error_message(status: u16, body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body.trim()).ok();
    let text = |v: &Value, k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let code_of = |v: &Value| match v.get("code") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    let mut hint: Option<&str> = None;
    let message = match parsed.as_ref() {
        Some(v) if v.get("error").is_some_and(|e| e.is_object()) => {
            let m = text(&v["error"], "message").unwrap_or_else(|| format!("HTTP {status}"));
            let lower = m.to_ascii_lowercase();
            if status == 401 && lower.contains("proxy server token") {
                hint = Some(HINT_MISSING_MODEL);
            } else if status == 404 && lower.contains("model") {
                hint = Some(HINT_UNKNOWN_MODEL);
            }
            m
        }
        Some(v) if code_of(v).is_some_and(|c| c.starts_with("OIDC")) => {
            let code = code_of(v).unwrap_or_default();
            let m = text(v, "message").or_else(|| text(v, "error")).unwrap_or_else(|| "Unauthorized".into());
            hint = Some(HINT_CLIENT);
            format!("{m} ({code})")
        }
        Some(v) if code_of(v).is_some() => {
            let code = code_of(v).unwrap_or_default();
            let mut m = text(v, "message").unwrap_or_else(|| format!("HTTP {status}"));
            if let Some(d) = text(v, "description") {
                m = format!("{m}: {d}");
            }
            if code.starts_with("9009") {
                hint = Some(HINT_TOKEN);
            }
            m
        }
        _ => {
            let t = body.trim();
            if t.is_empty() {
                format!("HTTP {status}")
            } else {
                t.chars().take(1000).collect()
            }
        }
    };
    if hint.is_none() {
        hint = match status {
            429 => Some(HINT_RATE),
            405 => Some(HINT_PATH),
            _ => None,
        };
    }
    match hint {
        Some(h) => format!("FabriX HTTP {status} — {message} [안내: {h}]"),
        None => format!("FabriX HTTP {status} — {message}"),
    }
}

// ---------------------------------------------------------------------------
// SSE stream accumulation
// ---------------------------------------------------------------------------

/// One tool call assembled from streamed `delta.tool_calls` pieces.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolCallAcc {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Accumulates one round's SSE stream. `feed` returns the events to emit live
/// (thinking/text deltas, stream errors); tool calls, the finish reason and
/// usage are collected for the loop. Pure — unit-tested with captured fixtures.
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    pub content: String,
    pub tool_calls: BTreeMap<u64, ToolCallAcc>,
    pub finish_reason: Option<String>,
    pub usage: Option<(Option<u64>, Option<u64>)>,
    /// `[DONE]` seen.
    pub done: bool,
}

impl StreamAccumulator {
    /// Feed one SSE `data:` payload (prefix already stripped).
    pub fn feed(&mut self, data: &str) -> Vec<RunEvent> {
        let data = data.trim();
        if data.is_empty() {
            return vec![];
        }
        if data == "[DONE]" {
            self.done = true;
            return vec![];
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { return vec![] };

        // Mid-stream error object (sample `stream_error`).
        if let Some(err) = v.get("error") {
            let code = err
                .get("code")
                .and_then(|c| c.as_str().map(str::to_string).or_else(|| c.as_u64().map(|n| n.to_string())))
                .and_then(|c| c.parse::<u16>().ok())
                .unwrap_or(500);
            return vec![RunEvent::Error { message: error_message(code, data) }];
        }

        // The trailing `choices: []` chunk carries only usage (always sent).
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            self.usage = Some((
                u.get("prompt_tokens").and_then(|x| x.as_u64()),
                u.get("completion_tokens").and_then(|x| x.as_u64()),
            ));
        }

        let mut out = Vec::new();
        let Some(choices) = v.get("choices").and_then(|c| c.as_array()) else { return out };
        for (ci, choice) in choices.iter().enumerate() {
            if let Some(f) = choice.get("finish_reason").and_then(|x| x.as_str()) {
                self.finish_reason = Some(f.to_string());
            }
            let Some(delta) = choice.get("delta") else { continue };
            // vLLM versions emit `reasoning` or `reasoning_content`.
            if let Some(r) = delta
                .get("reasoning")
                .or_else(|| delta.get("reasoning_content"))
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty())
            {
                out.push(RunEvent::ThinkingDelta { delta: r.to_string() });
            }
            if let Some(c) = delta.get("content").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) {
                self.content.push_str(c);
                out.push(RunEvent::TextDelta { delta: c.to_string() });
            }
            if let Some(calls) = delta.get("tool_calls").and_then(|x| x.as_array()) {
                for (ti, tc) in calls.iter().enumerate() {
                    let index = tc
                        .get("index")
                        .and_then(|x| x.as_u64())
                        .unwrap_or((ci * 1000 + ti) as u64);
                    let slot = self.tool_calls.entry(index).or_default();
                    // id/name arrive whole in the first piece; arguments stream.
                    if slot.id.is_empty() {
                        if let Some(id) = tc.get("id").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) {
                            slot.id = id.to_string();
                        }
                    }
                    if let Some(f) = tc.get("function") {
                        if slot.name.is_empty() {
                            if let Some(n) = f.get("name").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) {
                                slot.name = n.to_string();
                            }
                        }
                        if let Some(a) = f.get("arguments").and_then(|x| x.as_str()) {
                            slot.arguments.push_str(a);
                        }
                    }
                }
            }
        }
        out
    }

    /// The assembled tool calls in index order; ids are filled in when the
    /// server sent none (`call_{round}_{n}`). Check this — not `finish_reason`
    /// — to decide whether tools were called (sample Gotcha #8).
    pub fn calls(&self, round: usize) -> Vec<ToolCallAcc> {
        self.tool_calls
            .values()
            .enumerate()
            .map(|(i, c)| ToolCallAcc {
                id: if c.id.is_empty() { format!("call_{round}_{i}") } else { c.id.clone() },
                ..c.clone()
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Chat request + tool loop
// ---------------------------------------------------------------------------

fn system_prompt(tools: bool, roots: &[String], limit: u32) -> String {
    let mut s = String::from(
        "당신은 Samsung SDS Operation Wizard의 업무 보조 AI입니다. 사용자 요청에 정확하고 도움이 되게 답합니다.",
    );
    if tools && !roots.is_empty() {
        s.push_str("\n\n## 파일 도구\n다음 폴더의 파일을 읽기 전용 도구(list_dir, find_files, search_text, outline_file, find_references, read_file)로 조회할 수 있습니다:\n");
        for r in roots {
            s.push_str(&format!("- {r}\n"));
        }
        s.push_str(
            "\n원칙:\n\
             - 코드·문서의 내용은 추측하지 말고, 필요한 파일을 도구로 찾아 읽은 뒤 근거를 들어 답하세요.\n\
             - 상대경로는 첫 번째 폴더(작업 폴더) 기준이며, 없으면 다른 허용 폴더에서 찾습니다. 도구 결과에 나온 경로를 그대로 쓰면 됩니다.\n\
             - 소스 파일은 본문을 읽기 전에 outline_file로 선언부(시그니처·줄 범위)를 먼저 보고, 필요한 범위만 read_file로 읽으세요.\n\
             - 누가 호출하는지·변경 영향 범위는 find_references로 확인하세요(Class.method 형식 권장).\n\
             - 큰 파일은 search_text로 위치를 찾고 read_file의 start_line/max_lines로 필요한 부분만 읽으세요.\n\
             - 도구로는 파일을 만들거나 수정할 수 없습니다. 문서 산출물 저장은 앱이 처리하므로, 문서를 작성하는 단계에서는 최종 답변에 문서 본문만 출력하세요.\n\
             - 충분히 파악했으면 더 읽지 말고 바로 답하세요.",
        );
        s.push_str(&call_budget_rule(limit));
    }
    s
}

/// The call-cost rule appended to the tool principles (D91): every response
/// that calls tools is one API call against the licence quota, so independent
/// lookups must be requested together.
fn call_budget_rule(limit: u32) -> String {
    let quota = if limit == 0 { "호출 수는 비용과 대기 시간으로 이어집니다".to_string() } else { format!("라이선스상 분당 {limit}회까지만 호출할 수 있습니다") };
    format!(
        "\n- 도구를 호출하는 응답 1회가 API 호출 1회로 계산되고 {quota}. 서로 독립적인 조회(여러 파일 읽기·여러 검색어·여러 아웃라인)는 \
         한 응답에서 여러 도구를 동시에 호출해 한 번에 요청하고, 파일을 하나씩 순차로 열지 마세요."
    )
}

/// The `/chat/completions` body. No `max_tokens`: the gateway default (16384)
/// leaves room for reasoning tokens — a small cap yields `content=null` on
/// reasoning models (Gotcha #4). `final_round` keeps the tools declared but
/// forbids calling them, forcing an answer.
fn chat_body(messages: &[Value], tools: bool, final_round: bool) -> Value {
    let mut b = json!({
        "model": BODY_MODEL_PLACEHOLDER,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
        "temperature": TEMPERATURE,
    });
    if tools {
        b["tools"] = fabrix_tools::tool_specs();
        b["tool_choice"] = json!(if final_round { "none" } else { "auto" });
    }
    b
}

/// The assistant turn that requested tools (reasoning excluded; empty content
/// is sent as `null`, as the captured tool-result fixtures do).
fn assistant_tool_message(content: &str, calls: &[ToolCallAcc]) -> Value {
    json!({
        "role": "assistant",
        "content": if content.trim().is_empty() { Value::Null } else { json!(content) },
        "tool_calls": calls.iter().map(|c| json!({
            "id": c.id,
            "type": "function",
            "function": {
                "name": c.name,
                "arguments": if c.arguments.trim().is_empty() { "{}" } else { c.arguments.as_str() },
            },
        })).collect::<Vec<_>>(),
    })
}

/// Tool-call `arguments` → a JSON object (`""` → `{}`).
fn parse_arguments(raw: &str) -> Result<Value, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(json!({}));
    }
    match serde_json::from_str::<Value>(t) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err("arguments가 JSON 객체가 아닙니다".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// "128K" / "1M" / "32000" → tokens.
fn parse_context_tokens(ctx: &str) -> Option<u64> {
    let t = ctx.trim().to_ascii_uppercase();
    let (num, mult) = if let Some(n) = t.strip_suffix('K') {
        (n.to_string(), 1_000f64)
    } else if let Some(n) = t.strip_suffix('M') {
        (n.to_string(), 1_000_000f64)
    } else {
        (t.clone(), 1f64)
    };
    num.trim().parse::<f64>().ok().map(|f| (f * mult) as u64)
}

/// Total tool-output characters allowed for one turn: ~0.8 × the model's
/// context tokens (clamped), or a conservative default when unknown.
fn tool_budget(context: Option<&str>) -> usize {
    context
        .and_then(parse_context_tokens)
        .map(|t| ((t as f64) * 0.8) as usize)
        .map(|b| b.clamp(16_000, 300_000))
        .unwrap_or(DEFAULT_TOOL_BUDGET)
}

/// Spend `content` against the remaining budget, truncating when it runs out.
fn spend_budget(content: String, budget: &mut usize) -> String {
    let n = content.chars().count();
    if n <= *budget {
        *budget -= n;
        return content;
    }
    let kept: String = content.chars().take(*budget).collect();
    *budget = 0;
    format!("{kept}\n…(도구 출력 예산 소진 — 지금까지 읽은 내용으로 답하세요)")
}

enum PostError {
    Canceled,
    Failed(String),
}

/// Everything one chat request needs (bundled to keep `post_round` readable).
struct Turn<'a> {
    client: &'a reqwest::blocking::Client,
    url: &'a str,
    cfg: &'a FabrixConfig,
    model: &'a str,
    plain_system: &'a str,
    cancel: &'a AtomicBool,
    /// Licence quota (calls per minute; 0 = unpaced) — D91.
    limit: u32,
    emit: &'a dyn Fn(RunEvent),
}

/// POST one round. Every attempt first waits for the licence quota (the
/// process-wide gate in `fabrix_rate.rs`, D91). A 429 is waited out and the
/// SAME request re-sent (the conversation so far stays in memory, so no tool
/// round is lost) up to `MAX_RATE_RETRIES` times; on the first round the
/// request is also retried once without tools if the gateway rejects them (400).
fn post_round(
    t: &Turn,
    messages: &mut [Value],
    tools: &mut bool,
    final_round: bool,
    round: usize,
) -> Result<reqwest::blocking::Response, PostError> {
    let mut rate_retries = 0u32;
    let mut retried_tools = false;
    loop {
        let paced = fabrix_rate::acquire(t.limit, t.cancel, &|secs| {
            (t.emit)(RunEvent::Notice { message: pace_notice(t.limit, secs) })
        });
        if !paced {
            return Err(PostError::Canceled);
        }
        let body = chat_body(messages, *tools, final_round);
        let resp = with_auth(t.client.post(t.url), t.cfg)
            .header("x-llm-model-id", t.model)
            .header("Accept", "text/event-stream")
            .json(&body)
            .send()
            .map_err(|e| PostError::Failed(format!("FabriX 요청 실패: {e}")))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());
        let text = resp.text().unwrap_or_default();
        if status.as_u16() == 429 && rate_retries < fabrix_rate::MAX_RATE_RETRIES {
            if let Some(wait) = fabrix_rate::on_rate_limited(retry_after, rate_retries) {
                rate_retries += 1;
                (t.emit)(RunEvent::Notice {
                    message: format!(
                        "FabriX 호출 한도 초과(HTTP 429) — {}초 대기 후 같은 요청을 다시 보냅니다 ({rate_retries}/{}).",
                        fabrix_rate::ceil_secs(wait),
                        fabrix_rate::MAX_RATE_RETRIES
                    ),
                });
                if fabrix_rate::sleep_cancellable(wait, t.cancel) {
                    return Err(PostError::Canceled);
                }
                continue;
            }
        }
        if status.as_u16() == 400 && round == 0 && *tools && !retried_tools && text.to_ascii_lowercase().contains("tool") {
            retried_tools = true;
            *tools = false;
            messages[0] = json!({ "role": "system", "content": t.plain_system });
            continue;
        }
        return Err(PostError::Failed(error_message(status.as_u16(), &text)));
    }
}

/// The chat notice shown while a call waits for the licence quota (D91).
fn pace_notice(limit: u32, secs: u64) -> String {
    if limit == 0 {
        format!("FabriX 호출 한도 초과 이후 대기 중 — {secs}초 후 이어서 진행합니다.")
    } else {
        format!("FabriX 분당 호출 한도({limit}회)에 맞춰 {secs}초 대기 후 이어서 진행합니다.")
    }
}

/// Run one chat turn (all rounds) and return the terminal status. Emits every
/// event except the final `End` (the caller sends it after unregistering).
fn drive(
    cfg: &FabrixConfig,
    model: &str,
    args: &RunArgs,
    emit: &dyn Fn(RunEvent),
    cancel: &AtomicBool,
) -> &'static str {
    let client = match build_client(READ_TIMEOUT) {
        Ok(c) => c,
        Err(e) => {
            emit(RunEvent::Error { message: e });
            return "failed";
        }
    };
    let meta = cfg.models.iter().find(|m| m.id == model);
    let sandbox = Sandbox::new(&args.cwd, &args.extra_dirs);
    let mut tools = args.tools != Some(false) && meta.and_then(|m| m.tools) != Some(false) && !sandbox.is_empty();
    let mut budget = tool_budget(meta.and_then(|m| m.context.as_deref()));
    let url = format!("{}/chat/completions", base_url(cfg));
    let limit = fabrix_rate::limit_of(cfg);
    let plain_system = system_prompt(false, &[], limit);
    let turn = Turn { client: &client, url: &url, cfg, model, plain_system: &plain_system, cancel, limit, emit };

    let mut messages = vec![
        json!({ "role": "system", "content": system_prompt(tools, &sandbox.root_labels(), limit) }),
        json!({ "role": "user", "content": args.prompt }),
    ];

    // Report the requested id — the response `model` is a serving name that
    // differs from it (Gotcha #7). FabriX is sessionless.
    emit(RunEvent::Status { label: "streaming".to_string(), model: Some(model.to_string()), session_id: None });

    let (mut tokens_in, mut tokens_out, mut saw_usage) = (0u64, 0u64, false);
    let (mut any_text, mut any_thinking) = (false, false);
    let mut force_final = false;

    for round in 0..=MAX_ROUNDS {
        if cancel.load(Ordering::Relaxed) {
            return "canceled";
        }
        let final_round = force_final || round == MAX_ROUNDS || budget == 0;
        let resp = match post_round(&turn, &mut messages, &mut tools, final_round, round) {
            Ok(r) => r,
            Err(PostError::Canceled) => return "canceled",
            Err(PostError::Failed(m)) => {
                emit(RunEvent::Error { message: m });
                return "failed";
            }
        };

        // Stream the round, decoding lossily (an invalid UTF-8 byte must not
        // drop the rest of the reply — same as `run::stream_lines`). Rounds are
        // separated by a blank line in both the text and the thinking section.
        let mut acc = StreamAccumulator::default();
        let (mut text_sep, mut think_sep) = (any_text, any_thinking);
        let mut had_error = false;
        let mut reader = BufReader::new(resp);
        let mut buf = Vec::new();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return "canceled"; // dropping `reader` closes the connection
            }
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let Some(data) = line.trim_end_matches(['\r', '\n']).strip_prefix("data:") else { continue };
                    for ev in acc.feed(data) {
                        match &ev {
                            RunEvent::TextDelta { .. } => {
                                if text_sep {
                                    emit(RunEvent::TextDelta { delta: "\n\n".to_string() });
                                    text_sep = false;
                                }
                                any_text = true;
                            }
                            RunEvent::ThinkingDelta { .. } => {
                                if think_sep {
                                    emit(RunEvent::ThinkingDelta { delta: "\n\n".to_string() });
                                    think_sep = false;
                                }
                                any_thinking = true;
                            }
                            RunEvent::Error { .. } => had_error = true,
                            _ => {}
                        }
                        emit(ev);
                    }
                    if acc.done {
                        break;
                    }
                }
                Err(e) => {
                    emit(RunEvent::Error { message: format!("FabriX 스트림 수신 실패: {e}") });
                    return "failed";
                }
            }
        }
        drop(reader);
        if had_error {
            return "failed";
        }
        if let Some((i, o)) = acc.usage {
            saw_usage = true;
            tokens_in += i.unwrap_or(0);
            tokens_out += o.unwrap_or(0);
        }

        let calls = acc.calls(round);
        if calls.is_empty() || final_round {
            if acc.finish_reason.as_deref() == Some("length") {
                emit(RunEvent::Error { message: LENGTH_MSG.to_string() });
                return "failed";
            }
            // `finish_reason:"tool_calls"` with no calls, or an empty reply after
            // tool results (Gotcha #8): ask once more with tools forbidden.
            if calls.is_empty() && acc.content.trim().is_empty() && !final_round && tools {
                force_final = true;
                continue;
            }
            if !any_text {
                emit(RunEvent::Error {
                    message: "FabriX가 빈 응답을 반환했습니다 — 다시 시도하거나 다른 모델을 선택하세요.".to_string(),
                });
                return "failed";
            }
            break;
        }

        // Tool round: record the request, run each call, feed results back.
        messages.push(assistant_tool_message(&acc.content, &calls));
        for call in &calls {
            if cancel.load(Ordering::Relaxed) {
                return "canceled";
            }
            let parsed = parse_arguments(&call.arguments);
            emit(RunEvent::ToolUse {
                id: call.id.clone(),
                name: call.name.clone(),
                input: parsed.clone().unwrap_or_else(|_| json!({ "_raw": call.arguments })),
            });
            let outcome = match parsed {
                Err(e) => ToolOutcome {
                    content: format!("arguments JSON 파싱 실패: {e} — 올바른 JSON 객체로 다시 호출하세요."),
                    is_error: true,
                },
                Ok(_) if budget == 0 => ToolOutcome {
                    content: "도구 출력 예산을 모두 사용했습니다 — 지금까지 읽은 내용으로 답하세요.".to_string(),
                    is_error: true,
                },
                Ok(v) => fabrix_tools::run_tool(&call.name, &v, &sandbox, cancel),
            };
            let content = spend_budget(outcome.content, &mut budget);
            emit(RunEvent::ToolResult {
                tool_use_id: call.id.clone(),
                content: fabrix_tools::clip_chars(&content, UI_PREVIEW_CHARS),
                is_error: outcome.is_error,
            });
            messages.push(json!({ "role": "tool", "tool_call_id": call.id, "content": content }));
        }
    }

    if saw_usage {
        emit(RunEvent::Usage { input_tokens: Some(tokens_in), output_tokens: Some(tokens_out) });
    }
    "succeeded"
}

/// Start a FabriX chat run: load config, pick the model, mint a run id +
/// cancel flag, then drive the tool loop on a worker thread — the HTTP analog
/// of `run::run_agent` (returns the id immediately; one terminal `End`
/// follows). Cancellation is cooperative: polled between SSE lines, rounds and
/// tool calls (and inside directory walks).
pub fn run_fabrix(app: tauri::AppHandle, args: RunArgs, on_event: Channel<RunEvent>) -> Result<String, String> {
    let config_dir = crate::ow_home()?;
    let cfg = settings::load(&config_dir)
        .fabrix
        .filter(|c| !c.endpoint_url.trim().is_empty())
        .ok_or("FabriX 연결 정보가 설정되지 않았습니다. Agents 화면에서 저장하세요.")?;
    let model = resolve_model(args.model.as_deref(), &cfg);

    let registry = app.state::<RunRegistry>();
    let run_id = registry.next_id();
    let canceled = registry.register_remote(&run_id);
    let app2 = app.clone();
    let run_id2 = run_id.clone();

    std::thread::spawn(move || {
        let emit = |ev: RunEvent| {
            let _ = on_event.send(ev);
        };
        let status = drive(&cfg, &model, &args, &emit, &canceled);
        app2.state::<RunRegistry>().unregister(&run_id2);
        emit(RunEvent::End { code: None, status: status.to_string() });
    });

    Ok(run_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(models: Vec<ModelOption>, default: Option<&str>) -> FabrixConfig {
        FabrixConfig {
            endpoint_url: "https://nsds-api.fabrix-s.samsungsds.com/sds/trial/api-llm".into(),
            client: Some("c".into()),
            openapi_token: Some("t".into()),
            allow_invalid_certs: false,
            default_model_id: default.map(str::to_string),
            models,
            calls_per_minute: None,
        }
    }

    fn m(id: &str, label: &str) -> ModelOption {
        ModelOption::new(id, label)
    }

    #[test]
    fn endpoint_normalization_strips_pasted_paths() {
        let e = "https://h/sds/trial/api-llm";
        for raw in [
            "https://h/sds/trial/api-llm",
            "  https://h/sds/trial/api-llm/  ",
            "https://h/sds/trial/api-llm/openapi/llm",
            "https://h/sds/trial/api-llm/openapi/llm/",
            "https://h/sds/trial/api-llm/openapi/llm/v1",
            "https://h/sds/trial/api-llm/openapi/llm/v1/models",
            "https://h/sds/trial/api-llm/openapi/llm/chat/completions",
            "https://h/sds/trial/api-llm/OpenAPI/LLM/v1/chat/completions",
        ] {
            assert_eq!(normalize_endpoint(raw), e, "{raw}");
        }
        let cfg = cfg_with(vec![], None);
        assert_eq!(base_url(&cfg), "https://nsds-api.fabrix-s.samsungsds.com/sds/trial/api-llm/openapi/llm");
    }

    #[test]
    fn token_gets_bearer_prefix_once() {
        assert_eq!(auth_token("eyJ4"), "Bearer eyJ4");
        assert_eq!(auth_token("  Bearer eyJ4 "), "Bearer eyJ4");
        assert_eq!(auth_token("bearer eyJ4"), "bearer eyJ4");
    }

    /// Subset of the captured `mock/fixtures/models_list.json` (real API).
    const MODELS_FIXTURE: &str = r#"[
        {"modelId": 4, "modelGuid": "01992d20-4584-752b-a3aa-4d17612e8df9", "modelServingId": "samsung-v2-3-37b-fp8",
         "name": [{"languageCode": "ko", "content": "Samsung 2.3 37B"}, {"languageCode": "en", "content": "Samsung 2.3 37B"}],
         "description": [{"languageCode": "ko", "content": "문서 작성·요약과 지식 활용을 돕는 업무용 모델 \n[Tools] Knowledge\n[Context] 32K\n[Cutoff] 2024-04-01\n[Speed] Medium\n[Price] High "}]},
        {"modelId": 70, "modelGuid": "x", "modelServingId": "glm5-2-autorouter",
         "name": [{"languageCode": "en", "content": "Glm 5.2"}, {"languageCode": "ko", "content": "Glm 5.2"}],
         "description": [{"languageCode": "ko", "content": "복잡한 업무 분석과 도구 활용을 지원하는 모델\n[Tools] Function Calling, Knowledge\n[Context]\u00a0128K\n[Speed] Fast"}]},
        {"modelId": 13, "modelServingId": "gpt-oss-120b",
         "name": [{"languageCode": "en", "content": "gpt-oss-120b(Low)"}],
         "description": [{"languageCode": "en", "content": "Fast\n[Tools] Function Calling, Knowledge\n[Context] 128K"}]},
        {"name": [{"languageCode": "ko", "content": "id 없음"}]}
    ]"#;

    #[test]
    fn parses_fabrix_model_list() {
        let models = parse_models(MODELS_FIXTURE).unwrap();
        assert_eq!(models.len(), 3, "items without modelId are skipped");
        assert_eq!(models[0].id, "4");
        assert_eq!(models[0].label, "Samsung 2.3 37B");
        assert_eq!(models[0].context.as_deref(), Some("32K"));
        assert_eq!(models[0].tools, Some(false), "Knowledge only → no function calling");
        assert_eq!(models[1].id, "70");
        assert_eq!(models[1].label, "Glm 5.2", "ko preferred");
        assert_eq!(models[1].context.as_deref(), Some("128K"), "nbsp trimmed");
        assert_eq!(models[1].tools, Some(true));
        assert_eq!(models[2].label, "gpt-oss-120b(Low)", "falls back to en/first");

        // OpenAI-style wrapper and string ids are accepted too.
        let wrapped = parse_models(r#"{"data":[{"modelId":"16","name":"gpt-oss-120b(Mid)"}]}"#).unwrap();
        assert_eq!(wrapped[0].id, "16");
        assert_eq!(wrapped[0].label, "gpt-oss-120b(Mid)");
        assert!(parse_models("not json").is_err());
        assert!(parse_models(r#""str""#).is_err());
    }

    #[test]
    fn default_model_goes_first_and_resolves() {
        let models = vec![m("4", "Samsung"), m("16", "gpt-oss"), m("70", "Glm 5.2")];
        let ordered = order_default_first(models.clone(), "70");
        assert_eq!(ordered.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["70", "4", "16"]);
        // Unknown default → order unchanged.
        assert_eq!(order_default_first(models.clone(), "999")[0].id, "4");

        let cfg = cfg_with(models, Some("16"));
        assert_eq!(resolve_model(Some("4"), &cfg), "4", "known id kept");
        assert_eq!(resolve_model(Some("default"), &cfg), "16", "sentinel → configured default");
        assert_eq!(resolve_model(None, &cfg), "16");
        assert_eq!(resolve_model(Some("99"), &cfg), "16", "not in the cache → default");
        assert_eq!(resolve_model(Some("glm-5.1"), &cfg), "16", "old AI Pro id → default");

        // No cache: a numeric id is trusted, anything else → "70".
        let bare = cfg_with(vec![], None);
        assert_eq!(resolve_model(Some("16"), &bare), "16");
        assert_eq!(resolve_model(Some("019f-uuid"), &bare), "70");
        assert_eq!(resolve_model(None, &bare), DEFAULT_MODEL_ID);
    }

    #[test]
    fn legacy_uuid_cache_is_not_current() {
        assert!(cache_is_current(&[m("70", "Glm"), m("16", "gpt")]));
        assert!(!cache_is_current(&[]));
        assert!(!cache_is_current(&[m("019f23a1-46aa-7fa5", "old")]), "pre-D84 cache must be refetched");
    }

    #[test]
    fn detect_uses_current_cache_without_network() {
        // Cache-first (D66): bogus endpoint, so a live fetch would fail.
        let mut cfg = cfg_with(vec![m("4", "Samsung"), m("70", "Glm 5.2")], None);
        cfg.endpoint_url = "http://127.0.0.1:9/never".into();
        let agent = detect_fabrix(Some(cfg), false, None);
        assert!(agent.available);
        assert_eq!(agent.name, "FabriX");
        assert_eq!(agent.source, "remote");
        assert_eq!(agent.models_source, "fallback");
        assert_eq!(agent.models[0].id, "70", "default (70) first");
        assert!(agent.diagnostic.is_none());
    }

    #[test]
    fn detect_reports_not_configured_without_endpoint() {
        let agent = detect_fabrix(None, false, None);
        assert!(!agent.available);
        assert!(agent.models.is_empty());
        assert_eq!(agent.diagnostic.as_deref(), Some("not-configured"));
    }

    #[test]
    fn normalizes_the_three_error_formats() {
        // WSO2 gateway — bad x-openapi-token.
        let e = error_message(401, r#"{"code": "900901", "message": "Invalid Credentials", "description": "Access failure for API: /sds/trial/api-llm/v1"}"#);
        assert!(e.starts_with("FabriX HTTP 401 — Invalid Credentials: Access failure"), "{e}");
        assert!(e.contains("x-openapi-token"));
        // OIDC — bad/expired x-fabrix-client.
        let e = error_message(401, r#"{"statusCode": 401, "code": "OIDC-TOKEN-17", "error": "Unauthorized", "message": "Unauthorized"}"#);
        assert!(e.contains("Unauthorized (OIDC-TOKEN-17)") && e.contains("재발급"), "{e}");
        // litellm — missing x-llm-model-id.
        let e = error_message(401, r#"{"error": {"message": "Authentication Error, Invalid proxy server token passed.", "type": "auth_error", "param": "key", "code": "401"}}"#);
        assert!(e.contains("Invalid proxy server token") && e.contains("x-llm-model-id"), "{e}");
        // litellm — unknown model id.
        let e = error_message(404, r#"{"error": {"message": "FabriX model could not be resolved from the provided model headers.", "type": "auth_error", "param": "None", "code": "404"}}"#);
        assert!(e.starts_with("FabriX HTTP 404") && e.contains("존재하지 않는 모델"), "{e}");
        // WSO2 throttling.
        let e = error_message(429, r#"{"code": "900804", "message": "Message throttled out", "description": "You have exceeded your quota"}"#);
        assert!(e.contains("Message throttled out") && e.contains("호출 한도"), "{e}");
        // WSO2 405 (wrong models path).
        let e = error_message(405, r#"{"code": "405", "type": "Status report", "message": "Runtime Error", "description": "Method not allowed for given API resource"}"#);
        assert!(e.contains("Method not allowed") && e.contains("/openapi/llm"), "{e}");
        // Plain text / empty bodies.
        assert_eq!(error_message(502, "Bad Gateway"), "FabriX HTTP 502 — Bad Gateway");
        assert_eq!(error_message(500, ""), "FabriX HTTP 500 — HTTP 500");
    }

    /// Captured `chat_tool_call_stream_70` lines (real API, abridged).
    const TOOL_STREAM: &[&str] = &[
        r#"{"id":"chatcmpl-a7e2","object":"chat.completion.chunk","created":1791213770,"model":"glm-5-2-awq","choices":[{"index":0,"delta":{"role":"assistant","content":""},"logprobs":null,"finish_reason":null}],"prompt_token_ids":null,"prompt_text":null}"#,
        r#"{"id":"chatcmpl-a7e2","object":"chat.completion.chunk","created":1791213770,"model":"glm-5-2-awq","choices":[{"index":0,"delta":{"reasoning":"The"},"logprobs":null,"finish_reason":null,"token_ids":null}]}"#,
        r#"{"id":"chatcmpl-a7e2","object":"chat.completion.chunk","created":1791213770,"model":"glm-5-2-awq","choices":[{"index":0,"delta":{"content":"네, 도"},"logprobs":null,"finish_reason":null,"token_ids":null}]}"#,
        r#"{"id":"chatcmpl-a7e2","object":"chat.completion.chunk","created":1791213770,"model":"glm-5-2-awq","choices":[{"index":0,"delta":{"content":"구를 사용해서"},"logprobs":null,"finish_reason":null,"token_ids":null}]}"#,
        r#"{"id":"chatcmpl-a7e2","object":"chat.completion.chunk","created":1791213770,"model":"glm-5-2-awq","choices":[{"index":0,"delta":{"tool_calls":[{"id":"chatcmpl-tool-926607a570265ad3","type":"function","index":0,"function":{"name":"add","arguments":"{\"a\": "}}]},"logprobs":null,"finish_reason":null,"token_ids":null}]}"#,
        r#"{"id":"chatcmpl-a7e2","object":"chat.completion.chunk","created":1791213770,"model":"glm-5-2-awq","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1, \"b\": "}}]},"logprobs":null,"finish_reason":null,"token_ids":null}]}"#,
        r#"{"id":"chatcmpl-a7e2","object":"chat.completion.chunk","created":1791213770,"model":"glm-5-2-awq","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"3}"}}]},"logprobs":null,"finish_reason":"tool_calls","stop_reason":154829,"token_ids":null}]}"#,
        r#"{"id":"chatcmpl-a7e2","object":"chat.completion.chunk","created":1791213770,"model":"glm-5-2-awq","choices":[],"usage":{"prompt_tokens":181,"total_tokens":233,"completion_tokens":52},"system_fingerprint":"vllm-0.23.1rc1"}"#,
        "[DONE]",
    ];

    #[test]
    fn accumulates_streamed_tool_calls() {
        let mut acc = StreamAccumulator::default();
        let mut events = Vec::new();
        for line in TOOL_STREAM {
            events.extend(acc.feed(line));
        }
        assert!(acc.done);
        assert_eq!(acc.content, "네, 도구를 사용해서");
        assert_eq!(acc.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(acc.usage, Some((Some(181), Some(52))));
        let calls = acc.calls(0);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "chatcmpl-tool-926607a570265ad3", "id from the first piece, not concatenated");
        assert_eq!(calls[0].name, "add");
        assert_eq!(calls[0].arguments, r#"{"a": 1, "b": 3}"#);
        assert_eq!(parse_arguments(&calls[0].arguments).unwrap(), json!({ "a": 1, "b": 3 }));
        assert_eq!(
            events,
            vec![
                RunEvent::ThinkingDelta { delta: "The".into() },
                RunEvent::TextDelta { delta: "네, 도".into() },
                RunEvent::TextDelta { delta: "구를 사용해서".into() },
            ],
            "role-only opener and the empty-choices usage chunk emit nothing"
        );
    }

    #[test]
    fn accumulates_plain_reasoning_stream() {
        // Captured `chat_stream_70_include_usage` shape: reasoning, then content.
        let mut acc = StreamAccumulator::default();
        let mut events = Vec::new();
        for line in [
            r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"reasoning_content":"1. Analyze"},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"Hi there!"},"finish_reason":"stop","stop_reason":154827}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":19,"total_tokens":165,"completion_tokens":146}}"#,
            "[DONE]",
        ] {
            events.extend(acc.feed(line));
        }
        assert!(acc.calls(0).is_empty());
        assert_eq!(acc.finish_reason.as_deref(), Some("stop"));
        assert_eq!(acc.usage, Some((Some(19), Some(146))));
        assert_eq!(
            events,
            vec![
                RunEvent::ThinkingDelta { delta: "1. Analyze".into() },
                RunEvent::TextDelta { delta: "Hi there!".into() },
            ]
        );
    }

    #[test]
    fn stream_error_chunk_becomes_error_and_ids_are_generated() {
        let mut acc = StreamAccumulator::default();
        let evs = acc.feed(r#"{"error":{"message":"litellm.InternalServerError: boom","type":"None","code":"500"}}"#);
        assert!(matches!(&evs[0], RunEvent::Error { message } if message.starts_with("FabriX HTTP 500 — litellm.InternalServerError")));

        // Tool call pieces without ids get stable generated ones.
        let mut acc = StreamAccumulator::default();
        acc.feed(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"list_dir","arguments":""}},{"index":1,"function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}}]}}]}"#);
        let calls = acc.calls(3);
        assert_eq!(calls[0].id, "call_3_0");
        assert_eq!(calls[1].id, "call_3_1");
        assert_eq!(parse_arguments(&calls[0].arguments).unwrap(), json!({}), "empty arguments → {{}}");
        assert!(parse_arguments("[1]").is_err());
        assert!(parse_arguments("{bad").is_err());

        // The follow-up assistant message: null content, "{}" arguments.
        let msg = assistant_tool_message("  ", &calls);
        assert!(msg["content"].is_null());
        assert_eq!(msg["tool_calls"][0]["function"]["arguments"], "{}");
        assert_eq!(msg["tool_calls"][1]["type"], "function");
    }

    #[test]
    fn chat_body_follows_the_fabrix_contract() {
        let msgs = vec![json!({"role":"user","content":"hi"})];
        let b = chat_body(&msgs, false, false);
        assert_eq!(b["model"], "/mnt/models");
        assert_eq!(b["stream"], true);
        assert_eq!(b["stream_options"]["include_usage"], true);
        assert!(b.get("max_tokens").is_none(), "gateway default leaves room for reasoning");
        assert!(b.get("tools").is_none() && b.get("tool_choice").is_none());

        let t = chat_body(&msgs, true, false);
        assert_eq!(t["tool_choice"], "auto");
        assert_eq!(t["tools"].as_array().unwrap().len(), 6);
        assert_eq!(chat_body(&msgs, true, true)["tool_choice"], "none", "final round forbids calls");
    }

    #[test]
    fn tool_budget_follows_context_size() {
        assert_eq!(parse_context_tokens("128K"), Some(128_000));
        assert_eq!(parse_context_tokens("1M"), Some(1_000_000));
        assert_eq!(parse_context_tokens("32000"), Some(32_000));
        assert_eq!(parse_context_tokens("-"), None);
        assert_eq!(tool_budget(Some("128K")), 102_400);
        assert_eq!(tool_budget(Some("400K")), 300_000, "clamped");
        assert_eq!(tool_budget(Some("8K")), 16_000, "clamped");
        assert_eq!(tool_budget(None), DEFAULT_TOOL_BUDGET);

        let mut budget = 5;
        assert_eq!(spend_budget("abc".into(), &mut budget), "abc");
        assert_eq!(budget, 2);
        let cut = spend_budget("가나다".into(), &mut budget);
        assert!(cut.starts_with("가나\n") && cut.contains("예산 소진"));
        assert_eq!(budget, 0);
    }

    /// End-to-end against the FabrixSample mock server (replays captured real
    /// responses, validates headers/paths like the real gateway):
    ///   `Desktop\FabrixSample> .venv\Scripts\python -m mock.server --port 8765`
    ///   `cargo test -- --ignored fabrix_e2e_mock`
    /// Exercises the real HTTP path: auth headers (+ Bearer auto-prefix),
    /// `/v1/models`, SSE streaming, the tool loop (the mock answers a tools
    /// request with a `list_dir` call, then a final answer to the tool result)
    /// and error normalization.
    #[test]
    #[ignore = "needs the FabrixSample mock server on 127.0.0.1:8765"]
    fn fabrix_e2e_mock() {
        use std::cell::RefCell;

        let endpoint = std::env::var("OW_FABRIX_MOCK")
            .unwrap_or_else(|_| "http://127.0.0.1:8765/sds/trial/api-llm".into());
        let mut cfg = FabrixConfig {
            endpoint_url: format!("{endpoint}/openapi/llm"), // pasted base → normalized
            client: Some("mock-client-key".into()),
            openapi_token: Some("mock-openapi-token".into()), // no "Bearer " → auto-added
            ..Default::default()
        };
        let models = fetch_models(&cfg).expect("model list");
        assert_eq!(models.len(), 8);
        assert!(models.iter().any(|m| m.id == "70" && m.tools == Some(true) && m.context.as_deref() == Some("128K")));
        assert!(models.iter().any(|m| m.id == "4" && m.tools == Some(false)));
        cfg.models = models;

        let dir = std::env::temp_dir().join(format!("ow-fx-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("README.md"), "# demo\n").unwrap();
        let args = |tools: Option<bool>, model: &str| RunArgs {
            agent_id: "fabrix".into(),
            prompt: "작업 폴더를 확인하고 요약해줘".into(),
            cwd: dir.to_string_lossy().into_owned(),
            model: Some(model.into()),
            session_id: None,
            resume: None,
            extra_dirs: vec![],
            tools,
        };
        let run = |a: &RunArgs| {
            let events = RefCell::new(Vec::new());
            let status = drive(&cfg, a.model.as_deref().unwrap(), a, &|e| events.borrow_mut().push(e), &AtomicBool::new(false));
            (status, events.into_inner())
        };

        // Tool loop: list_dir round → final answer.
        let (status, events) = run(&args(None, "70"));
        assert_eq!(status, "succeeded", "{events:?}");
        assert!(events.iter().any(|e| matches!(e, RunEvent::ToolUse { name, .. } if name == "list_dir")), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, RunEvent::ToolResult { is_error: false, content, .. } if content.contains("README.md"))), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, RunEvent::TextDelta { .. })));
        assert!(events.iter().any(|e| matches!(e, RunEvent::Usage { .. })));
        assert!(matches!(&events[0], RunEvent::Status { model: Some(m), .. } if m == "70"));

        // Text-only side turn: no tools → a single streamed answer.
        let (status, events) = run(&args(Some(false), "70"));
        assert_eq!(status, "succeeded", "{events:?}");
        assert!(!events.iter().any(|e| matches!(e, RunEvent::ToolUse { .. })));
        assert!(events.iter().any(|e| matches!(e, RunEvent::ThinkingDelta { .. })), "reasoning streamed");

        // Unknown model id (bypassing resolve_model) → normalized 404 + hint.
        let (status, events) = run(&args(Some(false), "99999"));
        assert_eq!(status, "failed");
        assert!(events.iter().any(|e| matches!(e, RunEvent::Error { message } if message.contains("HTTP 404") && message.contains("존재하지 않는 모델"))), "{events:?}");

        // Bad token → WSO2 401 + token hint.
        let bad = FabrixConfig { openapi_token: Some("wrong".into()), ..cfg.clone() };
        let err = fetch_models(&bad).unwrap_err();
        assert!(err.contains("HTTP 401") && err.contains("x-openapi-token"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn system_prompt_lists_roots_only_with_tools() {
        let roots = vec![r"C:\work".to_string(), r"D:\code".to_string()];
        let with = system_prompt(true, &roots, 10);
        assert!(with.contains(r"- C:\work") && with.contains("read_file") && with.contains("outline_file") && with.contains("find_references") && with.contains("앱이 처리"));
        let without = system_prompt(false, &roots, 10);
        assert!(!without.contains("read_file"));
    }
}

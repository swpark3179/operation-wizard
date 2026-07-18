//! Minimal MCP (Model Context Protocol) client over the **streamable HTTP**
//! transport (JSON-RPC 2.0) — D82.
//!
//! Used to reach the official in-house Confluence MCP server
//! (`https://sdsdev.co.kr/mcp-confluence/mcp`) after the REST/WebView approach
//! kept failing (403 WAF — D75/D77). Auth travels as the `x-auth` request
//! header (from `settings.confluence.auth_key`).
//!
//! Transport shape (streamable HTTP): every message is an HTTP POST whose body
//! is one JSON-RPC message. The server answers either with a single
//! `application/json` response or a `text/event-stream` SSE body carrying the
//! response frame(s). This client reads the full response body and parses both
//! shapes — MCP tool responses are bounded messages (not long token streams),
//! so no incremental reader is needed; the client's total timeout bounds a hung
//! stream.
//!
//! Session strategy: one handshake per operation. `connect()` does
//! `initialize` → capture `Mcp-Session-Id` → `notifications/initialized` →
//! `tools/list`, and the returned `McpSession` reuses that session for every
//! `tools/call` in the operation, then is dropped. This matches the app's
//! stateless-command idiom and sidesteps cross-command session-expiry bugs (a
//! mid-operation 404 triggers one automatic re-handshake).
//!
//! The HTTP client reuses the `reqwest` blocking + native-tls (schannel) recipe
//! from `fabrix.rs`/`aipro.rs` — no new crate, OS cert store trusted, env proxy
//! bypassed (D66). A `User-Agent` is sent because the `sdsdev.co.kr` gateway
//! gates on it (the same class of failure as AI Pro D74 / Confluence REST D75).

use std::time::Duration;

use serde_json::{json, Value};

/// Connect timeout for the MCP endpoint.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Total per-request timeout. Generous (D53) — a slow corporate backend plus a
/// bounded MCP response; also caps a server that never closes the SSE stream.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// MCP protocol version we advertise (updated from the server's initialize
/// result when it replies with its own).
const PROTOCOL_VERSION: &str = "2025-06-18";
/// The `sdsdev.co.kr` gateway allowlists the opencode client by User-Agent
/// (AI Pro needed the same, D74; `reqwest` sends none by default). If the MCP
/// gateway rejects it (406/403), adjust this constant.
const MCP_UA: &str = "opencode/0.1.0";

/// One tool advertised by the server (`tools/list`).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's arguments (used to infer argument names).
    pub input_schema: Value,
}

/// A live MCP session (post-handshake). Not `Clone` — one session per operation.
pub struct McpSession {
    http: reqwest::blocking::Client,
    url: String,
    auth_key: Option<String>,
    session_id: Option<String>,
    protocol_version: String,
    initialized: bool,
    next_id: u64,
    /// Tools captured at connect — argument-name inference reads their schemas.
    pub tools: Vec<ToolInfo>,
}

/// Build the blocking client (fabrix/aipro recipe: no env proxy, connect+total
/// timeout, mandatory UA, OS cert store via native-tls).
fn build_client(allow_invalid_certs: bool) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .user_agent(MCP_UA)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(CALL_TIMEOUT)
        .danger_accept_invalid_certs(allow_invalid_certs)
        .build()
        .map_err(|e| e.to_string())
}

impl McpSession {
    /// Handshake with the server and list its tools. Blocking — call on a worker
    /// thread. `allow_invalid_certs` is currently always false (TLS on, D75).
    pub fn connect(
        url: &str,
        auth_key: Option<&str>,
        allow_invalid_certs: bool,
    ) -> Result<Self, String> {
        let url = url.trim().to_string();
        if url.is_empty() {
            return Err("MCP URL이 비어 있습니다 — 지식 화면에서 등록해 주세요".to_string());
        }
        let mut s = McpSession {
            http: build_client(allow_invalid_certs)?,
            url,
            auth_key: auth_key
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_string),
            session_id: None,
            protocol_version: PROTOCOL_VERSION.to_string(),
            initialized: false,
            next_id: 0,
            tools: Vec::new(),
        };
        s.initialize()?;
        s.tools = s.list_tools()?;
        Ok(s)
    }

    /// `tools/list` → the server's tools.
    pub fn list_tools(&mut self) -> Result<Vec<ToolInfo>, String> {
        let result = self.request("tools/list", json!({}))?;
        Ok(parse_tools(&result))
    }

    /// `tools/call` → the concatenated text of the result `content[]`
    /// (`structuredContent` as a fallback). Errors on a JSON-RPC error or an
    /// `isError: true` tool result.
    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<String, String> {
        let result = self.request("tools/call", json!({ "name": name, "arguments": arguments }))?;
        tool_result_text(&result)
    }

    // ── transport ──────────────────────────────────────────────────────────

    fn initialize(&mut self) -> Result<(), String> {
        self.next_id += 1;
        let id = self.next_id;
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "operation-wizard", "version": env!("CARGO_PKG_VERSION") }
            }
        });
        let result = self.do_post(body, id, true)?;
        if let Some(pv) = result.get("protocolVersion").and_then(|x| x.as_str()) {
            if !pv.trim().is_empty() {
                self.protocol_version = pv.trim().to_string();
            }
        }
        self.initialized = true;
        // The server acknowledges initialization; then we may call tools.
        self.notify("notifications/initialized", Value::Null)?;
        Ok(())
    }

    /// One JSON-RPC request with an automatic single re-handshake if the session
    /// expired (HTTP 404 after we were initialized).
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match self.rpc(method, params.clone()) {
            Err(e) if self.initialized && e.contains("HTTP 404") => {
                self.session_id = None;
                self.initialized = false;
                self.initialize()?;
                self.rpc(method, params)
            }
            other => other,
        }
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let mut body = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if !params.is_null() {
            body["params"] = params;
        }
        self.do_post(body, id, false)
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let mut body = json!({ "jsonrpc": "2.0", "method": method });
        if !params.is_null() {
            body["params"] = params;
        }
        let resp = self.send(&body)?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let text = resp.text().unwrap_or_default();
            Err(format!("HTTP {status} — {}", snippet(&text)))
        }
    }

    fn send(&self, body: &Value) -> Result<reqwest::blocking::Response, String> {
        let mut req = self
            .http
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .json(body);
        if let Some(k) = self.auth_key.as_deref() {
            req = req.header("x-auth", k);
        }
        if let Some(sid) = self.session_id.as_deref() {
            req = req.header("Mcp-Session-Id", sid);
        }
        // Spec: the protocol-version header is sent after initialization.
        if self.initialized {
            req = req.header("MCP-Protocol-Version", &self.protocol_version);
        }
        req.send().map_err(|e| format!("MCP 요청 실패: {e}"))
    }

    /// POST a request-bearing message and return its JSON-RPC `result`. Handles
    /// both `application/json` and `text/event-stream` responses.
    fn do_post(&mut self, body: Value, want_id: u64, capture_session: bool) -> Result<Value, String> {
        let resp = self.send(&body)?;
        if capture_session {
            if let Some(sid) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
                let sid = sid.trim();
                if !sid.is_empty() {
                    self.session_id = Some(sid.to_string());
                }
            }
        }
        let status = resp.status();
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let text = resp.text().map_err(|e| format!("MCP 응답 본문 읽기 실패: {e}"))?;
        if !status.is_success() {
            return Err(format!("HTTP {status} — {}", snippet(&text)));
        }
        if ct.contains("text/event-stream") || (!ct.contains("application/json") && looks_like_sse(&text)) {
            parse_sse_string(&text, want_id)
        } else {
            parse_jsonrpc_body(&text, want_id)
        }
    }
}

// ── pure parsers (unit-tested) ─────────────────────────────────────────────

fn snippet(body: &str) -> String {
    let s: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    s.chars().take(300).collect()
}

fn looks_like_sse(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("data:") || t.starts_with("event:") || t.starts_with(':')
}

/// Extract a JSON-RPC `result` (or map an `error`) from one message object, or a
/// batch array (pick the message matching `want_id`).
fn extract_result(v: &Value, want_id: u64) -> Result<Value, String> {
    if let Some(arr) = v.as_array() {
        for m in arr {
            if m.get("id").and_then(|x| x.as_u64()) == Some(want_id) {
                return one_result(m);
            }
        }
        return Err("MCP 응답에서 요청 id를 찾지 못했습니다".to_string());
    }
    one_result(v)
}

fn one_result(m: &Value) -> Result<Value, String> {
    if let Some(err) = m.get("error") {
        if !err.is_null() {
            return Err(jsonrpc_error_msg(err));
        }
    }
    m.get("result")
        .cloned()
        .ok_or_else(|| "MCP 응답에 result가 없습니다".to_string())
}

/// Parse a plain `application/json` JSON-RPC body.
pub fn parse_jsonrpc_body(body: &str, want_id: u64) -> Result<Value, String> {
    let v: Value =
        serde_json::from_str(body.trim()).map_err(|e| format!("MCP 응답 JSON 파싱 실패: {e}"))?;
    extract_result(&v, want_id)
}

/// Parse an SSE body: accumulate `data:` lines per event (until a blank line),
/// return the first event that is our JSON-RPC response (`id == want_id`).
/// Server-initiated notifications (no id / other id) and comment lines (`:`) are
/// ignored.
pub fn parse_sse_string(text: &str, want_id: u64) -> Result<Value, String> {
    let mut data_lines: Vec<String> = Vec::new();
    let flush = |lines: &mut Vec<String>| -> Option<Result<Value, String>> {
        if lines.is_empty() {
            return None;
        }
        let data = lines.join("\n");
        lines.clear();
        sse_event_to_result(&data, want_id)
    };
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            if let Some(res) = flush(&mut data_lines) {
                return res;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
        // `event:` / `id:` / `retry:` / `:` comment lines carry no payload.
    }
    if let Some(res) = flush(&mut data_lines) {
        return res;
    }
    Err("MCP SSE 응답에서 결과를 찾지 못했습니다".to_string())
}

/// One assembled SSE event's `data` → our JSON-RPC result, or `None` if it is
/// not our response (a notification, another id, or unparseable).
pub fn sse_event_to_result(data: &str, want_id: u64) -> Option<Result<Value, String>> {
    let data = data.trim();
    if data.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(data).ok()?;
    if v.get("id").and_then(|x| x.as_u64()) != Some(want_id) {
        return None; // a server notification or an unrelated id
    }
    Some(one_result(&v))
}

fn jsonrpc_error_msg(err: &Value) -> String {
    let msg = err.get("message").and_then(|x| x.as_str()).unwrap_or("알 수 없는 오류");
    match err.get("code").and_then(|x| x.as_i64()) {
        Some(c) => format!("MCP 오류 {c}: {msg}"),
        None => format!("MCP 오류: {msg}"),
    }
}

/// `tools/list` result → tool infos.
pub fn parse_tools(result: &Value) -> Vec<ToolInfo> {
    result
        .get("tools")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let name = t.get("name").and_then(|x| x.as_str())?.to_string();
                    let description =
                        t.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let input_schema = t.get("inputSchema").cloned().unwrap_or(Value::Null);
                    Some(ToolInfo { name, description, input_schema })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A `tools/call` result → text. Prefers `content[]` text parts; falls back to a
/// stringified `structuredContent`. `isError: true` maps to `Err`.
pub fn tool_result_text(result: &Value) -> Result<String, String> {
    let is_error = result.get("isError").and_then(|x| x.as_bool()).unwrap_or(false);
    let mut parts: Vec<String> = Vec::new();
    if let Some(arr) = result.get("content").and_then(|x| x.as_array()) {
        for c in arr {
            if c.get("type").and_then(|x| x.as_str()) == Some("text") {
                if let Some(t) = c.get("text").and_then(|x| x.as_str()) {
                    parts.push(t.to_string());
                }
            }
        }
    }
    let mut text = parts.join("\n");
    if text.trim().is_empty() {
        if let Some(sc) = result.get("structuredContent") {
            if !sc.is_null() {
                text = sc.to_string();
            }
        }
    }
    if is_error {
        return Err(if text.trim().is_empty() {
            "MCP 도구 오류".to_string()
        } else {
            text
        });
    }
    Ok(text)
}

/// Pick the argument key for a tool from its input schema: the first `required`
/// property, else the first declared property, else `fallback`. Lets us call
/// `getPageById`/`getChild`/`searchContent` without hard-coding the exact
/// argument name (which we don't know until `tools/list`).
pub fn arg_key_for(tool: &ToolInfo, fallback: &str) -> String {
    let schema = &tool.input_schema;
    if let Some(req) = schema.get("required").and_then(|x| x.as_array()) {
        for r in req {
            if let Some(name) = r.as_str() {
                if !name.trim().is_empty() {
                    return name.to_string();
                }
            }
        }
    }
    if let Some(props) = schema.get("properties").and_then(|x| x.as_object()) {
        if let Some(k) = props.keys().next() {
            return k.clone();
        }
    }
    fallback.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_result_and_error() {
        let ok = parse_jsonrpc_body(r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#, 2).unwrap();
        assert!(ok.get("tools").is_some());

        let err = parse_jsonrpc_body(
            r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"Method not found"}}"#,
            3,
        );
        assert!(err.unwrap_err().contains("Method not found"));

        assert!(parse_jsonrpc_body("not json", 1).is_err());
        // Batch: pick the matching id.
        let batch = parse_jsonrpc_body(
            r#"[{"jsonrpc":"2.0","id":9,"result":{"a":1}},{"jsonrpc":"2.0","id":5,"result":{"b":2}}]"#,
            5,
        )
        .unwrap();
        assert_eq!(batch.get("b").and_then(|x| x.as_i64()), Some(2));
    }

    #[test]
    fn parses_sse_response_ignoring_noise() {
        // A keep-alive comment, then a server notification (no id), then our
        // response frame with the matching id.
        let body = ": ping\n\n\
            data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{}}\n\n\
            data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";
        let res = parse_sse_string(body, 7).unwrap();
        assert_eq!(res.get("ok").and_then(|x| x.as_bool()), Some(true));

        // Multi-line data joined; id mismatch → not found.
        assert!(parse_sse_string("data: {\"id\":1,\"result\":{}}\n\n", 2).is_err());
    }

    #[test]
    fn tool_result_text_joins_and_flags_errors() {
        let ok = json!({ "content": [ {"type":"text","text":"line1"}, {"type":"text","text":"line2"} ] });
        assert_eq!(tool_result_text(&ok).unwrap(), "line1\nline2");

        let structured = json!({ "content": [], "structuredContent": {"id":"1"} });
        assert!(tool_result_text(&structured).unwrap().contains("\"id\""));

        let err = json!({ "isError": true, "content": [ {"type":"text","text":"boom"} ] });
        assert_eq!(tool_result_text(&err).unwrap_err(), "boom");
    }

    #[test]
    fn parse_tools_reads_name_and_schema() {
        let result = json!({ "tools": [
            { "name": "getPageById", "description": "get a page", "inputSchema": {"type":"object","properties":{"id":{"type":"string"}},"required":["id"]} },
            { "description": "no name — skipped" }
        ]});
        let tools = parse_tools(&result);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "getPageById");
        assert_eq!(arg_key_for(&tools[0], "fallback"), "id");
    }

    #[test]
    fn arg_key_for_falls_back() {
        let no_required = ToolInfo {
            name: "t".into(),
            description: String::new(),
            input_schema: json!({ "properties": { "query": {} } }),
        };
        assert_eq!(arg_key_for(&no_required, "x"), "query");

        let empty = ToolInfo { name: "t".into(), description: String::new(), input_schema: Value::Null };
        assert_eq!(arg_key_for(&empty, "cql"), "cql");
    }
}

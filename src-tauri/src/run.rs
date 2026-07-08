//! Agent run engine: spawn a resolved CLI agent, stream its output to the
//! webview via a Tauri `Channel`.
//!
//! Ports the run/stream half of Open Design's daemon
//! (`apps/daemon/src/runtimes/runs.ts` + `defs/claude.ts` + `claude-stream.ts`),
//! replacing HTTP + Server-Sent Events with `tauri::ipc::Channel`. There is no
//! long-lived HTTP server: `run_agent` spawns the child on a worker thread and
//! pushes normalized `RunEvent`s down the channel as stdout lines arrive.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::Manager;

use crate::agents::{self, PromptFormat, RunCtx, StreamFormat};
use crate::resolve::resolve_agent;
use crate::settings;

/// Normalized run event streamed to the webview. Ports the subset of Open
/// Design's `DaemonAgentPayload` this app renders. Serialized as
/// `{ "type": "...", ...camelCase fields }`.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RunEvent {
    /// Lifecycle status; carries the model + captured session id on init.
    Status {
        label: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    /// A chunk of assistant text to append.
    TextDelta { delta: String },
    /// A chunk of assistant reasoning to append.
    ThinkingDelta { delta: String },
    /// A tool invocation by the agent.
    ToolUse { id: String, name: String, input: Value },
    /// The result returned to a prior tool invocation.
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    /// Token usage for the turn.
    Usage {
        #[serde(skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u64>,
    },
    /// Raw stdout line (plain-format agents).
    Stdout { chunk: String },
    /// A failure message (spawn error, non-zero exit, etc.).
    Error { message: String },
    /// Terminal event: the child exited (or was canceled).
    End {
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<i32>,
        /// "succeeded" | "failed" | "canceled".
        status: String,
    },
}

/// Arguments for one run (a single agent turn). Deserialized from the webview.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunArgs {
    pub agent_id: String,
    pub prompt: String,
    pub cwd: String,
    #[serde(default)]
    pub model: Option<String>,
    /// Session id to carry across turns: claude uses a client-minted UUID, codex
    /// the captured `thread_id`. `None` = new/sessionless.
    #[serde(default)]
    pub session_id: Option<String>,
    /// True when continuing a prior turn (resume) rather than starting fresh.
    #[serde(default)]
    pub resume: Option<bool>,
}

struct RunHandle {
    child: Arc<Mutex<Child>>,
    canceled: Arc<AtomicBool>,
}

/// In-memory registry of active runs, keyed by run id. Managed Tauri state.
#[derive(Default)]
pub struct RunRegistry {
    counter: AtomicU64,
    runs: Mutex<HashMap<String, RunHandle>>,
}

/// Turn a tool-result `content` (string, or array of `{type:text,text}`) into a
/// single display string.
fn stringify_tool_content(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(|x| x.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Parse one line of Claude Code's `--output-format stream-json` into zero or
/// more `RunEvent`s. Port of `claude-stream.ts` (message-level; we do not pass
/// `--include-partial-messages`, so each `assistant` message arrives complete).
pub fn parse_claude_stream_line(line: &str) -> Vec<RunEvent> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
        "system" => {
            if v.get("subtype").and_then(|x| x.as_str()) == Some("init") {
                return vec![RunEvent::Status {
                    label: "initializing".to_string(),
                    model: v.get("model").and_then(|x| x.as_str()).map(String::from),
                    session_id: v
                        .get("session_id")
                        .and_then(|x| x.as_str())
                        .map(String::from),
                }];
            }
            vec![]
        }
        "assistant" => {
            let mut out = vec![];
            if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                for block in content {
                    match block.get("type").and_then(|x| x.as_str()) {
                        Some("text") => {
                            if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                                if !t.is_empty() {
                                    out.push(RunEvent::TextDelta { delta: t.to_string() });
                                }
                            }
                        }
                        Some("thinking") => {
                            if let Some(t) = block.get("thinking").and_then(|x| x.as_str()) {
                                if !t.is_empty() {
                                    out.push(RunEvent::ThinkingDelta { delta: t.to_string() });
                                }
                            }
                        }
                        Some("tool_use") => out.push(RunEvent::ToolUse {
                            id: block.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            name: block
                                .get("name")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string(),
                            input: block.get("input").cloned().unwrap_or(Value::Null),
                        }),
                        _ => {}
                    }
                }
            }
            out
        }
        "user" => {
            let mut out = vec![];
            if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                for block in content {
                    if block.get("type").and_then(|x| x.as_str()) == Some("tool_result") {
                        out.push(RunEvent::ToolResult {
                            tool_use_id: block
                                .get("tool_use_id")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string(),
                            content: stringify_tool_content(block.get("content")),
                            is_error: block.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false),
                        });
                    }
                }
            }
            out
        }
        "result" => {
            let usage = v.get("usage");
            vec![RunEvent::Usage {
                input_tokens: usage.and_then(|u| u.get("input_tokens")).and_then(|x| x.as_u64()),
                output_tokens: usage.and_then(|u| u.get("output_tokens")).and_then(|x| x.as_u64()),
            }]
        }
        _ => vec![],
    }
}

/// Extract a human message from an error value (a string, or an object with a
/// `message` field), falling back to `default`.
fn extract_msg(v: Option<&Value>, default: &str) -> String {
    match v {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(Value::Object(o)) => o
            .get("message")
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(String::from)
            .unwrap_or_else(|| default.to_string()),
        _ => default.to_string(),
    }
}

/// Parse one line of Codex's `exec --json` event stream (JSONL) into `RunEvent`s.
/// Port of `handleCodexEvent` (`json-event-stream.ts`). `thread.started` carries
/// codex's own session id (captured for `exec resume`); `agent_message` items are
/// assistant text; `command_execution` items are Bash tool calls/results.
pub fn parse_codex_event_line(line: &str) -> Vec<RunEvent> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
        "thread.started" => vec![RunEvent::Status {
            label: "initializing".to_string(),
            model: None,
            session_id: v
                .get("thread_id")
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from),
        }],
        "turn.started" => vec![RunEvent::Status {
            label: "thinking".to_string(),
            model: None,
            session_id: None,
        }],
        "item.started" => {
            let item = match v.get("item").filter(|i| i.is_object()) {
                Some(i) => i,
                None => return vec![],
            };
            if item.get("type").and_then(|x| x.as_str()) == Some("command_execution") {
                return vec![RunEvent::ToolUse {
                    id: item.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    name: "Bash".to_string(),
                    input: serde_json::json!({
                        "command": item.get("command").and_then(|x| x.as_str()).unwrap_or("")
                    }),
                }];
            }
            vec![]
        }
        "item.completed" => {
            let item = match v.get("item").filter(|i| i.is_object()) {
                Some(i) => i,
                None => return vec![],
            };
            match item.get("type").and_then(|x| x.as_str()) {
                Some("command_execution") => {
                    let is_error = match item.get("exit_code").and_then(|x| x.as_i64()) {
                        Some(code) => code != 0,
                        None => item.get("status").and_then(|x| x.as_str()) == Some("failed"),
                    };
                    vec![RunEvent::ToolResult {
                        tool_use_id: item.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        content: stringify_tool_content(item.get("aggregated_output")),
                        is_error,
                    }]
                }
                Some("agent_message") => match item.get("text").and_then(|x| x.as_str()) {
                    Some(t) if !t.is_empty() => vec![RunEvent::TextDelta { delta: t.to_string() }],
                    _ => vec![],
                },
                _ => vec![],
            }
        }
        "turn.completed" => {
            let usage = v.get("usage");
            vec![RunEvent::Usage {
                input_tokens: usage.and_then(|u| u.get("input_tokens")).and_then(|x| x.as_u64()),
                output_tokens: usage.and_then(|u| u.get("output_tokens")).and_then(|x| x.as_u64()),
            }]
        }
        "error" => vec![RunEvent::Error {
            message: extract_msg(v.get("message").or_else(|| v.get("error")), "Codex error"),
        }],
        "turn.failed" => vec![RunEvent::Error {
            message: extract_msg(v.get("error").or_else(|| v.get("message")), "Codex turn failed"),
        }],
        _ => vec![],
    }
}

/// Parse one line of Gemini/aipro's `--output-format stream-json` event stream
/// (JSONL) into `RunEvent`s. Port of `handleGeminiEvent` (`json-event-stream.ts`).
pub fn parse_gemini_event_line(line: &str) -> Vec<RunEvent> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
        "init" => vec![RunEvent::Status {
            label: "initializing".to_string(),
            model: v.get("model").and_then(|x| x.as_str()).map(String::from),
            session_id: None,
        }],
        "message" => {
            if v.get("role").and_then(|x| x.as_str()) == Some("assistant") {
                if let Some(c) = v.get("content").and_then(|x| x.as_str()) {
                    if !c.is_empty() {
                        return vec![RunEvent::TextDelta { delta: c.to_string() }];
                    }
                }
            }
            vec![]
        }
        "tool_use" => {
            let id = v.get("tool_id").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let name = v.get("tool_name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if id.is_empty() && name.is_empty() {
                return vec![];
            }
            vec![RunEvent::ToolUse {
                id,
                name,
                input: v.get("parameters").cloned().unwrap_or(Value::Null),
            }]
        }
        "tool_result" => {
            let is_error = v.get("status").and_then(|x| x.as_str()) == Some("error")
                || v.get("error").map(|e| !e.is_null()).unwrap_or(false);
            vec![RunEvent::ToolResult {
                tool_use_id: v.get("tool_id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                content: v
                    .get("output")
                    .and_then(|x| x.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| stringify_tool_content(v.get("output"))),
                is_error,
            }]
        }
        "error" => {
            let sev = v
                .get("severity")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_lowercase();
            let msg = extract_msg(v.get("message").or_else(|| v.get("error")), "Gemini CLI error");
            if sev == "warning" {
                vec![RunEvent::Status { label: format!("warning: {msg}"), model: None, session_id: None }]
            } else {
                vec![RunEvent::Error { message: msg }]
            }
        }
        "result" => {
            if v.get("status").and_then(|x| x.as_str()) == Some("error")
                || v.get("error").map(|e| !e.is_null()).unwrap_or(false)
            {
                return vec![RunEvent::Error {
                    message: extract_msg(v.get("error"), "Gemini CLI error"),
                }];
            }
            let stats = v.get("stats");
            vec![RunEvent::Usage {
                input_tokens: stats.and_then(|s| s.get("input_tokens")).and_then(|x| x.as_u64()),
                output_tokens: stats.and_then(|s| s.get("output_tokens")).and_then(|x| x.as_u64()),
            }]
        }
        _ => vec![],
    }
}

/// Start an agent run. Resolves the binary, spawns the child on a worker thread,
/// and returns the new run id immediately; events stream over `on_event` until a
/// terminal `End`. The slow work (child I/O) never touches the IPC thread.
#[tauri::command]
pub fn run_agent(
    app: tauri::AppHandle,
    args: RunArgs,
    on_event: Channel<RunEvent>,
) -> Result<String, String> {
    let def = agents::find(&args.agent_id).ok_or_else(|| format!("unknown agent: {}", args.agent_id))?;
    let run = def
        .run
        .as_ref()
        .ok_or_else(|| format!("agent is not runnable: {}", args.agent_id))?;

    if args.cwd.trim().is_empty() {
        return Err("no working folder selected".to_string());
    }

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let custom = settings::load(&config_dir).agent_custom_bin(&args.agent_id);
    let resolved = resolve_agent(def, custom.as_deref())
        .ok_or_else(|| format!("could not resolve executable for agent: {}", args.agent_id))?;

    let built = (run.build_args)(&RunCtx {
        cwd: &args.cwd,
        model: args.model.as_deref(),
        session_id: args.session_id.as_deref(),
        resume: args.resume.unwrap_or(false),
        prompt: &args.prompt,
    });

    // Mint a run id.
    let registry = app.state::<RunRegistry>();
    let n = registry.counter.fetch_add(1, Ordering::Relaxed) + 1;
    let run_id = format!("run-{n}");

    // Build the (not-yet-spawned) command on the caller thread.
    let mut cmd = crate::exec::command_for(&resolved.path, &built);
    cmd.current_dir(&args.cwd);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in run.env {
        cmd.env(k, v);
    }
    if run.prompt_via_stdin {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }

    let fmt = run.stream_format;
    let prompt_via_stdin = run.prompt_via_stdin;
    let prompt_format = run.prompt_format;
    let prompt = args.prompt.clone();
    let app2 = app.clone();
    let run_id2 = run_id.clone();

    std::thread::spawn(move || {
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = on_event.send(RunEvent::Error { message: format!("failed to launch agent: {e}") });
                let _ = on_event.send(RunEvent::End { code: None, status: "failed".into() });
                return;
            }
        };

        // Take the pipes so the reader loop never holds the child lock (which
        // `cancel_run` needs to kill).
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let canceled = Arc::new(AtomicBool::new(false));
        let child_arc = Arc::new(Mutex::new(child));
        let reg = app2.state::<RunRegistry>();
        reg.runs.lock().unwrap().insert(
            run_id2.clone(),
            RunHandle { child: child_arc.clone(), canceled: canceled.clone() },
        );

        // Deliver the prompt. Claude uses one stream-json user message, then we
        // close stdin to signal end-of-turn.
        if prompt_via_stdin {
            if let Some(mut si) = stdin {
                match prompt_format {
                    PromptFormat::ClaudeJson => {
                        let msg = serde_json::json!({
                            "type": "user",
                            "message": { "role": "user", "content": prompt }
                        });
                        let _ = writeln!(si, "{}", serde_json::to_string(&msg).unwrap_or_default());
                    }
                    PromptFormat::Text => {
                        let _ = si.write_all(prompt.as_bytes());
                    }
                }
                // `si` drops here → stdin closes.
            }
        }

        // Drain stderr on its own thread (avoids a full-pipe deadlock).
        let err_handle = stderr.map(|mut e| {
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = e.read_to_string(&mut s);
                s
            })
        });

        // Stream stdout line-by-line.
        if let Some(out) = stdout {
            let reader = BufReader::new(out);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                match fmt {
                    StreamFormat::ClaudeStreamJson => {
                        for ev in parse_claude_stream_line(&line) {
                            let _ = on_event.send(ev);
                        }
                    }
                    StreamFormat::CodexJson => {
                        for ev in parse_codex_event_line(&line) {
                            let _ = on_event.send(ev);
                        }
                    }
                    StreamFormat::GeminiJson => {
                        for ev in parse_gemini_event_line(&line) {
                            let _ = on_event.send(ev);
                        }
                    }
                    StreamFormat::Plain => {
                        let _ = on_event.send(RunEvent::Stdout { chunk: format!("{line}\n") });
                    }
                }
            }
        }

        // stdout is closed → the child is exiting (or was killed). `wait` returns
        // promptly; the lock is held only briefly.
        let status = child_arc.lock().unwrap().wait();
        let code = status.as_ref().ok().and_then(|s| s.code());
        let succeeded = matches!(status, Ok(ref s) if s.success());
        let was_canceled = canceled.load(Ordering::Relaxed);
        let stderr_str = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();

        reg.runs.lock().unwrap().remove(&run_id2);

        let final_status = if was_canceled {
            "canceled"
        } else if succeeded {
            "succeeded"
        } else {
            "failed"
        };

        if !succeeded && !was_canceled {
            let msg = if stderr_str.trim().is_empty() {
                format!("agent exited with code {code:?}")
            } else {
                stderr_str.trim().to_string()
            };
            let _ = on_event.send(RunEvent::Error { message: msg });
        }
        let _ = on_event.send(RunEvent::End { code, status: final_status.into() });
    });

    Ok(run_id)
}

/// Cancel an active run: kill the child's whole process tree (terminal `End`
/// follows on the worker thread once stdout closes).
#[tauri::command]
pub fn cancel_run(registry: tauri::State<RunRegistry>, run_id: String) -> Result<(), String> {
    if let Some(h) = registry.runs.lock().unwrap().get(&run_id) {
        h.canceled.store(true, Ordering::Relaxed);
        let mut child = h.child.lock().unwrap();
        // On Windows the agent runs as a node grandchild under a `.cmd` shim, so
        // killing the direct child (cmd.exe) leaves it running. `taskkill /T`
        // terminates the entire tree.
        #[cfg(windows)]
        {
            let pid = child.id().to_string();
            let _ = crate::exec::command_for("taskkill", &["/PID", pid.as_str(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        // Fallback / non-Windows: kill the direct child.
        let _ = child.kill();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_captures_model_and_session() {
        let evs = parse_claude_stream_line(
            r#"{"type":"system","subtype":"init","model":"claude-sonnet-4-5","session_id":"abc-123"}"#,
        );
        assert_eq!(
            evs,
            vec![RunEvent::Status {
                label: "initializing".into(),
                model: Some("claude-sonnet-4-5".into()),
                session_id: Some("abc-123".into()),
            }]
        );
    }

    #[test]
    fn assistant_text_and_tool_use() {
        let evs = parse_claude_stream_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hi"},{"type":"tool_use","id":"t1","name":"Read","input":{"path":"a.txt"}}]}}"#,
        );
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0], RunEvent::TextDelta { delta: "Hi".into() });
        match &evs[1] {
            RunEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "t1");
                assert_eq!(name, "Read");
                assert_eq!(input.get("path").and_then(|x| x.as_str()), Some("a.txt"));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn user_tool_result_stringifies_array_content() {
        let evs = parse_claude_stream_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"line1"},{"type":"text","text":"line2"}],"is_error":false}]}}"#,
        );
        assert_eq!(
            evs,
            vec![RunEvent::ToolResult {
                tool_use_id: "t1".into(),
                content: "line1\nline2".into(),
                is_error: false,
            }]
        );
    }

    #[test]
    fn result_maps_to_usage() {
        let evs = parse_claude_stream_line(
            r#"{"type":"result","subtype":"success","usage":{"input_tokens":10,"output_tokens":20}}"#,
        );
        assert_eq!(
            evs,
            vec![RunEvent::Usage { input_tokens: Some(10), output_tokens: Some(20) }]
        );
    }

    #[test]
    fn blank_and_non_json_yield_nothing() {
        assert!(parse_claude_stream_line("").is_empty());
        assert!(parse_claude_stream_line("   ").is_empty());
        assert!(parse_claude_stream_line("not json").is_empty());
        // Unknown event types are ignored.
        assert!(parse_claude_stream_line(r#"{"type":"stream_event","event":{}}"#).is_empty());
    }

    #[test]
    fn text_delta_serializes_camel_case() {
        let json = serde_json::to_string(&RunEvent::TextDelta { delta: "x".into() }).unwrap();
        assert_eq!(json, r#"{"type":"textDelta","delta":"x"}"#);
    }

    // ── codex parser ──────────────────────────────────────────────────────────

    #[test]
    fn codex_thread_started_captures_session() {
        let evs = parse_codex_event_line(r#"{"type":"thread.started","thread_id":"th_42"}"#);
        assert_eq!(
            evs,
            vec![RunEvent::Status {
                label: "initializing".into(),
                model: None,
                session_id: Some("th_42".into()),
            }]
        );
    }

    #[test]
    fn codex_agent_message_is_text() {
        let evs = parse_codex_event_line(
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"안녕"}}"#,
        );
        assert_eq!(evs, vec![RunEvent::TextDelta { delta: "안녕".into() }]);
    }

    #[test]
    fn codex_command_execution_started_then_result() {
        let started = parse_codex_event_line(
            r#"{"type":"item.started","item":{"type":"command_execution","id":"c1","command":"ls"}}"#,
        );
        match &started[..] {
            [RunEvent::ToolUse { id, name, .. }] => {
                assert_eq!(id, "c1");
                assert_eq!(name, "Bash");
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
        let done = parse_codex_event_line(
            r#"{"type":"item.completed","item":{"type":"command_execution","id":"c1","aggregated_output":"out","exit_code":0}}"#,
        );
        assert_eq!(
            done,
            vec![RunEvent::ToolResult { tool_use_id: "c1".into(), content: "out".into(), is_error: false }]
        );
    }

    #[test]
    fn codex_turn_completed_and_failed() {
        assert_eq!(
            parse_codex_event_line(r#"{"type":"turn.completed","usage":{"input_tokens":3,"output_tokens":7}}"#),
            vec![RunEvent::Usage { input_tokens: Some(3), output_tokens: Some(7) }]
        );
        assert_eq!(
            parse_codex_event_line(r#"{"type":"turn.failed","error":{"message":"boom"}}"#),
            vec![RunEvent::Error { message: "boom".into() }]
        );
    }

    // ── gemini parser ─────────────────────────────────────────────────────────

    #[test]
    fn gemini_init_and_assistant_text() {
        assert_eq!(
            parse_gemini_event_line(r#"{"type":"init","model":"glm-5.1"}"#),
            vec![RunEvent::Status { label: "initializing".into(), model: Some("glm-5.1".into()), session_id: None }]
        );
        assert_eq!(
            parse_gemini_event_line(r#"{"type":"message","role":"assistant","content":"hi"}"#),
            vec![RunEvent::TextDelta { delta: "hi".into() }]
        );
        // user echoes and empty content produce nothing.
        assert!(parse_gemini_event_line(r#"{"type":"message","role":"user","content":"q"}"#).is_empty());
    }

    #[test]
    fn gemini_result_usage_and_errors() {
        assert_eq!(
            parse_gemini_event_line(r#"{"type":"result","stats":{"input_tokens":5,"output_tokens":9}}"#),
            vec![RunEvent::Usage { input_tokens: Some(5), output_tokens: Some(9) }]
        );
        assert_eq!(
            parse_gemini_event_line(r#"{"type":"error","message":"Model not found"}"#),
            vec![RunEvent::Error { message: "Model not found".into() }]
        );
        match &parse_gemini_event_line(r#"{"type":"error","severity":"warning","message":"256-color"}"#)[..] {
            [RunEvent::Status { label, .. }] => assert!(label.starts_with("warning")),
            other => panic!("expected Status warning, got {other:?}"),
        }
    }
}

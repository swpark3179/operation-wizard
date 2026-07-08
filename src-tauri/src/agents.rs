//! Agent definitions + registry.
//!
//! Ports the per-agent "runtime definition" data from Open Design
//! (`apps/daemon/src/runtimes/defs/*.ts`) so the detection pipeline in
//! `detect.rs` is driven by data, not OpenCode-specific code. Adding an agent
//! is a new entry in `AGENT_DEFS` (plus a parser in `detect.rs` if it lists
//! models in a new format).

use std::time::Duration;

use crate::detect::{parse_codex_debug_models, parse_line_separated_models, ModelOption};

/// Which stdout parser the run engine (`run.rs`) uses for an agent's output.
#[derive(Clone, Copy)]
pub enum StreamFormat {
    /// Claude Code's `--output-format stream-json` (JSON object per line).
    ClaudeStreamJson,
    /// Codex `exec --json` event stream (JSONL). Parsed by `run::parse_codex_event_line`.
    CodexJson,
    /// Gemini/aipro `--output-format stream-json` event stream (JSONL).
    /// Parsed by `run::parse_gemini_event_line`.
    GeminiJson,
    /// Unstructured stdout, forwarded line-by-line as raw text.
    Plain,
}

/// How the prompt is framed when delivered over stdin.
#[derive(Clone, Copy)]
pub enum PromptFormat {
    /// One Claude `stream-json` user message line (`{"type":"user",...}`).
    ClaudeJson,
    /// The raw prompt text (codex/gemini/aipro read stdin as plain text).
    Text,
}

/// Per-turn context handed to a `RunSpec::build_args`. Mirrors Open Design's
/// `RuntimeContext` (subset needed for this app's runs).
pub struct RunCtx<'a> {
    pub cwd: &'a str,
    /// Selected model id, or `None`/`"default"` to let the CLI use its config.
    pub model: Option<&'a str>,
    /// Session id: for claude a client-minted UUID (used with `--session-id` on
    /// the first turn, `--resume` after); for codex the captured `thread_id`
    /// (used with `exec resume`). `None` = new/sessionless.
    pub session_id: Option<&'a str>,
    /// True when continuing a prior turn (resume) vs. starting fresh.
    pub resume: bool,
    /// The user prompt (used by plain agents that take it as an argument;
    /// stdin-based agents deliver it separately in `run.rs`).
    pub prompt: &'a str,
}

/// How to *run* an agent (build args + deliver prompt + parse output). The run
/// counterpart of `ModelsProbe`; ports Open Design's `RuntimeAgentDef.buildArgs`
/// / `streamFormat` / `promptViaStdin` / `env`.
pub struct RunSpec {
    pub build_args: fn(&RunCtx) -> Vec<String>,
    /// Deliver the prompt over stdin (vs. as a CLI argument).
    pub prompt_via_stdin: bool,
    /// stdin framing (only meaningful when `prompt_via_stdin`).
    pub prompt_format: PromptFormat,
    pub stream_format: StreamFormat,
    /// Extra environment variables set on the child (merged over the inherited
    /// parent env). Empty for most agents.
    pub env: &'static [(&'static str, &'static str)],
}

impl RunSpec {
    /// Best-effort spec for agents without a first-class integration: pass the
    /// prompt as an argument and forward raw stdout.
    pub const fn plain() -> RunSpec {
        RunSpec {
            build_args: plain_build_args,
            prompt_via_stdin: false,
            prompt_format: PromptFormat::Text,
            stream_format: StreamFormat::Plain,
            env: &[],
        }
    }
}

/// Env shared by gemini and the gemini-compatible aipro (`defs/gemini.ts`).
const GEMINI_ENV: &[(&str, &str)] = &[("GEMINI_CLI_TRUST_WORKSPACE", "true")];

/// Append `--model <m>` unless the model is empty/`"default"`.
fn push_model(a: &mut Vec<String>, model: Option<&str>) {
    if let Some(m) = model {
        if !m.is_empty() && m != "default" {
            a.push("--model".to_string());
            a.push(m.to_string());
        }
    }
}

/// Claude Code headless invocation (`defs/claude.ts`): stream-json in/out, bypass
/// permission prompts, scope file access to the working dir. Session is a
/// client-minted UUID — `--session-id` on the first turn, `--resume` after.
fn claude_build_args(ctx: &RunCtx) -> Vec<String> {
    let mut a = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--permission-mode".to_string(),
        "bypassPermissions".to_string(),
        "--add-dir".to_string(),
        ctx.cwd.to_string(),
    ];
    push_model(&mut a, ctx.model);
    if let Some(sid) = ctx.session_id {
        if !sid.is_empty() {
            a.push(if ctx.resume { "--resume" } else { "--session-id" }.to_string());
            a.push(sid.to_string());
        }
    }
    a
}

/// Codex non-interactive invocation (`defs/codex.ts`): `codex exec [resume]
/// --json ...`, prompt on stdin. Windows requires an explicit sandbox. Session is
/// capture-style: codex mints its own `thread_id` (captured from the stream) and
/// we replay it as `exec resume <id>`.
fn codex_build_args(ctx: &RunCtx) -> Vec<String> {
    let resuming = ctx.resume && ctx.session_id.map(|s| !s.is_empty()).unwrap_or(false);
    let mut a = vec!["exec".to_string()];
    if resuming {
        a.push("resume".to_string());
    }
    a.push("--json".to_string());
    a.push("--skip-git-repo-check".to_string());
    // Windows: codex needs a sandbox; `exec resume` rejects `--sandbox`, so a
    // config override is used there instead.
    if resuming {
        a.push("-c".to_string());
        a.push("sandbox_mode=\"danger-full-access\"".to_string());
    } else {
        a.push("--sandbox".to_string());
        a.push("danger-full-access".to_string());
    }
    push_model(&mut a, ctx.model);
    if !resuming {
        // create-only: working dir (resume inherits the thread's cwd).
        a.push("-C".to_string());
        a.push(ctx.cwd.to_string());
    } else if let Some(sid) = ctx.session_id {
        // resume: the session id is the trailing positional.
        a.push(sid.to_string());
    }
    a
}

/// Gemini CLI invocation (`defs/gemini.ts`): stream-json out, `--yolo` to skip
/// approvals; prompt on stdin. No CLI session (context re-sent each turn).
fn gemini_build_args(ctx: &RunCtx) -> Vec<String> {
    let mut a = vec![
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--yolo".to_string(),
    ];
    push_model(&mut a, ctx.model);
    a
}

/// AI Pro (gemini-compatible, in-house). Same as gemini, but always pins an
/// in-house model via `--model` — a missing/`default` model makes aipro request a
/// public Gemini id its backend rejects ("Model not found").
fn aipro_build_args(ctx: &RunCtx) -> Vec<String> {
    let model = match ctx.model {
        Some(m) if !m.is_empty() && m != "default" => m,
        _ => "glm-5.1",
    };
    vec![
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--yolo".to_string(),
        "--model".to_string(),
        model.to_string(),
    ]
}

/// Plain fallback: `<bin> -p "<prompt>"`. Best-effort for agents without a
/// first-class parser (opencode/antigravity — see docs/design/05-decisions.md D19).
fn plain_build_args(ctx: &RunCtx) -> Vec<String> {
    vec!["-p".to_string(), ctx.prompt.to_string()]
}

/// How to list models for an agent (the model-listing probe). Agents without a
/// model-listing command (claude/gemini/antigravity here) set this to `None`
/// and always use the static fallback catalog.
pub struct ModelsProbe {
    /// Subcommand/args, e.g. `["models"]` or `["debug", "models"]`.
    pub args: &'static [&'static str],
    /// Parse stdout into models, or `None` to fall back (parse error / no
    /// models survived).
    pub parse: fn(&str) -> Option<Vec<ModelOption>>,
    pub timeout: Duration,
}

/// Declarative definition of a detectable CLI agent.
pub struct AgentDef {
    pub id: &'static str,
    pub name: &'static str,
    /// Binary names to try, in order (Open Design: `bin` then `fallbackBins`).
    pub bin_candidates: &'static [&'static str],
    /// Env var that overrides the binary path (e.g. `OPENCODE_BIN`); `None` if
    /// the agent has no env override.
    pub env_var: Option<&'static str>,
    /// Extra `%USERPROFILE%`-relative dirs to scan beyond the shared toolchain
    /// set (e.g. opencode's `.opencode\bin`). Empty for most agents.
    pub extra_search_subdirs: &'static [&'static str],
    pub version_timeout: Duration,
    /// `Some` to probe a live model list; `None` for fallback-only agents.
    pub models_probe: Option<ModelsProbe>,
    /// Static catalog used when no live list is available. `(id, label)` pairs;
    /// the synthetic `default` option is prepended in code (see
    /// `detect::fallback_from`).
    pub fallback_models: &'static [(&'static str, &'static str)],
    /// How to run the agent in the workspace (`None` = detection-only). Claude
    /// Code is first-class; others use the plain fallback (see `run.rs`).
    pub run: Option<RunSpec>,
}

const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// The agents this app knows how to detect, in display order. Values are ported
/// 1:1 from Open Design's `apps/daemon/src/runtimes/defs/*.ts`.
pub static AGENT_DEFS: [AgentDef; 6] = [
    AgentDef {
        id: "opencode",
        name: "OpenCode",
        bin_candidates: &["opencode-cli", "opencode"],
        env_var: Some("OPENCODE_BIN"),
        extra_search_subdirs: &[".opencode\\bin"],
        version_timeout: VERSION_TIMEOUT,
        models_probe: Some(ModelsProbe {
            args: &["models"],
            parse: parse_line_separated_models,
            timeout: Duration::from_secs(15),
        }),
        fallback_models: &[
            ("anthropic/claude-sonnet-4-5", "anthropic/claude-sonnet-4-5"),
            ("openai/gpt-5", "openai/gpt-5"),
            ("google/gemini-2.5-pro", "google/gemini-2.5-pro"),
        ],
        run: Some(RunSpec::plain()),
    },
    AgentDef {
        id: "claude",
        name: "Claude Code",
        bin_candidates: &["claude", "openclaude"],
        env_var: Some("CLAUDE_BIN"),
        extra_search_subdirs: &[],
        version_timeout: VERSION_TIMEOUT,
        // No simple line-based `models` command; Open Design fetches via its own
        // MMS routes (proxy infra, out of scope here) → fallback catalog only.
        models_probe: None,
        fallback_models: &[
            ("sonnet", "Sonnet (alias)"),
            ("opus", "Opus (alias)"),
            ("haiku", "Haiku (alias)"),
            ("claude-opus-4-5", "claude-opus-4-5"),
            ("claude-sonnet-4-5", "claude-sonnet-4-5"),
            ("claude-haiku-4-5", "claude-haiku-4-5"),
        ],
        run: Some(RunSpec {
            build_args: claude_build_args,
            prompt_via_stdin: true,
            prompt_format: PromptFormat::ClaudeJson,
            stream_format: StreamFormat::ClaudeStreamJson,
            env: &[],
        }),
    },
    AgentDef {
        id: "codex",
        name: "Codex CLI",
        bin_candidates: &["codex"],
        env_var: Some("CODEX_BIN"),
        extra_search_subdirs: &[],
        version_timeout: VERSION_TIMEOUT,
        models_probe: Some(ModelsProbe {
            args: &["debug", "models"],
            parse: parse_codex_debug_models,
            timeout: Duration::from_secs(5),
        }),
        fallback_models: &[
            ("gpt-5.5", "gpt-5.5"),
            ("gpt-5.4", "gpt-5.4"),
            ("gpt-5.4-mini", "gpt-5.4-mini"),
            ("gpt-5.3-codex", "gpt-5.3-codex"),
            ("gpt-5.1", "gpt-5.1"),
            ("gpt-5.1-codex-mini", "gpt-5.1-codex-mini"),
            ("gpt-5-codex", "gpt-5-codex"),
            ("gpt-5", "gpt-5"),
            ("o3", "o3"),
            ("o4-mini", "o4-mini"),
        ],
        run: Some(RunSpec {
            build_args: codex_build_args,
            prompt_via_stdin: true,
            prompt_format: PromptFormat::Text,
            stream_format: StreamFormat::CodexJson,
            env: &[],
        }),
    },
    AgentDef {
        id: "gemini",
        name: "Gemini CLI",
        bin_candidates: &["gemini"],
        env_var: Some("GEMINI_BIN"),
        extra_search_subdirs: &[],
        version_timeout: VERSION_TIMEOUT,
        models_probe: None,
        fallback_models: &[
            ("gemini-3-pro-preview", "gemini-3-pro-preview"),
            ("gemini-3-flash-preview", "gemini-3-flash-preview"),
            ("gemini-2.5-pro", "gemini-2.5-pro"),
            ("gemini-2.5-flash", "gemini-2.5-flash"),
            ("gemini-2.5-flash-lite", "gemini-2.5-flash-lite"),
        ],
        run: Some(RunSpec {
            build_args: gemini_build_args,
            prompt_via_stdin: true,
            prompt_format: PromptFormat::Text,
            stream_format: StreamFormat::GeminiJson,
            env: GEMINI_ENV,
        }),
    },
    AgentDef {
        id: "antigravity",
        name: "Antigravity",
        bin_candidates: &["agy"],
        // Upstream has no env override; `ANTIGRAVITY_BIN` is our deliberate
        // addition for consistency (see docs/design/05-decisions.md).
        env_var: Some("ANTIGRAVITY_BIN"),
        extra_search_subdirs: &[],
        version_timeout: VERSION_TIMEOUT,
        models_probe: None,
        fallback_models: &[
            ("Gemini 3.1 Pro (High)", "Gemini 3.1 Pro (High)"),
            ("Gemini 3.1 Pro (Low)", "Gemini 3.1 Pro (Low)"),
            ("Gemini 3.5 Flash (High)", "Gemini 3.5 Flash (High)"),
            ("Gemini 3.5 Flash (Medium)", "Gemini 3.5 Flash (Medium)"),
            ("Gemini 3.5 Flash (Low)", "Gemini 3.5 Flash (Low)"),
            ("Claude Sonnet 4.6 (Thinking)", "Claude Sonnet 4.6 (Thinking)"),
            ("Claude Opus 4.6 (Thinking)", "Claude Opus 4.6 (Thinking)"),
            ("GPT-OSS 120B (Medium)", "GPT-OSS 120B (Medium)"),
        ],
        run: Some(RunSpec::plain()),
    },
    AgentDef {
        // In-house tool, Gemini-CLI-compatible. Originally an Open Design local
        // profile (`baseAgent: "gemini"` in ~/.open-design/agents.local.json);
        // baked in here as a built-in def. Gemini-like → no model-listing
        // command, so fallback-only. The profile's spawn env
        // (GEMINI_CLI_TRUST_WORKSPACE) is run-time only and irrelevant to detection.
        id: "aipro",
        name: "AI Pro",
        bin_candidates: &["aipro"],
        env_var: Some("AIPRO_BIN"),
        extra_search_subdirs: &[],
        version_timeout: VERSION_TIMEOUT,
        models_probe: None,
        fallback_models: &[
            ("glm-5.1", "GLM-5.1"),
            ("qwen3.6-27b", "Qwen3.6-27b"),
            ("gpt-oss-120b", "Gpt-Oss-120b"),
        ],
        run: Some(RunSpec {
            build_args: aipro_build_args,
            prompt_via_stdin: true,
            prompt_format: PromptFormat::Text,
            stream_format: StreamFormat::GeminiJson,
            env: GEMINI_ENV,
        }),
    },
];

/// All known agent definitions, in display order.
pub fn all() -> &'static [AgentDef] {
    &AGENT_DEFS
}

/// Look up a definition by id (`None` for an unknown id).
pub fn find(id: &str) -> Option<&'static AgentDef> {
    AGENT_DEFS.iter().find(|d| d.id == id)
}

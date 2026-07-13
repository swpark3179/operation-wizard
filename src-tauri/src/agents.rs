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
    /// Extra directories the agent should be able to read beyond `cwd` (the
    /// project's codebase path + armed skill resource folders). claude maps
    /// each to `--add-dir`, gemini/aipro to `--include-directories` (D52);
    /// codex already runs full-access, plain agents degrade to the prompt-only
    /// mention the frontend always adds — D45.
    pub extra_dirs: &'a [String],
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

/// Claude run env (D53): lift the Bash tool's default/max command timeouts so a
/// long build or search in a document step is not cut off mid-turn (which reads
/// as a dropped response). Applied only when the user has not set their own
/// value (see `run.rs` env merge).
const CLAUDE_ENV: &[(&str, &str)] = &[
    ("BASH_DEFAULT_TIMEOUT_MS", "300000"),
    ("BASH_MAX_TIMEOUT_MS", "1200000"),
];

/// Append `--model <m>` unless the model is empty/`"default"`.
fn push_model(a: &mut Vec<String>, model: Option<&str>) {
    if let Some(m) = model {
        if !m.is_empty() && m != "default" {
            a.push("--model".to_string());
            a.push(m.to_string());
        }
    }
}

/// Append `--include-directories <dir>` per extra dir (gemini CLI's official
/// multi-directory workspace flag; aipro is gemini-compatible). This is the
/// gemini counterpart of claude's `--add-dir` (D52): without it the CLI's
/// workspace trust is limited to the cwd and the selected codebase folder is
/// unreadable, so the codebase-analysis step used to fall back to analyzing the
/// (output-only) workdir. One flag per dir — the comma-separated form would
/// mis-split a path containing a comma.
fn push_include_directories(a: &mut Vec<String>, extra_dirs: &[String]) {
    for d in extra_dirs {
        let d = d.trim();
        if !d.is_empty() {
            a.push("--include-directories".to_string());
            a.push(d.to_string());
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
    for d in ctx.extra_dirs {
        let d = d.trim();
        if !d.is_empty() {
            a.push("--add-dir".to_string());
            a.push(d.to_string());
        }
    }
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
/// Extra dirs ride `--include-directories` (D52).
fn gemini_build_args(ctx: &RunCtx) -> Vec<String> {
    let mut a = vec![
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--yolo".to_string(),
    ];
    push_include_directories(&mut a, ctx.extra_dirs);
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
    let mut a = vec![
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--yolo".to_string(),
        "--model".to_string(),
        model.to_string(),
    ];
    push_include_directories(&mut a, ctx.extra_dirs);
    a
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

/// Transport of an agent (D64). `Local` agents are CLI binaries resolved on the
/// filesystem and run as child processes (all fields below apply). `Remote`
/// agents are HTTP APIs (Fabrix): detection and run bypass resolve/spawn and go
/// through `fabrix.rs`, so `bin_candidates`/`models_probe`/`run` are left empty.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentKind {
    Local,
    Remote,
}

/// Declarative definition of a detectable agent.
pub struct AgentDef {
    pub id: &'static str,
    pub name: &'static str,
    /// Local CLI process vs. remote HTTP API (D64).
    pub kind: AgentKind,
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
pub static AGENT_DEFS: [AgentDef; 7] = [
    AgentDef {
        id: "opencode",
        name: "OpenCode",
        kind: AgentKind::Local,
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
        kind: AgentKind::Local,
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
            env: CLAUDE_ENV,
        }),
    },
    AgentDef {
        id: "codex",
        name: "Codex CLI",
        kind: AgentKind::Local,
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
        kind: AgentKind::Local,
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
        kind: AgentKind::Local,
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
        kind: AgentKind::Local,
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
    AgentDef {
        // Remote HTTP API agent (D64) — the first non-CLI agent. Detection and
        // run go through `fabrix.rs` (GET model list, POST + SSE chat), so the
        // CLI-oriented fields are empty and `run` is `None`. Connection config
        // (endpoint + headers) lives in `settings.fabrix`, not a `*_BIN` path.
        id: "fabrix",
        name: "Fabrix",
        kind: AgentKind::Remote,
        bin_candidates: &[],
        env_var: None,
        extra_search_subdirs: &[],
        version_timeout: VERSION_TIMEOUT,
        models_probe: None,
        fallback_models: &[],
        run: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(extra_dirs: &'a [String]) -> RunCtx<'a> {
        RunCtx {
            cwd: "F:\\work",
            model: None,
            session_id: None,
            resume: false,
            prompt: "hi",
            extra_dirs,
        }
    }

    #[test]
    fn claude_add_dir_pairs_for_extra_dirs() {
        // No extra dirs → exactly one --add-dir (the cwd).
        let a = claude_build_args(&ctx(&[]));
        assert_eq!(a.iter().filter(|s| *s == "--add-dir").count(), 1);

        // Non-empty entries each get an --add-dir pair after the cwd; blank
        // entries are skipped and values are trimmed.
        let dirs = vec!["F:\\legacy".to_string(), "  ".to_string(), " F:\\skills\\sa ".to_string()];
        let a = claude_build_args(&ctx(&dirs));
        let pairs: Vec<&str> = a
            .iter()
            .enumerate()
            .filter(|(_, s)| *s == "--add-dir")
            .map(|(i, _)| a[i + 1].as_str())
            .collect();
        assert_eq!(pairs, vec!["F:\\work", "F:\\legacy", "F:\\skills\\sa"]);
    }

    #[test]
    fn codex_ignores_extra_dirs() {
        // codex runs a full-access sandbox — no per-dir grant flags needed.
        let dirs = vec!["F:\\legacy".to_string()];
        let args = codex_build_args(&ctx(&dirs));
        assert!(!args.iter().any(|s| s == "--add-dir" || s == "--include-directories" || s == "F:\\legacy"));
    }

    #[test]
    fn gemini_and_aipro_include_directories_for_extra_dirs() {
        // No extra dirs → no flag at all.
        assert!(!gemini_build_args(&ctx(&[])).iter().any(|s| s == "--include-directories"));
        assert!(!aipro_build_args(&ctx(&[])).iter().any(|s| s == "--include-directories"));

        // One flag pair per non-blank dir, trimmed (D52).
        let dirs = vec!["F:\\legacy".to_string(), "  ".to_string(), " F:\\skills\\sa ".to_string()];
        for args in [gemini_build_args(&ctx(&dirs)), aipro_build_args(&ctx(&dirs))] {
            let pairs: Vec<&str> = args
                .iter()
                .enumerate()
                .filter(|(_, s)| *s == "--include-directories")
                .map(|(i, _)| args[i + 1].as_str())
                .collect();
            assert_eq!(pairs, vec!["F:\\legacy", "F:\\skills\\sa"]);
        }
    }
}

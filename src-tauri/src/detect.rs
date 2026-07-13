//! Agent detection pipeline (definition-driven). Mirrors Open Design's `probe()`
//! (`apps/daemon/src/runtimes/detection.ts`); the per-agent data lives in
//! `agents.rs` (ported from `apps/daemon/src/runtimes/defs/*.ts`).

use serde::Serialize;

use crate::agents::AgentDef;
use crate::exec::run_capture;
use crate::resolve::resolve_agent;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DetectedAgent {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    /// "custom-path" | "path" | "not-found"
    pub source: String,
    pub models: Vec<ModelOption>,
    /// "live" | "fallback"
    pub models_source: String,
    /// "not-on-path" | "not-executable" | "missing-target" | None
    pub diagnostic: Option<String>,
}

fn default_model() -> ModelOption {
    ModelOption { id: "default".into(), label: "Default (CLI config)".into() }
}

fn model(id: &str) -> ModelOption {
    ModelOption { id: id.into(), label: id.into() }
}

/// Build the static fallback catalog for an agent: the synthetic `default`
/// option followed by the agent's declared `(id, label)` pairs.
fn fallback_from(def: &AgentDef) -> Vec<ModelOption> {
    let mut out = vec![default_model()];
    out.extend(
        def.fallback_models
            .iter()
            .map(|(id, label)| ModelOption { id: (*id).into(), label: (*label).into() }),
    );
    out
}

/// Port of `parseLineSeparatedModels` (`defs/shared.ts`): one id per line,
/// trimmed, drop empties and `#` comments, de-dupe preserving order, prepend
/// the synthetic default option. Always returns `Some` — kept fallible only for
/// a uniform parser signature with the JSON parsers.
pub(crate) fn parse_line_separated_models(stdout: &str) -> Option<Vec<ModelOption>> {
    let mut out = vec![default_model()];
    let mut seen = std::collections::HashSet::new();
    for line in stdout.lines() {
        let id = line.trim();
        if id.is_empty() || id.starts_with('#') {
            continue;
        }
        if seen.insert(id.to_string()) {
            out.push(model(id));
        }
    }
    Some(out)
}

/// Port of `parseCodexDebugModels` (`defs/codex.ts`): parse `codex debug models`
/// JSON (`{ "models": [...] }`), skip `visibility == "hidden"`, take id from
/// `slug` else `id`, label from `display_name` else `name` else id, de-dupe,
/// prepend default. Returns `None` on parse failure or when no real models
/// survive (the caller then uses the fallback catalog).
pub(crate) fn parse_codex_debug_models(stdout: &str) -> Option<Vec<ModelOption>> {
    #[derive(serde::Deserialize)]
    struct Entry {
        slug: Option<String>,
        id: Option<String>,
        display_name: Option<String>,
        name: Option<String>,
        visibility: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Root {
        models: Option<Vec<Entry>>,
    }

    let root: Root = serde_json::from_str(stdout.trim()).ok()?;
    let entries = root.models?;
    let mut out = vec![default_model()];
    let mut seen: std::collections::HashSet<String> =
        std::collections::HashSet::from(["default".to_string()]);
    for e in entries {
        if e.visibility.as_deref() == Some("hidden") {
            continue;
        }
        let id = e
            .slug
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| e.id.as_deref().map(str::trim).filter(|s| !s.is_empty()));
        let id = match id {
            Some(id) => id,
            None => continue,
        };
        if !seen.insert(id.to_string()) {
            continue;
        }
        let label = e
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| e.name.as_deref().map(str::trim).filter(|s| !s.is_empty()))
            .unwrap_or(id);
        out.push(ModelOption { id: id.into(), label: label.into() });
    }
    if out.len() > 1 {
        Some(out)
    } else {
        None
    }
}

/// Blocking detection for one agent — run on a blocking thread (see
/// `lib::detect_agent`). `def` is `'static`, so it crosses the thread freely.
pub fn detect_agent_blocking(def: &'static AgentDef, custom: Option<String>) -> DetectedAgent {
    let mut agent = DetectedAgent {
        id: def.id.into(),
        name: def.name.into(),
        available: false,
        path: None,
        version: None,
        source: "not-found".into(),
        models: fallback_from(def),
        models_source: "fallback".into(),
        diagnostic: None,
    };

    // 1. Resolve the executable.
    let resolved = match resolve_agent(def, custom.as_deref()) {
        Some(r) => r,
        None => {
            agent.diagnostic = Some("not-on-path".into());
            return agent;
        }
    };
    agent.path = Some(resolved.path.clone());
    agent.source = resolved.source;

    // 2. Version probe (`--version`), classified like Open Design.
    let v = run_capture(&resolved.path, &["--version"], def.version_timeout);
    if let Some(err) = &v.spawn_error {
        agent.diagnostic = Some(
            match err.kind() {
                std::io::ErrorKind::PermissionDenied => "not-executable",
                _ => "missing-target",
            }
            .into(),
        );
        return agent;
    }
    match v.status_code {
        Some(127) => {
            agent.diagnostic = Some("missing-target".into());
            return agent;
        }
        Some(126) => {
            agent.diagnostic = Some("not-executable".into());
            return agent;
        }
        _ => {}
    }

    // Spawned successfully → available, even if `--version` itself failed.
    agent.available = true;
    if !v.timed_out && v.status_code == Some(0) {
        agent.version = v
            .stdout
            .lines()
            .next()
            .map(|l| l.trim().to_string())
            .filter(|s| !s.is_empty());
    }

    // 3. Model listing (definition-driven); fall back to the static catalog on
    //    failure or for agents with no model-listing command.
    if let Some(probe) = &def.models_probe {
        let m = run_capture(&resolved.path, probe.args, probe.timeout);
        let probe_ok = m.spawn_error.is_none()
            && !m.timed_out
            && m.status_code == Some(0)
            && !m.stdout.trim().is_empty();
        if probe_ok {
            if let Some(models) = (probe.parse)(&m.stdout) {
                agent.models = models;
                agent.models_source = "live".into();
            }
        }
    }

    agent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents;

    #[test]
    fn parses_trims_dedupes_and_prepends_default() {
        let out = "anthropic/claude-sonnet-4-5\n# a comment\n\n  openai/gpt-5  \nopenai/gpt-5\n";
        let models = parse_line_separated_models(out).unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["default", "anthropic/claude-sonnet-4-5", "openai/gpt-5"]);
    }

    #[test]
    fn codex_parser_skips_hidden_dedupes_and_picks_label() {
        let json = r#"{
            "models": [
                { "slug": "gpt-5.5", "display_name": "GPT-5.5" },
                { "slug": "secret", "visibility": "hidden" },
                { "id": "o3" },
                { "slug": "gpt-5.5", "display_name": "dupe" },
                { "slug": "named", "name": "Display Via Name" }
            ]
        }"#;
        let models = parse_codex_debug_models(json).unwrap();
        let pairs: Vec<(&str, &str)> =
            models.iter().map(|m| (m.id.as_str(), m.label.as_str())).collect();
        assert_eq!(
            pairs,
            [
                ("default", "Default (CLI config)"),
                ("gpt-5.5", "GPT-5.5"),
                ("o3", "o3"),
                ("named", "Display Via Name"),
            ]
        );
    }

    #[test]
    fn codex_parser_returns_none_on_garbage_or_empty() {
        assert!(parse_codex_debug_models("not json").is_none());
        assert!(parse_codex_debug_models(r#"{"models":[]}"#).is_none());
        assert!(parse_codex_debug_models(r#"{"models":[{"visibility":"hidden","slug":"x"}]}"#)
            .is_none());
    }

    #[test]
    fn registry_ids_unique_and_have_fallbacks() {
        let defs = agents::all();
        assert_eq!(defs.len(), 7);
        let mut seen = std::collections::HashSet::new();
        for d in defs {
            assert!(!d.id.is_empty());
            assert!(seen.insert(d.id), "duplicate id {}", d.id);
            // Local agents ship an offline catalog; remote (Fabrix) has none
            // (models are fetched live over HTTP).
            if d.kind == agents::AgentKind::Local {
                assert!(!d.fallback_models.is_empty(), "{} has no fallback models", d.id);
            }
        }
        assert_eq!(
            agents::find("fabrix").map(|d| d.kind),
            Some(agents::AgentKind::Remote),
            "fabrix must be a remote agent"
        );
        for id in ["opencode", "claude", "codex", "gemini", "antigravity", "aipro", "fabrix"] {
            assert!(agents::find(id).is_some(), "missing {id}");
        }
    }

    /// End-to-end: resolve a custom-path `.cmd`, probe `--version`, list `models`.
    /// Exercises resolve.rs + exec.rs (cmd.exe wrapping) + the parse pipeline.
    #[cfg(windows)]
    #[test]
    fn detects_stub_cmd_via_custom_path() {
        use std::io::Write;

        let dir = std::env::temp_dir().join(format!("ow-detect-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stub = dir.join("opencode.cmd");
        let body = "@echo off\r\n\
                    if \"%~1\"==\"--version\" (\r\n echo 1.99.0-stub\r\n exit /b 0\r\n)\r\n\
                    if \"%~1\"==\"models\" (\r\n echo anthropic/claude-sonnet-4-5\r\n echo openai/gpt-5\r\n exit /b 0\r\n)\r\n\
                    exit /b 0\r\n";
        std::fs::File::create(&stub)
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();

        let def = agents::find("opencode").unwrap();
        let agent = detect_agent_blocking(def, Some(stub.to_string_lossy().into_owned()));

        assert!(agent.available, "stub should be detected");
        assert_eq!(agent.source, "custom-path");
        assert_eq!(agent.version.as_deref(), Some("1.99.0-stub"));
        assert_eq!(agent.models_source, "live");
        // default + the two stub models
        assert_eq!(agent.models.len(), 3);
        assert_eq!(agent.models[0].id, "default");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_custom_path_falls_through_to_search() {
        let def = agents::find("opencode").unwrap();
        let agent =
            detect_agent_blocking(def, Some("Z:\\nope\\does\\not\\exist\\opencode.exe".into()));
        // With no real opencode installed, resolution fails → not-on-path.
        if !agent.available {
            assert_eq!(agent.source, "not-found");
            assert_eq!(agent.diagnostic.as_deref(), Some("not-on-path"));
        }
    }
}

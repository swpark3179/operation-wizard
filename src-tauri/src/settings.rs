//! Persisted app settings, stored as `settings.json` in the app config dir.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Category ids — keep in sync with the `Category` union in
/// `src/components/workspace.ts` (frontend owns the display metadata).
pub const CATEGORIES: [&str; 4] = ["plan", "guide", "query", "change"];

/// Valid workflow step kinds — keep in sync with `StepKind` in
/// `src/lib/workflow.ts`. The foundation kinds (`codebase`/`rag`/`knowledge`)
/// are the mandatory pre-phase steps pinned at the front of a workflow (D44).
pub const STEP_KINDS: [&str; 6] = ["search", "document", "chat", "codebase", "rag", "knowledge"];

/// Valid step output modes — keep in sync with `STEP_OUTPUTS` in
/// `src/lib/workflow.ts` (D47). Absent → frontend derives from kind.
pub const STEP_OUTPUTS: [&str; 3] = ["chat", "file", "html"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    /// Custom executable path (the per-agent `*_BIN` override equivalent).
    #[serde(default)]
    pub custom_bin: Option<String>,
}

/// One system skill: an instruction pack injected into the wire prompt on the
/// turn that runs a workflow step carrying it (see `StepDef.skill_ids`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub body: String,
    /// Optional resource folder (claude-skill style reference files/scripts).
    /// The frontend mentions the path in the wire prompt and passes it through
    /// `RunArgs.extraDirs` (claude `--add-dir`); no existence check here — a
    /// missing folder degrades gracefully at run time (D45).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// One step of a category's guided workflow.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepDef {
    pub id: String,
    /// Display name (settings editor + derived "N/M단계 · <name> 중…" notes).
    pub name: String,
    /// "search" | "document" | "chat". Kept as a plain string (not an enum):
    /// `load()` falls back to `Settings::default()` on any parse error, so one
    /// unknown enum value would silently wipe the whole settings file. Commands
    /// validate on save; the frontend coerces on read.
    pub kind: String,
    #[serde(default)]
    pub instruction: String,
    /// For a `document` step: the file the agent writes (workdir-relative).
    #[serde(default)]
    pub file: Option<String>,
    /// Skills injected on the turn that runs this step (dangling ids are
    /// tolerated — the runtime skips unknown ids, the editor warns).
    #[serde(default)]
    pub skill_ids: Vec<String>,
    /// Result form: "chat" | "file" | "html" (see `STEP_OUTPUTS`). Kept a plain
    /// string for the same no-wipe reason as `kind`. Absent → the frontend
    /// derives it from `kind` (document→"file", else "chat") — D47.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

/// Confluence crawl source for RAG ingestion (D48). The PAT is stored as plain
/// text in settings.json — acceptable for a local single-user desktop app;
/// recommend a read-only-scope token.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConfluenceConfig {
    /// Base URL including any context path, e.g. "https://wiki.example.com/confluence".
    #[serde(default)]
    pub base_url: String,
    /// Bearer PAT (Confluence Server/DC). Cloud Basic auth is out of scope v1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Crawl root page id — descendants are collected recursively (BFS).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_page_id: Option<String>,
    /// Alternative to `root_page_id`: flat listing of one space's pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_key: Option<String>,
    /// Opt-in escape hatch for corporate TLS-inspection proxies whose CA is not
    /// in the Windows store. Default false; the UI labels it dangerous.
    #[serde(default)]
    pub allow_invalid_certs: bool,
}

/// The user's own RAG service (summarization + embedding happen there; this app
/// only passes raw content and queries — see `rag.rs` for the fill-in stubs).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RagConfig {
    #[serde(default)]
    pub endpoint: String,
    /// Plain-text secret — same caveat as `ConfluenceConfig.token`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Search result count requested by the rag workflow step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Per-agent config, keyed by agent id ("opencode", "claude", ...).
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
    /// User-registered skill registry. `None` → the app's built-in defaults
    /// (which are also the editable sample content in the Flows view).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<SkillDef>>,
    /// Per-category workflow overrides. Absent key → built-in default flow.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub workflows: HashMap<String, Vec<StepDef>>,
    /// Confluence crawl source for RAG ingestion. `None` → not configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confluence: Option<ConfluenceConfig>,
    /// The user's RAG service endpoint. `None` → the rag workflow step skips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rag: Option<RagConfig>,
    /// Legacy v0.1 single-agent field. Folded into `agents` on load and never
    /// re-serialized (so the next `save` drops it — self-healing migration).
    #[serde(default, skip_serializing)]
    pub opencode_bin: Option<String>,
}

impl Settings {
    pub fn agent_custom_bin(&self, id: &str) -> Option<String> {
        self.agents.get(id).and_then(|c| c.custom_bin.clone())
    }

    /// Set (`Some`) or clear (`None`) the custom binary path for one agent.
    pub fn set_agent_bin(&mut self, id: &str, path: Option<String>) {
        match path {
            Some(p) => {
                self.agents.insert(id.to_string(), AgentConfig { custom_bin: Some(p) });
            }
            None => {
                self.agents.remove(id);
            }
        }
    }

    /// Replace the skill registry, or reset to built-in defaults with `None`.
    pub fn set_skills(&mut self, skills: Option<Vec<SkillDef>>) {
        self.skills = skills;
    }

    /// Replace one category's workflow, or reset to the default with `None`.
    pub fn set_workflow(&mut self, category: &str, steps: Option<Vec<StepDef>>) {
        match steps {
            Some(s) => {
                self.workflows.insert(category.to_string(), s);
            }
            None => {
                self.workflows.remove(category);
            }
        }
    }

    /// Set (`Some`) or clear (`None`) the Confluence crawl config.
    pub fn set_confluence(&mut self, config: Option<ConfluenceConfig>) {
        self.confluence = config;
    }

    /// Set (`Some`) or clear (`None`) the RAG endpoint config.
    pub fn set_rag(&mut self, config: Option<RagConfig>) {
        self.rag = config;
    }
}

/// Save-time validation for a user skill registry.
pub fn validate_skills(skills: &[SkillDef]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for s in skills {
        if s.id.trim().is_empty() {
            return Err("skill id must not be empty".into());
        }
        if s.name.trim().is_empty() {
            return Err(format!("skill '{}' needs a name", s.id));
        }
        if !seen.insert(s.id.trim()) {
            return Err(format!("duplicate skill id: {}", s.id));
        }
    }
    Ok(())
}

/// Save-time validation for one category's workflow steps.
pub fn validate_steps(steps: &[StepDef]) -> Result<(), String> {
    if steps.is_empty() {
        return Err("a workflow needs at least one step".into());
    }
    let mut seen = std::collections::HashSet::new();
    for s in steps {
        if s.id.trim().is_empty() {
            return Err("step id must not be empty".into());
        }
        if s.name.trim().is_empty() {
            return Err(format!("step '{}' needs a name", s.id));
        }
        if !STEP_KINDS.contains(&s.kind.as_str()) {
            return Err(format!("step '{}' has unknown kind: {}", s.id, s.kind));
        }
        if let Some(output) = s.output.as_deref() {
            if !STEP_OUTPUTS.contains(&output) {
                return Err(format!("step '{}' has unknown output: {}", s.id, output));
            }
        }
        if !seen.insert(s.id.trim()) {
            return Err(format!("duplicate step id: {}", s.id));
        }
    }
    if steps.last().map(|s| s.kind.as_str()) != Some("chat") {
        return Err("the last step must be a terminal 'chat' step".into());
    }
    Ok(())
}

fn file_path(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("settings.json")
}

pub fn load(config_dir: &Path) -> Settings {
    let mut s: Settings = match fs::read_to_string(file_path(config_dir)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Settings::default(),
    };
    // Migrate the legacy `opencodeBin` field into the per-agent map.
    if let Some(bin) = s.opencode_bin.take().filter(|b| !b.trim().is_empty()) {
        s.agents
            .entry("opencode".to_string())
            .or_insert(AgentConfig { custom_bin: Some(bin) });
    }
    s
}

pub fn save(config_dir: &Path, settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(file_path(config_dir), json).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ow-settings-test-{name}"))
    }

    fn skill(id: &str) -> SkillDef {
        SkillDef {
            id: id.into(),
            name: format!("{id} 스킬"),
            body: format!("[{id}] 지시문"),
            dir: None,
        }
    }

    fn step(id: &str, kind: &str) -> StepDef {
        StepDef {
            id: id.into(),
            name: format!("{id} 단계"),
            kind: kind.into(),
            instruction: String::new(),
            file: None,
            skill_ids: vec![],
            output: None,
        }
    }

    #[test]
    fn roundtrips_skills_and_workflows() {
        let root = temp_root("roundtrip");
        let _ = fs::remove_dir_all(&root);

        let mut s = Settings::default();
        s.set_agent_bin("opencode", Some("C:\\bin\\opencode.cmd".into()));
        s.set_skills(Some(vec![
            SkillDef { dir: Some("C:\\skills\\sa".into()), ..skill("source-analysis") },
            skill("test-plan"),
        ]));
        s.set_workflow(
            "plan",
            Some(vec![
                StepDef {
                    file: Some("docs/plan.md".into()),
                    output: Some("html".into()),
                    ..step("doc", "document")
                },
                step("codebase", "codebase"),
                step("rag", "rag"),
                step("knowledge", "knowledge"),
                step("chat", "chat"),
            ]),
        );
        s.set_confluence(Some(ConfluenceConfig {
            base_url: "https://wiki.example.com/confluence".into(),
            token: Some("pat".into()),
            root_page_id: Some("12345".into()),
            space_key: None,
            allow_invalid_certs: false,
        }));
        s.set_rag(Some(RagConfig {
            endpoint: "https://rag.example.com".into(),
            api_key: Some("key".into()),
            top_k: Some(5),
        }));
        save(&root, &s).unwrap();

        let loaded = load(&root);
        assert_eq!(loaded, s);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn legacy_agents_only_file_loads_clean() {
        let root = temp_root("legacy");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("settings.json"),
            r#"{ "agents": { "claude": { "customBin": "C:\\c.cmd" } }, "opencodeBin": "C:\\o.cmd" }"#,
        )
        .unwrap();

        let s = load(&root);
        assert_eq!(s.agent_custom_bin("claude").as_deref(), Some("C:\\c.cmd"));
        // Legacy field migrated, no flow config present → defaults apply.
        assert_eq!(s.agent_custom_bin("opencode").as_deref(), Some("C:\\o.cmd"));
        assert!(s.skills.is_none());
        assert!(s.workflows.is_empty());
        // New sections absent from an old file → None.
        assert!(s.confluence.is_none());
        assert!(s.rag.is_none());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn old_skill_and_step_json_load_without_new_fields() {
        // Raw pre-extension JSON: no `dir`, no `output`, no confluence/rag.
        let raw = r#"{
            "skills": [{ "id": "a", "name": "A", "body": "b" }],
            "workflows": { "plan": [
                { "id": "doc", "name": "문서", "kind": "document", "file": "docs/p.md", "skillIds": ["a"] },
                { "id": "chat", "name": "대화", "kind": "chat" }
            ]}
        }"#;
        let s: Settings = serde_json::from_str(raw).unwrap();
        assert_eq!(s.skills.as_ref().unwrap()[0].dir, None);
        assert_eq!(s.workflows["plan"][0].output, None);
        assert!(s.confluence.is_none() && s.rag.is_none());
    }

    #[test]
    fn validate_steps_rules() {
        // Valid: generative steps ending in a terminal chat.
        let ok = vec![step("search", "search"), step("doc", "document"), step("chat", "chat")];
        assert!(validate_steps(&ok).is_ok());

        assert!(validate_steps(&[]).is_err(), "empty workflow");
        assert!(
            validate_steps(&[step("a", "chat"), step("a", "chat")]).is_err(),
            "duplicate step id"
        );
        assert!(validate_steps(&[step("a", "wizardry")]).is_err(), "unknown kind");
        assert!(
            validate_steps(&[step("a", "search")]).is_err(),
            "last step must be chat"
        );
        let mut unnamed = step("a", "chat");
        unnamed.name = "  ".into();
        assert!(validate_steps(&[unnamed]).is_err(), "step needs a name");

        // Foundation kinds are valid, non-terminal steps.
        let foundation = vec![
            step("codebase", "codebase"),
            step("rag", "rag"),
            step("knowledge", "knowledge"),
            step("chat", "chat"),
        ];
        assert!(validate_steps(&foundation).is_ok());

        // Output mode: valid values pass, unknown values fail.
        let mut html = step("doc", "document");
        html.output = Some("html".into());
        assert!(validate_steps(&[html, step("chat", "chat")]).is_ok());
        let mut bad = step("doc", "document");
        bad.output = Some("popup".into());
        assert!(validate_steps(&[bad, step("chat2", "chat")]).is_err(), "unknown output");
    }

    #[test]
    fn validate_skills_rules() {
        assert!(validate_skills(&[skill("a"), skill("b")]).is_ok());
        assert!(validate_skills(&[skill("a"), skill("a")]).is_err(), "duplicate id");
        let mut empty_id = skill("x");
        empty_id.id = " ".into();
        assert!(validate_skills(&[empty_id]).is_err(), "empty id");
        let mut unnamed = skill("y");
        unnamed.name = String::new();
        assert!(validate_skills(&[unnamed]).is_err(), "empty name");
    }

    #[test]
    fn reset_semantics_remove_overrides() {
        let root = temp_root("reset");
        let _ = fs::remove_dir_all(&root);

        let mut s = Settings::default();
        s.set_skills(Some(vec![skill("a")]));
        s.set_workflow("plan", Some(vec![step("chat", "chat")]));
        s.set_confluence(Some(ConfluenceConfig { base_url: "https://w".into(), ..Default::default() }));
        s.set_rag(Some(RagConfig { endpoint: "https://r".into(), ..Default::default() }));
        save(&root, &s).unwrap();

        let mut loaded = load(&root);
        loaded.set_skills(None);
        loaded.set_workflow("plan", None);
        loaded.set_confluence(None);
        loaded.set_rag(None);
        save(&root, &loaded).unwrap();

        let reloaded = load(&root);
        assert!(reloaded.skills.is_none());
        assert!(reloaded.workflows.is_empty());
        assert!(reloaded.confluence.is_none());
        assert!(reloaded.rag.is_none());
        // The serialized file should not even contain the cleared fields.
        let raw = fs::read_to_string(root.join("settings.json")).unwrap();
        assert!(!raw.contains("\"skills\""));
        assert!(!raw.contains("\"workflows\""));
        assert!(!raw.contains("\"confluence\""));
        assert!(!raw.contains("\"rag\""));

        let _ = fs::remove_dir_all(&root);
    }
}

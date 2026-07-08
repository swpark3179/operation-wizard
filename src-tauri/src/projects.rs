//! Conversation persistence: projects and sessions on disk.
//!
//! Layout (Windows): `%USERPROFILE%\.operation-wizard\projects\<projectId>\`
//!   - `project.json`                       — project manifest
//!   - `workspace\`                          — default agent working folder (cwd + canvas root)
//!   - `sessions\<sessionId>\session.json`  — one conversation (metadata + messages)
//!
//! A **project is a distinct work unit**, not a folder: the `projectId` is
//! minted on the frontend (`crypto.randomUUID`), so every new chat/category is
//! its own project. Each project stores its own `workdir` (the agent cwd): by
//! default the project's own `workspace\` subfolder, or an external folder the
//! user picked on Home. Backend commands are keyed by `projectId`.
//!
//! Style mirrors `settings.rs` (core fns take `root: &Path` for testability; the
//! command wrappers pass the real root) and `files.rs` (plain `std::fs`,
//! `Result<_, String>`). No new Cargo deps: ids are minted on the frontend,
//! timestamps come from `std::time::SystemTime`, and messages are stored as
//! opaque `serde_json::Value` so the backend stays decoupled from the frontend
//! `ChatMessage` shape.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Project manifest (`project.json`). One project per work unit.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// Frontend-minted id (also the project folder name).
    pub id: String,
    /// The agent working folder (cwd): the project's own `workspace\` or an
    /// external folder the user picked.
    pub workdir: String,
    /// Human title — the originating prompt (or category label), from the frontend.
    pub title: String,
    /// The work category (`plan`/`guide`/`query`/`change`). `#[serde(default)]`
    /// keeps backward-compat with manifests written before this field existed.
    #[serde(default)]
    pub category: String,
    pub created_at: u64,
}

/// Session metadata (the header of `session.json`, also returned by `list_sessions`).
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    /// Persistence id (frontend-minted UUID; the session folder name).
    pub id: String,
    /// Human title — the first user prompt, truncated by the frontend.
    pub title: String,
    pub agent_id: String,
    pub model: String,
    pub category: String,
    /// The agent CLI session id for resume (claude UUID / codex thread id), or
    /// null for sessionless agents (gemini/aipro).
    #[serde(default)]
    pub cli_session_id: Option<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub message_count: u64,
}

/// A project row for the Home "recent" list: manifest + activity rollup.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: String,
    pub workdir: String,
    pub title: String,
    pub category: String,
    pub created_at: u64,
    /// Latest session's `updatedAt` (falls back to `createdAt` if no sessions).
    pub updated_at: u64,
    pub session_count: u64,
    /// Most-recently-updated session id, to open when the project is clicked.
    pub last_session_id: Option<String>,
}

/// A full stored session: metadata (flattened) + the opaque message array.
#[derive(Serialize, Deserialize, Clone)]
pub struct StoredSession {
    #[serde(flatten)]
    pub meta: SessionMeta,
    /// The frontend `ChatMessage[]`, stored verbatim as JSON.
    #[serde(default)]
    pub messages: Value,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Root: `%USERPROFILE%\.operation-wizard\projects` (Windows; same home
/// resolution the resolver uses). Parents are created lazily on write.
fn projects_root() -> Result<PathBuf, String> {
    let up = std::env::var("USERPROFILE").map_err(|_| "USERPROFILE not set".to_string())?;
    Ok(PathBuf::from(up).join(".operation-wizard").join("projects"))
}

/// Reject ids that could escape their parent dir (project or session folder name).
fn is_safe_id(id: &str) -> bool {
    !id.trim().is_empty() && !id.contains("..") && !id.chars().any(|c| c == '/' || c == '\\')
}

fn valid_project_id(id: &str) -> bool {
    is_safe_id(id)
}

fn valid_session_id(id: &str) -> bool {
    is_safe_id(id)
}

fn project_dir(root: &Path, project_id: &str) -> PathBuf {
    root.join(project_id)
}

fn sessions_dir(root: &Path, project_id: &str) -> PathBuf {
    project_dir(root, project_id).join("sessions")
}

fn session_dir(root: &Path, project_id: &str, session_id: &str) -> PathBuf {
    sessions_dir(root, project_id).join(session_id)
}

fn new_project(id: &str, workdir: &str, title: &str, category: &str) -> Project {
    Project {
        id: id.to_string(),
        workdir: workdir.to_string(),
        title: title.to_string(),
        category: category.to_string(),
        created_at: now_millis(),
    }
}

/// Ensure the project folder + manifest exist; returns the (existing or new)
/// manifest. Idempotent — an existing manifest is reused as-is (createdAt/title
/// preserved). `workdir_in` empty → the project runs in its own `workspace\`
/// subfolder (created here); otherwise the given external folder is stored.
fn ensure_project_at(
    root: &Path,
    id: &str,
    workdir_in: &str,
    title: &str,
    category: &str,
) -> Result<Project, String> {
    if !valid_project_id(id) {
        return Err(format!("invalid project id: {id:?}"));
    }
    let dir = project_dir(root, id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let manifest = dir.join("project.json");
    if let Ok(txt) = fs::read_to_string(&manifest) {
        if let Ok(p) = serde_json::from_str::<Project>(&txt) {
            return Ok(p);
        }
    }
    // Resolve the working folder: external if provided, else the project's own
    // workspace/ subfolder (kept separate from project.json/sessions/ so the
    // canvas file tree doesn't show persistence noise).
    let workdir = if workdir_in.trim().is_empty() {
        let ws = dir.join("workspace");
        fs::create_dir_all(&ws).map_err(|e| e.to_string())?;
        ws.to_string_lossy().into_owned()
    } else {
        workdir_in.trim().to_string()
    };
    let p = new_project(id, &workdir, title, category);
    let json = serde_json::to_string_pretty(&p).map_err(|e| e.to_string())?;
    fs::write(&manifest, json).map_err(|e| e.to_string())?;
    Ok(p)
}

/// Write one session. The project must already exist (the frontend calls
/// `ensure_project` before the first save); this only creates the session
/// folder and writes it. Does not create the manifest.
fn save_session_at(root: &Path, project_id: &str, mut session: StoredSession) -> Result<(), String> {
    if !valid_project_id(project_id) {
        return Err(format!("invalid project id: {project_id:?}"));
    }
    let sid = session.meta.id.trim().to_string();
    if !valid_session_id(&sid) {
        return Err(format!("invalid session id: {sid:?}"));
    }
    let dir = session_dir(root, project_id, &sid);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let now = now_millis();
    session.meta.id = sid;
    session.meta.updated_at = now;
    if session.meta.created_at == 0 {
        session.meta.created_at = now;
    }
    session.meta.message_count = session.messages.as_array().map(|a| a.len() as u64).unwrap_or(0);

    let json = serde_json::to_string_pretty(&session).map_err(|e| e.to_string())?;
    fs::write(dir.join("session.json"), json).map_err(|e| e.to_string())?;
    Ok(())
}

fn list_sessions_at(root: &Path, project_id: &str) -> Result<Vec<SessionMeta>, String> {
    let dir = sessions_dir(root, project_id);
    let rd = match fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return Ok(Vec::new()), // no project/sessions yet
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let f = entry.path().join("session.json");
        if let Ok(txt) = fs::read_to_string(&f) {
            if let Ok(meta) = serde_json::from_str::<SessionMeta>(&txt) {
                out.push(meta);
            }
        }
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(out)
}

fn load_session_at(root: &Path, project_id: &str, session_id: &str) -> Result<StoredSession, String> {
    if !valid_session_id(session_id) {
        return Err(format!("invalid session id: {session_id:?}"));
    }
    let f = session_dir(root, project_id, session_id).join("session.json");
    let txt = fs::read_to_string(&f).map_err(|e| e.to_string())?;
    serde_json::from_str::<StoredSession>(&txt).map_err(|e| e.to_string())
}

/// All projects with an activity rollup, newest-updated first.
fn list_projects_at(root: &Path) -> Result<Vec<ProjectSummary>, String> {
    let rd = match fs::read_dir(root) {
        Ok(rd) => rd,
        Err(_) => return Ok(Vec::new()), // no projects yet
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        // The folder name is the project id that sessions are keyed by (matches
        // both minted ids and legacy deterministic ids).
        let id = entry.file_name().to_string_lossy().into_owned();
        let manifest = entry.path().join("project.json");
        let project = match fs::read_to_string(&manifest) {
            Ok(txt) => match serde_json::from_str::<Project>(&txt) {
                Ok(p) => p,
                Err(_) => continue,
            },
            Err(_) => continue,
        };
        // Sessions are already sorted newest-first by list_sessions_at.
        let sessions = list_sessions_at(root, &id).unwrap_or_default();
        let latest = sessions.first();
        out.push(ProjectSummary {
            id,
            workdir: project.workdir,
            title: project.title,
            category: project.category,
            created_at: project.created_at,
            updated_at: latest.map(|s| s.updated_at).unwrap_or(project.created_at),
            session_count: sessions.len() as u64,
            last_session_id: latest.map(|s| s.id.clone()),
        });
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(out)
}

// ── Tauri commands (projectId-keyed; the frontend mints the id) ───────────────

/// Create the project folder + manifest (idempotent). `workdir` empty/None →
/// the project's own `workspace\` subfolder; otherwise the external folder.
#[tauri::command]
pub fn ensure_project(
    project_id: String,
    workdir: Option<String>,
    title: String,
    category: String,
) -> Result<Project, String> {
    let workdir = workdir.unwrap_or_default();
    ensure_project_at(&projects_root()?, &project_id, &workdir, &title, &category)
}

/// Write one session to disk (project must already exist via `ensure_project`).
#[tauri::command]
pub fn save_session(project_id: String, session: StoredSession) -> Result<(), String> {
    save_session_at(&projects_root()?, &project_id, session)
}

/// List a project's sessions (metadata only), newest-updated first. Empty if
/// the project has no sessions yet.
#[tauri::command]
pub fn list_sessions(project_id: String) -> Result<Vec<SessionMeta>, String> {
    list_sessions_at(&projects_root()?, &project_id)
}

/// Load one full session (metadata + messages).
#[tauri::command]
pub fn load_session(project_id: String, session_id: String) -> Result<StoredSession, String> {
    load_session_at(&projects_root()?, &project_id, &session_id)
}

/// List all projects for the Home "recent" list.
#[tauri::command]
pub fn list_projects() -> Result<Vec<ProjectSummary>, String> {
    list_projects_at(&projects_root()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(id: &str) -> SessionMeta {
        SessionMeta {
            id: id.to_string(),
            title: "hello".to_string(),
            agent_id: "claude".to_string(),
            model: "default".to_string(),
            category: "plan".to_string(),
            cli_session_id: Some("cli-abc".to_string()),
            created_at: 0,
            updated_at: 0,
            message_count: 0,
        }
    }

    #[test]
    fn rejects_bad_project_id() {
        let root = std::env::temp_dir().join("ow-test-badpid");
        assert!(ensure_project_at(&root, "../evil", "", "t", "plan").is_err());
        assert!(ensure_project_at(&root, "a/b", "", "t", "plan").is_err());
        assert!(ensure_project_at(&root, "", "", "t", "plan").is_err());
    }

    #[test]
    fn ensure_default_workdir_is_workspace_subfolder() {
        let root = std::env::temp_dir().join("ow-test-defaultwd");
        let _ = fs::remove_dir_all(&root);

        let p1 = ensure_project_at(&root, "proj-1", "", "First", "plan").unwrap();
        // Idempotent: second ensure reuses the manifest (title/createdAt kept).
        let p2 = ensure_project_at(&root, "proj-1", "", "Second", "guide").unwrap();
        assert_eq!(p1.id, p2.id);
        assert_eq!(p1.created_at, p2.created_at);
        assert_eq!(p2.title, "First"); // reused, not overwritten
        assert_eq!(p1.category, "plan");
        // Default workdir is the project's own workspace/ subfolder.
        let norm = p1.workdir.replace('/', "\\");
        assert!(norm.ends_with("proj-1\\workspace"), "got {}", p1.workdir);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ensure_external_workdir_passthrough() {
        let root = std::env::temp_dir().join("ow-test-extwd");
        let _ = fs::remove_dir_all(&root);
        let p = ensure_project_at(&root, "proj-x", "F:\\SHI\\myrepo", "t", "plan").unwrap();
        assert_eq!(p.workdir, "F:\\SHI\\myrepo");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn save_list_load_roundtrip() {
        let root = std::env::temp_dir().join("ow-test-roundtrip");
        let _ = fs::remove_dir_all(&root);

        ensure_project_at(&root, "proj-1", "", "t", "plan").unwrap();
        let session = StoredSession {
            meta: meta("sess-1"),
            messages: json!([{"role":"user","content":"hi"},{"role":"assistant","content":"yo"}]),
        };
        save_session_at(&root, "proj-1", session).unwrap();

        let metas = list_sessions_at(&root, "proj-1").unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, "sess-1");
        assert_eq!(metas[0].message_count, 2); // computed on save
        assert!(metas[0].updated_at > 0); // stamped
        assert!(metas[0].created_at > 0); // stamped from 0

        let loaded = load_session_at(&root, "proj-1", "sess-1").unwrap();
        assert_eq!(loaded.meta.cli_session_id.as_deref(), Some("cli-abc"));
        assert_eq!(loaded.messages.as_array().unwrap().len(), 2);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn list_sessions_missing_is_empty() {
        let root = std::env::temp_dir().join("ow-test-missing");
        let _ = fs::remove_dir_all(&root);
        assert!(list_sessions_at(&root, "nope").unwrap().is_empty());
    }

    #[test]
    fn rejects_bad_session_id() {
        let root = std::env::temp_dir().join("ow-test-badid");
        let bad = StoredSession { meta: meta("../evil"), messages: json!([]) };
        assert!(save_session_at(&root, "proj-x", bad).is_err());
    }

    #[test]
    fn list_projects_rolls_up() {
        let root = std::env::temp_dir().join("ow-test-projects");
        let _ = fs::remove_dir_all(&root);
        assert!(list_projects_at(&root).unwrap().is_empty()); // no root yet

        // Project alpha: two sessions. ensure_project first (save no longer
        // creates the manifest).
        ensure_project_at(&root, "alpha-1", "F:\\SHI\\alpha", "Alpha", "plan").unwrap();
        save_session_at(&root, "alpha-1", StoredSession { meta: meta("a1"), messages: json!([{"x":1}]) })
            .unwrap();
        save_session_at(&root, "alpha-1", StoredSession { meta: meta("a2"), messages: json!([{"x":1}, {"y":2}]) })
            .unwrap();

        // Project beta: one session (saved last → most recently updated). Sleep so
        // its updatedAt is strictly greater regardless of ms-clock granularity.
        std::thread::sleep(std::time::Duration::from_millis(5));
        ensure_project_at(&root, "beta-1", "F:\\SHI\\beta", "Beta", "guide").unwrap();
        save_session_at(&root, "beta-1", StoredSession { meta: meta("b1"), messages: json!([]) }).unwrap();

        let projects = list_projects_at(&root).unwrap();
        assert_eq!(projects.len(), 2);
        // Newest-updated first → beta (saved last).
        assert_eq!(projects[0].id, "beta-1");
        assert_eq!(projects[0].workdir, "F:\\SHI\\beta");
        assert_eq!(projects[0].category, "guide");
        assert_eq!(projects[0].session_count, 1);
        assert_eq!(projects[0].last_session_id.as_deref(), Some("b1"));

        let alpha = projects.iter().find(|p| p.id == "alpha-1").unwrap();
        assert_eq!(alpha.session_count, 2);
        assert_eq!(alpha.title, "Alpha"); // from the manifest
        assert_eq!(alpha.category, "plan");
        assert!(alpha.last_session_id.is_some()); // most-recent of a1/a2
        assert!(alpha.updated_at > 0);

        let _ = fs::remove_dir_all(&root);
    }
}

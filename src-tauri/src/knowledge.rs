//! Knowledge base: simple on-disk entries injected into the foundation phase's
//! knowledge step as prompt context (no RAG involvement — D48).
//!
//! Layout (Windows): `%USERPROFILE%\.operation-wizard\knowledge\<id>.json`,
//! one file per entry so CRUD never rewrites unrelated entries. Ids are minted
//! on the frontend (`crypto.randomUUID`); timestamps come from `SystemTime`.
//! Style mirrors `projects.rs` (core fns take `root: &Path` for testability,
//! command wrappers pass the real root, plain `std::fs`, `Result<_, String>`).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// One knowledge entry: how a past task was approached (situation, tables
/// consulted, access method, conventions). Injected verbatim (size-capped on
/// the frontend) into the knowledge workflow step.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEntry {
    /// Frontend-minted UUID (also the file stem).
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Root: `%USERPROFILE%\.operation-wizard\knowledge` (same home resolution as
/// `projects.rs`). Created lazily on write.
fn knowledge_root() -> Result<PathBuf, String> {
    let up = std::env::var("USERPROFILE").map_err(|_| "USERPROFILE not set".to_string())?;
    Ok(PathBuf::from(up).join(".operation-wizard").join("knowledge"))
}

/// Reject ids that could escape the knowledge dir (duplicated from
/// `projects.rs::is_safe_id` — kept local to match the codebase's
/// low-abstraction style).
fn is_safe_id(id: &str) -> bool {
    !id.trim().is_empty() && !id.contains("..") && !id.chars().any(|c| c == '/' || c == '\\')
}

fn entry_path(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{id}.json"))
}

/// All entries, newest-updated first. Missing root → empty (nothing saved yet).
fn list_knowledge_at(root: &Path) -> Result<Vec<KnowledgeEntry>, String> {
    let rd = match fs::read_dir(root) {
        Ok(rd) => rd,
        Err(_) => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(txt) = fs::read_to_string(&path) {
            if let Ok(e) = serde_json::from_str::<KnowledgeEntry>(&txt) {
                out.push(e);
            }
        }
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(out)
}

/// Upsert one entry: stamps `updatedAt` (and `createdAt` when 0 — i.e. new),
/// preserving `createdAt` across later saves of the same entry.
fn save_knowledge_at(root: &Path, mut entry: KnowledgeEntry) -> Result<KnowledgeEntry, String> {
    entry.id = entry.id.trim().to_string();
    if !is_safe_id(&entry.id) {
        return Err(format!("invalid knowledge id: {:?}", entry.id));
    }
    if entry.title.trim().is_empty() {
        return Err("knowledge entry needs a title".to_string());
    }
    let now = now_millis();
    entry.updated_at = now;
    if entry.created_at == 0 {
        // Preserve the original createdAt when the frontend didn't round-trip it.
        entry.created_at = match fs::read_to_string(entry_path(root, &entry.id))
            .ok()
            .and_then(|txt| serde_json::from_str::<KnowledgeEntry>(&txt).ok())
        {
            Some(prev) if prev.created_at > 0 => prev.created_at,
            _ => now,
        };
    }
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&entry).map_err(|e| e.to_string())?;
    fs::write(entry_path(root, &entry.id), json).map_err(|e| e.to_string())?;
    Ok(entry)
}

/// Delete one entry; deleting a missing entry is Ok (idempotent).
fn delete_knowledge_at(root: &Path, id: &str) -> Result<(), String> {
    if !is_safe_id(id) {
        return Err(format!("invalid knowledge id: {id:?}"));
    }
    match fs::remove_file(entry_path(root, id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

// ── Tauri commands ────────────────────────────────────────────────────────────

/// All knowledge entries (full bodies — they are prompt-injection-sized).
#[tauri::command]
pub fn list_knowledge() -> Result<Vec<KnowledgeEntry>, String> {
    list_knowledge_at(&knowledge_root()?)
}

/// Upsert one entry; returns it with stamped timestamps.
#[tauri::command]
pub fn save_knowledge(entry: KnowledgeEntry) -> Result<KnowledgeEntry, String> {
    save_knowledge_at(&knowledge_root()?, entry)
}

/// Delete one entry (idempotent).
#[tauri::command]
pub fn delete_knowledge(id: String) -> Result<(), String> {
    delete_knowledge_at(&knowledge_root()?, &id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ow-knowledge-test-{name}"))
    }

    fn entry(id: &str, title: &str) -> KnowledgeEntry {
        KnowledgeEntry {
            id: id.into(),
            title: title.into(),
            body: "테이블 X를 Y 방식으로 조회".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn save_list_delete_roundtrip() {
        let root = temp_root("roundtrip");
        let _ = fs::remove_dir_all(&root);

        assert!(list_knowledge_at(&root).unwrap().is_empty()); // no root yet

        let a = save_knowledge_at(&root, entry("a", "A")).unwrap();
        assert!(a.created_at > 0 && a.updated_at > 0);
        std::thread::sleep(std::time::Duration::from_millis(5));
        save_knowledge_at(&root, entry("b", "B")).unwrap();

        let list = list_knowledge_at(&root).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "b"); // newest-updated first
        assert_eq!(list[1].body, "테이블 X를 Y 방식으로 조회");

        delete_knowledge_at(&root, "a").unwrap();
        delete_knowledge_at(&root, "a").unwrap(); // idempotent
        assert_eq!(list_knowledge_at(&root).unwrap().len(), 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn upsert_preserves_created_at() {
        let root = temp_root("upsert");
        let _ = fs::remove_dir_all(&root);

        let first = save_knowledge_at(&root, entry("k", "v1")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = save_knowledge_at(&root, entry("k", "v2")).unwrap();
        assert_eq!(second.created_at, first.created_at);
        assert!(second.updated_at > first.updated_at);
        assert_eq!(list_knowledge_at(&root).unwrap()[0].title, "v2");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_bad_ids_and_empty_title() {
        let root = temp_root("bad");
        assert!(save_knowledge_at(&root, entry("../evil", "t")).is_err());
        assert!(save_knowledge_at(&root, entry("a/b", "t")).is_err());
        assert!(save_knowledge_at(&root, entry("", "t")).is_err());
        assert!(save_knowledge_at(&root, entry("ok", "  ")).is_err());
        assert!(delete_knowledge_at(&root, "../evil").is_err());
    }
}

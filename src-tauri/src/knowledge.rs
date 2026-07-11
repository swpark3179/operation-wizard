//! Knowledge base: simple on-disk entries injected into the foundation phase's
//! knowledge step as prompt context (no RAG involvement — D48).
//!
//! Layout (Windows): `%USERPROFILE%\.operation-wizard\knowledge\<id>.json`,
//! one file per entry so CRUD never rewrites unrelated entries. Ids are minted
//! on the frontend (`crypto.randomUUID`); timestamps come from `SystemTime`.
//!
//! Artifact entries (D59) additionally own a folder of copied workflow output
//! files at `knowledge\artifacts\<id>\`. The entry's `files` field lists the
//! copied names (step order) so injection can build an absolute-path index
//! without extra directory reads; the agent then reads full originals on
//! demand via extraDirs.
//!
//! Style mirrors `projects.rs` (core fns take `root: &Path` for testability,
//! command wrappers pass the real root, plain `std::fs`, `Result<_, String>`).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Valid entry kinds. Plain strings (not a Rust enum) so an unknown value in
/// one file can never break deserialization — same convention as
/// `settings.rs::STEP_KINDS` (validated on save, coerced on read).
const KNOWLEDGE_KINDS: &[&str] = &["note", "artifact"];

/// Per-file guard for artifact copies — an accident stop, not a quota. The
/// frontend's `read_file` 2 MiB cap does not apply here (copies are pure
/// backend `fs::copy`, never round-tripped through the webview).
const MAX_ARTIFACT_FILE: u64 = 10 * 1024 * 1024;

/// One knowledge entry: how a past task was approached (situation, tables
/// consulted, access method, conventions). Injected verbatim (size-capped on
/// the frontend) into the knowledge workflow step.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEntry {
    /// Frontend-minted UUID (also the file stem).
    pub id: String,
    pub title: String,
    /// Free text for notes; the injected SUMMARY for artifact entries (full
    /// documents live in `artifacts/<id>/`, never inlined — D59).
    #[serde(default)]
    pub body: String,
    /// `"note"` (직접 작성) | `"artifact"` (워크플로우 산출물 저장 — D59).
    #[serde(default = "default_kind")]
    pub kind: String,
    /// Artifact entries: copied file names inside `artifacts/<id>/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Provenance of an artifact entry (origin project) — display only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

fn default_kind() -> String {
    "note".to_string()
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

/// Copied-files folder of one artifact entry.
fn artifacts_dir(root: &Path, id: &str) -> PathBuf {
    root.join("artifacts").join(id)
}

/// Staging folder for the copy-then-swap in `save_knowledge_files_at`.
fn artifacts_tmp_dir(root: &Path, id: &str) -> PathBuf {
    root.join("artifacts").join(format!("{id}.tmp"))
}

fn remove_dir_if_exists(dir: &Path) -> Result<(), String> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("산출물 폴더 정리 실패 — {e}")),
    }
}

/// Last path segment of a source path. Splits on both separators — the
/// frontend sends Windows paths, tests run anywhere.
fn dest_name(source: &str) -> Result<String, String> {
    let base = source
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim();
    if base.is_empty() || base == "." || base == ".." {
        return Err(format!("invalid artifact source path: {source:?}"));
    }
    Ok(base.to_string())
}

/// Collision-free destination name (`plan.md` → `plan-2.md`, `plan-3.md`, …).
/// `taken` holds lowercased names — one folder on a case-insensitive FS.
fn unique_name(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    let (stem, ext) = match base.rfind('.') {
        Some(i) if i > 0 => base.split_at(i),
        _ => (base, ""),
    };
    let mut n = 2u32;
    loop {
        let cand = format!("{stem}-{n}{ext}");
        if !taken.contains(&cand.to_lowercase()) {
            return cand;
        }
        n += 1;
    }
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
    // Kind: empty → "note" (old frontends never send it); unknown → reject
    // (validate-on-save, never an enum — see KNOWLEDGE_KINDS).
    entry.kind = entry.kind.trim().to_string();
    if entry.kind.is_empty() {
        entry.kind = default_kind();
    }
    if !KNOWLEDGE_KINDS.contains(&entry.kind.as_str()) {
        return Err(format!("invalid knowledge kind: {:?}", entry.kind));
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

/// Copy `sources` into `artifacts/<id>/` and upsert the entry as an artifact
/// entry whose `files` lists the copied names (D59). The copy is a staged
/// swap through `artifacts/<id>.tmp`: a mid-copy failure never destroys the
/// previous version's files, and the entry JSON is only written after the new
/// file set is fully in place (the entry never references uncopied files).
fn save_knowledge_files_at(
    root: &Path,
    mut entry: KnowledgeEntry,
    sources: Vec<String>,
) -> Result<KnowledgeEntry, String> {
    entry.id = entry.id.trim().to_string();
    if !is_safe_id(&entry.id) {
        return Err(format!("invalid knowledge id: {:?}", entry.id));
    }
    if entry.title.trim().is_empty() {
        return Err("knowledge entry needs a title".to_string());
    }

    // Validate every source up front — fail before touching the store.
    let mut plan: Vec<(&String, String)> = Vec::new();
    let mut taken: HashSet<String> = HashSet::new();
    for source in &sources {
        let meta = fs::metadata(source)
            .map_err(|e| format!("산출물 파일을 읽을 수 없습니다: {source} — {e}"))?;
        if !meta.is_file() {
            return Err(format!("산출물 경로가 파일이 아닙니다: {source}"));
        }
        if meta.len() > MAX_ARTIFACT_FILE {
            return Err(format!("산출물 파일이 너무 큽니다(최대 10 MiB): {source}"));
        }
        let name = unique_name(&dest_name(source)?, &taken);
        taken.insert(name.to_lowercase());
        plan.push((source, name));
    }

    // Stage into `<id>.tmp`, then swap into place.
    let dir = artifacts_dir(root, &entry.id);
    let tmp = artifacts_tmp_dir(root, &entry.id);
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let mut copied: Vec<String> = Vec::new();
    for (source, name) in &plan {
        if let Err(e) = fs::copy(source, tmp.join(name)) {
            let _ = fs::remove_dir_all(&tmp);
            return Err(format!("산출물 복사 실패: {source} — {e}"));
        }
        copied.push(name.clone());
    }
    if let Err(e) = remove_dir_if_exists(&dir) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, &dir) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(format!("산출물 폴더 교체 실패 — {e}"));
    }

    entry.kind = "artifact".to_string();
    entry.files = copied;
    save_knowledge_at(root, entry)
}

/// Delete one entry; deleting a missing entry is Ok (idempotent). An artifact
/// entry's copied-files folder (and any stray staging dir) goes with it.
fn delete_knowledge_at(root: &Path, id: &str) -> Result<(), String> {
    if !is_safe_id(id) {
        return Err(format!("invalid knowledge id: {id:?}"));
    }
    match fs::remove_file(entry_path(root, id)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    remove_dir_if_exists(&artifacts_dir(root, id))?;
    remove_dir_if_exists(&artifacts_tmp_dir(root, id))?;
    Ok(())
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

/// Copy workflow output files into the entry's artifact folder and upsert the
/// entry as kind "artifact" (D59). `sources` are absolute paths (workdir-joined
/// on the frontend); returns the entry with `files` set to the copied names.
#[tauri::command]
pub fn save_knowledge_files(
    entry: KnowledgeEntry,
    sources: Vec<String>,
) -> Result<KnowledgeEntry, String> {
    save_knowledge_files_at(&knowledge_root()?, entry, sources)
}

/// Absolute knowledge root path — the frontend joins `artifacts\<id>\<name>`
/// for the injection index and extraDirs grant (D59). Does not create it.
#[tauri::command]
pub fn get_knowledge_root() -> Result<String, String> {
    Ok(knowledge_root()?.to_string_lossy().to_string())
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
            kind: String::new(),
            files: Vec::new(),
            source_project_id: None,
            source_category: None,
            source_title: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    /// Temp source file for artifact-copy tests; returns its path as String.
    fn source_file(dir: &Path, name: &str, content: &str) -> String {
        fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, content).unwrap();
        p.to_string_lossy().to_string()
    }

    #[test]
    fn save_list_delete_roundtrip() {
        let root = temp_root("roundtrip");
        let _ = fs::remove_dir_all(&root);

        assert!(list_knowledge_at(&root).unwrap().is_empty()); // no root yet

        let a = save_knowledge_at(&root, entry("a", "A")).unwrap();
        assert!(a.created_at > 0 && a.updated_at > 0);
        assert_eq!(a.kind, "note"); // empty kind normalized
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

    #[test]
    fn kind_validated_on_save() {
        let root = temp_root("kind");
        let _ = fs::remove_dir_all(&root);

        let mut bad = entry("k", "t");
        bad.kind = "weird".into();
        assert!(save_knowledge_at(&root, bad).is_err());

        let mut art = entry("k", "t");
        art.kind = " artifact ".into(); // trimmed, then accepted
        assert_eq!(save_knowledge_at(&root, art).unwrap().kind, "artifact");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn old_json_without_new_fields_loads_as_note() {
        let root = temp_root("compat");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("legacy.json"),
            r#"{ "id": "legacy", "title": "old", "body": "b", "createdAt": 1, "updatedAt": 2 }"#,
        )
        .unwrap();

        let list = list_knowledge_at(&root).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].kind, "note");
        assert!(list[0].files.is_empty());
        assert!(list[0].source_project_id.is_none());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn artifact_save_roundtrip() {
        let root = temp_root("artifact-roundtrip");
        let _ = fs::remove_dir_all(&root);
        let src = temp_root("artifact-roundtrip-src");
        let _ = fs::remove_dir_all(&src);
        let a = source_file(&src, "plan.md", "# 계획");
        let b = source_file(&src, "plan.html", "<h1>계획</h1>");

        let mut e = entry("art", "작업 정리");
        e.source_category = Some("plan".into());
        let saved = save_knowledge_files_at(&root, e, vec![a, b]).unwrap();
        assert_eq!(saved.kind, "artifact");
        assert_eq!(saved.files, vec!["plan.md", "plan.html"]);

        let dir = artifacts_dir(&root, "art");
        assert_eq!(fs::read_to_string(dir.join("plan.md")).unwrap(), "# 계획");
        assert_eq!(fs::read_to_string(dir.join("plan.html")).unwrap(), "<h1>계획</h1>");

        // The `artifacts` dir must not pollute the entry listing.
        let list = list_knowledge_at(&root).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].files, vec!["plan.md", "plan.html"]);
        assert_eq!(list[0].source_category.as_deref(), Some("plan"));

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn artifact_basename_dedupe() {
        let root = temp_root("artifact-dedupe");
        let _ = fs::remove_dir_all(&root);
        let src = temp_root("artifact-dedupe-src");
        let _ = fs::remove_dir_all(&src);
        let a = source_file(&src.join("one"), "plan.md", "1");
        let b = source_file(&src.join("two"), "plan.md", "2");

        let saved = save_knowledge_files_at(&root, entry("art", "t"), vec![a, b]).unwrap();
        assert_eq!(saved.files, vec!["plan.md", "plan-2.md"]);
        let dir = artifacts_dir(&root, "art");
        assert_eq!(fs::read_to_string(dir.join("plan.md")).unwrap(), "1");
        assert_eq!(fs::read_to_string(dir.join("plan-2.md")).unwrap(), "2");

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn artifact_source_errors() {
        let root = temp_root("artifact-errors");
        let _ = fs::remove_dir_all(&root);
        let src = temp_root("artifact-errors-src");
        let _ = fs::remove_dir_all(&src);
        fs::create_dir_all(&src).unwrap();

        // Missing source.
        let missing = src.join("nope.md").to_string_lossy().to_string();
        assert!(save_knowledge_files_at(&root, entry("a", "t"), vec![missing]).is_err());
        // Directory as source.
        let dir_src = src.to_string_lossy().to_string();
        assert!(save_knowledge_files_at(&root, entry("a", "t"), vec![dir_src]).is_err());
        // Oversize source.
        let big = src.join("big.bin");
        fs::write(&big, vec![0u8; (MAX_ARTIFACT_FILE + 1) as usize]).unwrap();
        let big = big.to_string_lossy().to_string();
        assert!(save_knowledge_files_at(&root, entry("a", "t"), vec![big]).is_err());
        // Nothing was persisted by the failed saves.
        assert!(list_knowledge_at(&root).unwrap().is_empty());
        assert!(!artifacts_dir(&root, "a").exists());

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn artifact_resave_swaps_file_set() {
        let root = temp_root("artifact-swap");
        let _ = fs::remove_dir_all(&root);
        let src = temp_root("artifact-swap-src");
        let _ = fs::remove_dir_all(&src);
        let a = source_file(&src, "a.md", "a");
        let b = source_file(&src, "b.md", "b");
        let c = source_file(&src, "c.md", "c");

        let first = save_knowledge_files_at(&root, entry("art", "t"), vec![a.clone(), b]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = save_knowledge_files_at(&root, entry("art", "t"), vec![c]).unwrap();
        assert_eq!(second.files, vec!["c.md"]);
        assert_eq!(second.created_at, first.created_at); // upsert semantics

        let dir = artifacts_dir(&root, "art");
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["c.md"]); // old set fully replaced

        // A failed re-save (bad source) must keep the current set intact.
        let missing = src.join("nope.md").to_string_lossy().to_string();
        assert!(save_knowledge_files_at(&root, entry("art", "t"), vec![a, missing]).is_err());
        assert_eq!(fs::read_to_string(dir.join("c.md")).unwrap(), "c");
        assert_eq!(list_knowledge_at(&root).unwrap()[0].files, vec!["c.md"]);

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn delete_removes_artifact_folder() {
        let root = temp_root("artifact-delete");
        let _ = fs::remove_dir_all(&root);
        let src = temp_root("artifact-delete-src");
        let _ = fs::remove_dir_all(&src);
        let a = source_file(&src, "a.md", "a");

        save_knowledge_files_at(&root, entry("art", "t"), vec![a]).unwrap();
        assert!(artifacts_dir(&root, "art").exists());
        delete_knowledge_at(&root, "art").unwrap();
        assert!(!artifacts_dir(&root, "art").exists());
        assert!(list_knowledge_at(&root).unwrap().is_empty());
        delete_knowledge_at(&root, "art").unwrap(); // idempotent

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&src);
    }
}

//! Per-codebase Learnings cache (D90): reuse a code analysis across projects.
//!
//! The orientation step's `## Learnings for Next Steps` (D86) is saved under
//! `~/.operation-wizard/codebases/<key>/learnings-<category>.md` — never in
//! the codebase folder itself (it stays read-only). `<key>` is an FNV-1a hash
//! of the normalized path; `meta.json` keeps the original path (checked on
//! load, so a hash collision can't serve another codebase's notes) and, per
//! category, when and from which project the notes were saved. On load the
//! codebase is walked for files modified since then (the freshness signal the
//! agent uses to decide what to re-check), within a short deadline.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::files::{walk_files_with, WALK_MAX};

/// Cap on the stored Learnings text.
const TEXT_MAX: usize = 6000;
/// How many changed files are named in the freshness note.
const SAMPLE_MAX: usize = 8;
/// Freshness walk budget.
const FRESHNESS_DEADLINE: Duration = Duration::from_secs(3);
/// Build output is not "a change to the code".
const SKIP: &[&str] = &["build", "out", "dist", "classes", "generated", "logs"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MetaEntry {
    updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_project_id: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    codebase_path: String,
    #[serde(default)]
    entries: BTreeMap<String, MetaEntry>,
}

/// Cached Learnings + how stale they are.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodebaseLearnings {
    pub text: String,
    /// Epoch ms of the save.
    pub updated_at: u64,
    /// Files modified after `updated_at` (a lower bound when `changed_capped`).
    pub changed_count: usize,
    /// A few of those files (codebase-relative, `/`-separated).
    pub changed_sample: Vec<String>,
    /// The walk hit its deadline / entry cap — the count is partial.
    pub changed_capped: bool,
    pub source_project_id: Option<String>,
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Case/separator-insensitive form of a folder path (Windows paths are
/// case-insensitive; a trailing separator doesn't make a new codebase).
fn normalize(path: &str) -> String {
    path.trim().replace('\\', "/").trim_end_matches('/').to_lowercase()
}

fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn cache_dir(root: &Path, codebase: &str) -> PathBuf {
    root.join("codebases").join(format!("{:016x}", fnv1a64(&normalize(codebase))))
}

fn check_category(category: &str) -> Result<(), String> {
    if crate::settings::CATEGORIES.contains(&category) {
        Ok(())
    } else {
        Err(format!("알 수 없는 카테고리입니다: {category}"))
    }
}

fn read_meta(dir: &Path) -> Option<Meta> {
    serde_json::from_slice(&fs::read(dir.join("meta.json")).ok()?).ok()
}

/// Save (replace) the Learnings of one category for a codebase.
pub fn save_at(root: &Path, codebase: &str, category: &str, text: &str, project_id: Option<&str>, now_ms: u64) -> Result<(), String> {
    check_category(category)?;
    let text = text.trim();
    if codebase.trim().is_empty() || text.is_empty() {
        return Err("코드베이스 경로와 Learnings 내용이 필요합니다.".into());
    }
    let dir = cache_dir(root, codebase);
    let norm = normalize(codebase);
    let mut meta = read_meta(&dir).unwrap_or_default();
    if !meta.codebase_path.is_empty() && normalize(&meta.codebase_path) != norm {
        return Err("캐시 키 충돌 — 다른 코드베이스의 캐시가 같은 위치에 있습니다.".into());
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let body: String = text.chars().take(TEXT_MAX).collect();
    fs::write(dir.join(format!("learnings-{category}.md")), body).map_err(|e| e.to_string())?;
    meta.codebase_path = codebase.trim().to_string();
    meta.entries.insert(
        category.to_string(),
        MetaEntry { updated_at: now_ms, source_project_id: project_id.map(str::to_string).filter(|s| !s.is_empty()) },
    );
    let json = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
    fs::write(dir.join("meta.json"), json).map_err(|e| e.to_string())
}

/// The cached Learnings of one category for a codebase (+ freshness), or
/// `None` when nothing (valid) is cached.
pub fn get_at(root: &Path, codebase: &str, category: &str, deadline: Duration) -> Result<Option<CodebaseLearnings>, String> {
    check_category(category)?;
    let dir = cache_dir(root, codebase);
    let Some(meta) = read_meta(&dir) else { return Ok(None) };
    if normalize(&meta.codebase_path) != normalize(codebase) {
        return Ok(None); // hash collision — not ours
    }
    let Some(entry) = meta.entries.get(category) else { return Ok(None) };
    let Ok(text) = fs::read_to_string(dir.join(format!("learnings-{category}.md"))) else { return Ok(None) };
    if text.trim().is_empty() {
        return Ok(None);
    }

    // Freshness: files modified after the save.
    let base = Path::new(codebase.trim());
    let started = Instant::now();
    let mut changed_count = 0usize;
    let mut changed_sample: Vec<String> = Vec::new();
    let mut capped = false;
    if base.is_dir() {
        let stop = walk_files_with(base, &AtomicBool::new(false), WALK_MAX, &|n| SKIP.contains(&n), &mut |p, _| {
            let modified = fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if modified > entry.updated_at {
                changed_count += 1;
                if changed_sample.len() < SAMPLE_MAX {
                    if let Ok(rel) = p.strip_prefix(base) {
                        changed_sample.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
            started.elapsed() < deadline
        });
        capped = !matches!(stop, crate::files::WalkStop::Done);
    }
    Ok(Some(CodebaseLearnings {
        text,
        updated_at: entry.updated_at,
        changed_count,
        changed_sample,
        changed_capped: capped,
        source_project_id: entry.source_project_id.clone(),
    }))
}

/// Cached Learnings for the codebase-step preflight (D90). Off the IPC thread.
#[tauri::command]
pub async fn get_codebase_learnings(codebase_path: String, category: String) -> Result<Option<CodebaseLearnings>, String> {
    let root = crate::ow_home()?;
    tauri::async_runtime::spawn_blocking(move || get_at(&root, &codebase_path, &category, FRESHNESS_DEADLINE))
        .await
        .map_err(|e| e.to_string())?
}

/// Save an orientation step's Learnings for later projects on the same codebase.
#[tauri::command]
pub fn save_codebase_learnings(
    codebase_path: String,
    category: String,
    text: String,
    project_id: Option<String>,
) -> Result<(), String> {
    let root = crate::ow_home()?;
    save_at(&root, &codebase_path, &category, &text, project_id.as_deref(), now_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ow-cbcache-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn save_then_get_roundtrip_with_freshness() {
        let home = temp("home");
        let code = temp("code");
        fs::create_dir_all(code.join("src")).unwrap();
        fs::write(code.join("src").join("A.java"), "class A {}").unwrap();
        fs::create_dir_all(code.join("build")).unwrap();
        fs::write(code.join("build").join("Gen.java"), "class Gen {}").unwrap();
        let cb = code.to_string_lossy().into_owned();

        // Saved "long ago" → every source file counts as changed (build/ skipped).
        save_at(&home, &cb, "plan", "  - [Architecture Pattern]: 레이어드  ", Some("p1"), 1).unwrap();
        let got = get_at(&home, &cb, "plan", Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(got.text, "- [Architecture Pattern]: 레이어드");
        assert_eq!((got.updated_at, got.source_project_id.as_deref()), (1, Some("p1")));
        assert_eq!((got.changed_count, got.changed_capped), (1, false));
        assert_eq!(got.changed_sample, vec!["src/A.java".to_string()]);

        // Saved in the future → nothing changed since.
        save_at(&home, &cb, "plan", "v2", None, u64::MAX / 2).unwrap();
        let fresh = get_at(&home, &cb, "plan", Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!((fresh.text.as_str(), fresh.changed_count, fresh.source_project_id), ("v2", 0, None));

        // Categories are separate; unknown ones are rejected.
        assert!(get_at(&home, &cb, "query", Duration::from_secs(1)).unwrap().is_none());
        assert!(save_at(&home, &cb, "../evil", "x", None, 1).is_err());
        assert!(get_at(&home, &cb, "../evil", Duration::from_secs(1)).is_err());
        assert!(save_at(&home, &cb, "plan", "   ", None, 1).is_err());
        let _ = fs::remove_dir_all(&home);
        let _ = fs::remove_dir_all(&code);
    }

    #[test]
    fn path_normalization_and_collision_guard() {
        assert_eq!(normalize(r"C:\Work\Order\ "), normalize("c:/work/order"));
        assert_eq!(cache_dir(Path::new("/h"), r"C:\Work\Order\"), cache_dir(Path::new("/h"), "c:/work/order"));
        let home = temp("collide");
        let dir = cache_dir(&home, "/code/a");
        fs::create_dir_all(&dir).unwrap();
        // A meta for another codebase sitting at our key: never served, never overwritten.
        fs::write(dir.join("meta.json"), r#"{"codebasePath":"/code/other","entries":{"plan":{"updatedAt":1}}}"#).unwrap();
        fs::write(dir.join("learnings-plan.md"), "theirs").unwrap();
        assert!(get_at(&home, "/code/a", "plan", Duration::from_secs(1)).unwrap().is_none());
        assert!(save_at(&home, "/code/a", "plan", "ours", None, 2).unwrap_err().contains("충돌"));
        assert_eq!(fs::read_to_string(dir.join("learnings-plan.md")).unwrap(), "theirs");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn text_is_capped() {
        let home = temp("cap");
        let long = "가".repeat(TEXT_MAX + 50);
        save_at(&home, "/nowhere/code", "change", &long, None, 5).unwrap();
        let got = get_at(&home, "/nowhere/code", "change", Duration::from_secs(1)).unwrap().unwrap();
        assert_eq!(got.text.chars().count(), TEXT_MAX);
        assert_eq!((got.changed_count, got.changed_capped), (0, false), "missing codebase folder → no walk");
        let _ = fs::remove_dir_all(&home);
    }
}

//! Filesystem helpers for the canvas file viewer: list a directory, read a
//! file's text, and write a file. Implemented as plain Tauri commands (using
//! `std::fs`) instead of pulling in the `fs` plugin + its capability surface.
//! `list_dir`/`read_file` are read-only; `write_file` (D67) persists a remote
//! agent's streamed document output — the remote FabriX agent can only *read*
//! files (through the app's read-only tool loop, D84), so the app writes its
//! generative-step text to `step.file` itself, matching the artifacts local CLI
//! agents write with their own tools.

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// Heavy/noise directories hidden from the canvas tree and skipped by the
/// FabriX read-only file tools (D84) and the code map (D88) — shared so they
/// stay in sync. `.svn` matters for legacy checkouts: its pristine copies would
/// otherwise double every search hit.
pub(crate) const SKIP_DIRS: [&str; 8] = [".git", ".svn", "node_modules", "target", ".next", ".idea", ".gradle", ".vs"];

/// Upper bound on directory entries visited by one recursive walk.
pub(crate) const WALK_MAX: usize = 50_000;

/// Why a recursive walk stopped.
#[derive(Debug, PartialEq)]
pub(crate) enum WalkStop {
    Done,
    /// Visited the entry cap.
    Limit,
    Canceled,
    /// The visitor asked to stop (result cap / deadline reached).
    Visitor,
}

/// Depth-first walk of `root` (alphabetical), calling `visit(file, size)` for
/// every file. Skips symlinks/junctions and the shared noise dirs; polls the
/// cancel flag per directory.
pub(crate) fn walk_files(root: &Path, cancel: &AtomicBool, visit: &mut dyn FnMut(&Path, u64) -> bool) -> WalkStop {
    walk_files_with(root, cancel, WALK_MAX, &|_| false, visit)
}

/// [`walk_files`] with an entry cap and an extra directory-name filter
/// (`skip_dir(name) == true` → not entered) on top of `SKIP_DIRS`.
pub(crate) fn walk_files_with(
    root: &Path,
    cancel: &AtomicBool,
    max_entries: usize,
    skip_dir: &dyn Fn(&str) -> bool,
    visit: &mut dyn FnMut(&Path, u64) -> bool,
) -> WalkStop {
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return WalkStop::Canceled;
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let mut entries: Vec<fs::DirEntry> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());
        let mut subdirs = Vec::new();
        for e in entries {
            seen += 1;
            if seen > max_entries {
                return WalkStop::Limit;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !SKIP_DIRS.contains(&name.as_str()) && !skip_dir(&name) {
                    subdirs.push(e.path());
                }
                continue;
            }
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            if !visit(&e.path(), size) {
                return WalkStop::Visitor;
            }
        }
        // Reverse so the alphabetically-first subdirectory is walked next.
        stack.extend(subdirs.into_iter().rev());
    }
    WalkStop::Done
}

/// A NUL byte in the first 8 KiB marks a file as binary (skipped / refused).
pub(crate) fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(8192)].contains(&0)
}

/// `.env` / `.env.*` — secrets never read or analyzed.
pub(crate) fn is_secret_name(name: &str) -> bool {
    let name = name.to_lowercase();
    name == ".env" || name.starts_with(".env.")
}

/// List a directory's immediate children, directories first then files, each
/// alphabetical (case-insensitive). Common heavy/noise dirs are skipped.
#[tauri::command]
pub fn list_dir(path: String) -> Result<Vec<FileEntry>, String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(&path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        entries.push(FileEntry {
            name,
            path: entry.path().to_string_lossy().into_owned(),
            is_dir,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// Max file size the viewer will read (2 MiB); larger files are rejected.
const MAX_READ: u64 = 2 * 1024 * 1024;

/// Read a text file's contents (UTF-8, lossy). Errors on missing/too-large/
/// non-text files.
#[tauri::command]
pub fn read_file(path: String) -> Result<String, String> {
    let meta = fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_READ {
        return Err(format!("file too large to preview ({} bytes)", meta.len()));
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Max content size `write_file` will persist (5 MiB); larger writes rejected.
const MAX_WRITE: usize = 5 * 1024 * 1024;

/// Write `contents` (UTF-8) to `path`, creating parent directories as needed
/// (D67). Used by the client to persist a remote agent's document-step output
/// (FabriX can only read files via the app's read-only tool loop — D84). The path is supplied
/// by the frontend as `<workdir>/<step.file>` — the same location a local CLI
/// agent would write with its own tools.
#[tauri::command]
pub fn write_file(path: String, contents: String) -> Result<(), String> {
    if contents.len() > MAX_WRITE {
        return Err(format!("content too large to write ({} bytes)", contents.len()));
    }
    if let Some(parent) = Path::new(&path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    fs::write(&path, contents).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_creates_parent_dirs_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!("ow_write_test_{}", std::process::id()));
        let file = dir.join("docs").join("plan.md");
        let path = file.to_string_lossy().into_owned();
        // Parent (docs/) does not exist yet — write_file must create it.
        write_file(path.clone(), "# 계획\n본문".to_string()).unwrap();
        assert_eq!(read_file(path).unwrap(), "# 계획\n본문");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_rejects_oversized_content() {
        let dir = std::env::temp_dir().join(format!("ow_write_big_{}", std::process::id()));
        let path = dir.join("big.txt").to_string_lossy().into_owned();
        let too_big = "a".repeat(MAX_WRITE + 1);
        assert!(write_file(path, too_big).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}

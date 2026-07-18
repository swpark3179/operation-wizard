//! Filesystem helpers for the canvas file viewer: list a directory, read a
//! file's text, and write a file. Implemented as plain Tauri commands (using
//! `std::fs`) instead of pulling in the `fs` plugin + its capability surface.
//! `list_dir`/`read_file` are read-only; `write_file` (D67) persists a remote
//! agent's streamed document output — remote agents (Fabrix) have no filesystem
//! access, so the app writes their generative-step text to `step.file` itself,
//! matching the artifacts local CLI agents write with their own tools.

use std::fs;
use std::path::Path;

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// List a directory's immediate children, directories first then files, each
/// alphabetical (case-insensitive). Common heavy/noise dirs are skipped.
#[tauri::command]
pub fn list_dir(path: String) -> Result<Vec<FileEntry>, String> {
    const SKIP: [&str; 4] = [".git", "node_modules", "target", ".next"];

    let mut entries = Vec::new();
    for entry in fs::read_dir(&path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && SKIP.contains(&name.as_str()) {
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
/// (Fabrix streams text but cannot touch the filesystem). The path is supplied
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

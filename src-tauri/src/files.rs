//! Filesystem helpers for the canvas file viewer: list a directory and read a
//! file's text. Implemented as plain Tauri commands (using `std::fs`) instead of
//! pulling in the `fs` plugin + its capability surface — the canvas only needs
//! these two read-only operations.

use std::fs;

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

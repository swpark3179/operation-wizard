//! Executable resolution for CLI agents on Windows.
//!
//! Mirrors Open Design's `inspectAgentExecutableResolution`
//! (`apps/daemon/src/runtimes/executables.ts`) and the Windows toolchain-bin
//! augmentation from `packages/platform/src/index.ts` (`wellKnownUserToolchainBins`).
//!
//! Resolution priority (per agent definition):
//!   1. custom path  (persisted setting, or the agent's `*_BIN` env override)
//!   2. PATH + well-known toolchain bin directories, scanned with PATHEXT.

use std::path::PathBuf;

use crate::agents::AgentDef;

pub struct Resolved {
    pub path: String,
    /// "custom-path" when it came from a setting / env override, else "path".
    pub source: String,
}

/// Resolve an agent's executable, or `None` if it cannot be found.
pub fn resolve_agent(def: &AgentDef, custom: Option<&str>) -> Option<Resolved> {
    // 1. Explicit custom path (settings), then the agent's env override.
    //    A non-empty-but-invalid custom path intentionally falls through to the
    //    search below (it does not consult the env var).
    if let Some(c) = custom.map(str::trim).filter(|s| !s.is_empty()) {
        if is_valid_executable(c) {
            return Some(Resolved { path: c.to_string(), source: "custom-path".into() });
        }
    } else if let Some(env_key) = def.env_var {
        if let Ok(env_bin) = std::env::var(env_key) {
            let env_bin = env_bin.trim();
            if !env_bin.is_empty() && is_valid_executable(env_bin) {
                return Some(Resolved { path: env_bin.to_string(), source: "custom-path".into() });
            }
        }
    }

    // 2. PATH + toolchain bins, scanned with PATHEXT.
    let dirs = search_dirs(def);
    let exts = pathext();
    for bin in def.bin_candidates {
        for dir in &dirs {
            for ext in &exts {
                let candidate = dir.join(format!("{bin}{ext}"));
                if candidate.is_file() {
                    return Some(Resolved {
                        path: candidate.to_string_lossy().into_owned(),
                        source: "path".into(),
                    });
                }
            }
        }
    }
    None
}

fn is_valid_executable(p: &str) -> bool {
    let path = std::path::Path::new(p);
    path.is_absolute() && path.is_file()
}

/// PATHEXT extensions to try (with leading dot), plus a bare "" so an
/// extension-less file still matches. Defaults match Open Design
/// (`.EXE;.CMD;.BAT`).
fn pathext() -> Vec<String> {
    let raw = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string());
    let mut exts: Vec<String> = raw
        .split(';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    exts.push(String::new());
    exts
}

/// Build the ordered list of directories to scan: process PATH first, then the
/// shared well-known user toolchain bin directories (the GUI/packaged "stripped
/// PATH" compensation), then any agent-specific extras. De-duplicated, order
/// preserved.
fn search_dirs(def: &AgentDef) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let add = |dirs: &mut Vec<PathBuf>, p: PathBuf| {
        if !p.as_os_str().is_empty() && !dirs.contains(&p) {
            dirs.push(p);
        }
    };

    // process PATH
    if let Ok(path) = std::env::var("PATH") {
        for d in std::env::split_paths(&path) {
            add(&mut dirs, d);
        }
    }

    let appdata = std::env::var("APPDATA").ok();
    let localappdata = std::env::var("LOCALAPPDATA").ok();
    let userprofile = std::env::var("USERPROFILE").ok();

    // npm global prefix default: %APPDATA%\npm  (most important for npm installs)
    if let Some(ad) = &appdata {
        add(&mut dirs, PathBuf::from(ad).join("npm"));
    }

    // Explicit npm prefix override
    if let Ok(prefix) = std::env::var("NPM_CONFIG_PREFIX") {
        add(&mut dirs, PathBuf::from(&prefix));
        add(&mut dirs, PathBuf::from(&prefix).join("bin"));
    }

    // Standard user-level toolchain dirs, then agent-specific extras
    // (e.g. opencode's own ~/.opencode/bin).
    if let Some(up) = &userprofile {
        let home = PathBuf::from(up);
        for sub in [
            "scoop\\shims",
            ".bun\\bin",
            ".cargo\\bin",
            ".local\\bin",
            ".deno\\bin",
            ".volta\\bin",
        ] {
            add(&mut dirs, home.join(sub));
        }
        for sub in def.extra_search_subdirs {
            add(&mut dirs, home.join(sub));
        }
    }

    // fnm node-versions: Windows installs binaries directly in `installation`
    // (no /bin subdir). Globs %APPDATA%\fnm\... and %LOCALAPPDATA%\fnm\...
    for base in [appdata.as_ref(), localappdata.as_ref()].into_iter().flatten() {
        let node_versions = PathBuf::from(base).join("fnm").join("node-versions");
        if let Ok(entries) = std::fs::read_dir(&node_versions) {
            for entry in entries.flatten() {
                add(&mut dirs, entry.path().join("installation"));
            }
        }
    }
    // FNM_DIR override
    if let Ok(fnm_dir) = std::env::var("FNM_DIR") {
        let node_versions = PathBuf::from(fnm_dir).join("node-versions");
        if let Ok(entries) = std::fs::read_dir(&node_versions) {
            for entry in entries.flatten() {
                add(&mut dirs, entry.path().join("installation"));
            }
        }
    }

    dirs
}

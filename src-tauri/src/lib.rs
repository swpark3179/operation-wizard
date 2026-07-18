mod agents;
mod aipro;
mod confluence;
mod detect;
mod exec;
mod fabrix;
mod files;
mod knowledge;
mod mcp;
mod projects;
mod rag;
mod resolve;
mod run;
mod settings;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use detect::DetectedAgent;
use settings::{AiProConfig, ConfluenceConfig, FabrixConfig, RagConfig, Settings, SkillDef, StepDef};

// ---------------------------------------------------------------------------
// Boot diagnostics (D56). In release builds `windows_subsystem = "windows"`
// means a startup panic has no console to print to — the process just
// vanishes. These helpers make that failure diagnosable: every boot error is
// appended to a log file under the app's home root, and (on Windows, while
// still booting) surfaced to the user via a best-effort message box.
// ---------------------------------------------------------------------------

/// True until the Tauri event loop reports `RunEvent::Ready`. Confines the
/// failure dialog to boot-time death; post-boot worker panics only log.
static BOOT_PHASE: AtomicBool = AtomicBool::new(true);

/// The single app home root `~/.operation-wizard/` (Windows `%USERPROFILE%\
/// .operation-wizard\`). Every on-disk file the app writes lives under here —
/// `settings.json`, `projects/`, `knowledge/`, `startup-error.log` (D72). This
/// is the one definition of that path; `projects.rs`/`knowledge.rs`/`settings`
/// callers all resolve through it. Deliberately NOT the Tauri `app_config_dir`
/// (`%APPDATA%\<identifier>`) so all app data sits in one discoverable folder.
pub(crate) fn ow_home() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(|h| PathBuf::from(h).join(".operation-wizard"))
        .map_err(|_| "홈 디렉터리를 찾을 수 없습니다 (USERPROFILE/HOME 미설정)".to_string())
}

/// `~/.operation-wizard/startup-error.log`. The Tauri config dir is not an
/// option here: it needs an `AppHandle`, which does not exist when the builder
/// itself fails. Falls back to the temp dir if no home is resolvable (so a boot
/// failure is still logged somewhere).
fn startup_log_path() -> PathBuf {
    ow_home()
        .map(|d| d.join("startup-error.log"))
        .unwrap_or_else(|_| std::env::temp_dir().join("operation-wizard-startup-error.log"))
}

/// Append one timestamped line to the log. Every error is swallowed —
/// diagnostics must never become a second crash.
fn append_startup_log(path: &Path, msg: &str) {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(
            f,
            "[{}] operation-wizard v{} — {}",
            utc_timestamp(),
            env!("CARGO_PKG_VERSION"),
            msg
        );
    }
}

/// Log a boot/runtime diagnostic to `startup-error.log` (best-effort).
pub(crate) fn log_startup_error(msg: &str) {
    append_startup_log(&startup_log_path(), msg);
}

/// Zero-dep `YYYY-MM-DD HH:MM:SS Z` from the system clock (civil-from-days,
/// Howard Hinnant's algorithm) — avoids pulling in `chrono` for one line.
fn utc_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format_utc(secs)
}

fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if mo <= 2 { 1 } else { 0 };
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}Z")
}

/// Route panics through the log (and, during boot, the failure dialog) before
/// delegating to the default hook so dev builds still print to stderr.
fn install_startup_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic payload".into());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".into());
        let msg = format!("panic at {location}: {payload}");
        log_startup_error(&msg);
        if BOOT_PHASE.load(Ordering::Relaxed) {
            show_boot_failure_dialog(&msg);
        }
        default_hook(info);
    }));
}

/// Best-effort native alert for a boot failure. PowerShell keeps this at zero
/// new dependencies (mshta is a corporate-blocked LOLBin, `rfd`/`windows-sys`
/// would be new crates, and the dialog plugin needs the very `AppHandle` we
/// failed to build). `exec::command_for` applies CREATE_NO_WINDOW, so only the
/// WPF message box is visible. The log is always written first — if PowerShell
/// is blocked this silently does nothing and the log remains the diagnostic.
#[cfg(windows)]
fn show_boot_failure_dialog(detail: &str) {
    let detail: String = detail.chars().take(300).collect::<String>().replace(['\r', '\n'], " ");
    let text = format!(
        "Operation Wizard를 시작하지 못했습니다.\n\n가장 흔한 원인은 Microsoft Edge WebView2 런타임 부재/손상입니다. WebView2 런타임을 설치(복구)한 뒤 다시 실행해 주세요.\n\n오류: {detail}\n로그: {}",
        startup_log_path().display()
    );
    // PowerShell single-quoted string: only `'` needs escaping (doubled).
    let script = format!(
        "Add-Type -AssemblyName PresentationFramework; [System.Windows.MessageBox]::Show('{}','Operation Wizard','OK','Error')",
        text.replace('\'', "''")
    );
    let mut cmd = exec::command_for(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", script.as_str()],
    );
    let _ = cmd.status();
}

#[cfg(not(windows))]
fn show_boot_failure_dialog(_detail: &str) {}

/// Registry metadata for one agent. The frontend renders one card per entry,
/// in registry order.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AgentInfo {
    id: String,
    name: String,
    /// Env var that overrides the binary path (e.g. `OPENCODE_BIN`), or null.
    env_var: Option<String>,
}

/// List the agents this app knows how to detect (registry metadata).
#[tauri::command]
fn list_agents() -> Vec<AgentInfo> {
    agents::all()
        .iter()
        .map(|d| AgentInfo {
            id: d.id.into(),
            name: d.name.into(),
            env_var: d.env_var.map(|s| s.to_string()),
        })
        .collect()
}

/// Detect one agent by id: resolve the executable, probe `--version`, list
/// models. Runs on a blocking thread so the long `models` probe never blocks
/// the UI thread.
#[tauri::command]
async fn detect_agent(agent_id: String, force: bool) -> Result<DetectedAgent, String> {
    let def = agents::find(&agent_id).ok_or_else(|| format!("unknown agent: {agent_id}"))?;
    let config_dir = ow_home()?;
    let s = settings::load(&config_dir);
    // Remote (HTTP) agents detect via config presence + a (cache-first)
    // model-list call, not resolve+probe (Fabrix D64, AI Pro D71). `force` forces
    // a live fetch; otherwise a cached list is returned without a network call
    // (D66). Two remote agents now, so dispatch on the id.
    if def.kind == agents::AgentKind::Remote {
        let dir = config_dir.clone();
        let agent = match def.id {
            "fabrix" => {
                let cfg = s.fabrix.clone();
                tauri::async_runtime::spawn_blocking(move || fabrix::detect_fabrix(cfg, force))
                    .await
                    .map_err(|e| format!("{e:?}"))?
            }
            "aipro" => {
                let cfg = s.aipro.clone();
                tauri::async_runtime::spawn_blocking(move || aipro::detect_aipro(cfg, force))
                    .await
                    .map_err(|e| format!("{e:?}"))?
            }
            other => return Err(format!("unknown remote agent: {other}")),
        };
        // Persist a freshly fetched (live) list so it survives restarts and
        // seeds the cache path next time (best-effort — D66).
        if agent.models_source == "live" && !agent.models.is_empty() {
            let mut s2 = settings::load(&dir);
            match def.id {
                "fabrix" => {
                    if let Some(f) = s2.fabrix.as_mut() {
                        f.models = agent.models.clone();
                        let _ = settings::save(&dir, &s2);
                    }
                }
                "aipro" => {
                    if let Some(a) = s2.aipro.as_mut() {
                        a.models = agent.models.clone();
                        let _ = settings::save(&dir, &s2);
                    }
                }
                _ => {}
            }
        }
        return Ok(agent);
    }
    let custom = s.agent_custom_bin(&agent_id);
    tauri::async_runtime::spawn_blocking(move || detect::detect_agent_blocking(def, custom))
        .await
        .map_err(|e| format!("{e:?}"))
}

#[tauri::command]
fn get_settings() -> Result<Settings, String> {
    let config_dir = ow_home()?;
    Ok(settings::load(&config_dir))
}

/// Set (or clear, with `None`/empty) the custom binary path for one agent.
#[tauri::command]
fn set_agent_bin(agent_id: String, path: Option<String>) -> Result<Settings, String> {
    if agents::find(&agent_id).is_none() {
        return Err(format!("unknown agent: {agent_id}"));
    }
    let config_dir = ow_home()?;
    let mut s = settings::load(&config_dir);
    let normalized = match path {
        Some(p) if !p.trim().is_empty() => Some(p.trim().to_string()),
        _ => None,
    };
    s.set_agent_bin(&agent_id, normalized);
    settings::save(&config_dir, &s)?;
    Ok(s)
}

/// Replace the user's skill registry, or reset to built-in defaults (`None`).
/// Returns the full new settings (frontend replaces its state atomically).
#[tauri::command]
fn set_skills(skills: Option<Vec<SkillDef>>) -> Result<Settings, String> {
    if let Some(list) = &skills {
        settings::validate_skills(list)?;
    }
    let config_dir = ow_home()?;
    let mut s = settings::load(&config_dir);
    s.set_skills(skills);
    settings::save(&config_dir, &s)?;
    Ok(s)
}

/// Replace one category's workflow steps, or reset to the default (`None`).
#[tauri::command]
fn set_workflow(category: String, steps: Option<Vec<StepDef>>) -> Result<Settings, String> {
    if !settings::CATEGORIES.contains(&category.as_str()) {
        return Err(format!("unknown category: {category}"));
    }
    if let Some(list) = &steps {
        settings::validate_steps(list)?;
    }
    let config_dir = ow_home()?;
    let mut s = settings::load(&config_dir);
    s.set_workflow(&category, steps);
    settings::save(&config_dir, &s)?;
    Ok(s)
}

/// Set (or clear, with `None`/empty URL) the Confluence MCP connection (D82):
/// the MCP endpoint URL + `x-auth` key. An empty URL clears the config.
#[tauri::command]
fn set_confluence_config(config: Option<ConfluenceConfig>) -> Result<Settings, String> {
    let normalized = config
        .map(|mut c| {
            c.url = c.url.trim().to_string();
            c.auth_key = c.auth_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
            c
        })
        .filter(|c| !c.url.is_empty());
    let config_dir = ow_home()?;
    let mut s = settings::load(&config_dir);
    s.set_confluence(normalized);
    settings::save(&config_dir, &s)?;
    Ok(s)
}

/// Set (or clear, with `None`/empty endpoint) the RAG service config. The model
/// cache (`models`) is backend-owned: preserved across an unchanged-connection
/// re-save, cleared when the endpoint/credentials change (D66).
#[tauri::command]
fn set_rag_config(config: Option<RagConfig>) -> Result<Settings, String> {
    let config_dir = ow_home()?;
    let mut s = settings::load(&config_dir);
    let prev = s.rag.clone();
    let normalized = config
        .map(|mut c| {
            c.endpoint = c.endpoint.trim().trim_end_matches('/').to_string();
            c.secret_key = c.secret_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
            c.pass_key = c.pass_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
            c.knowledge_asset_id =
                c.knowledge_asset_id.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
            // Carry over the cached model list only when the connection is
            // unchanged; a save is followed by a live fetch (연결 테스트) that
            // refills it either way.
            c.models = match &prev {
                Some(p)
                    if p.endpoint == c.endpoint
                        && p.secret_key == c.secret_key
                        && p.pass_key == c.pass_key =>
                {
                    p.models.clone()
                }
                _ => Vec::new(),
            };
            c
        })
        .filter(|c| !c.endpoint.is_empty());
    s.set_rag(normalized);
    settings::save(&config_dir, &s)?;
    Ok(s)
}

/// Set (or clear, with `None`/empty endpoint) the Fabrix connection config
/// (D64). Trims the endpoint and header values; an empty endpoint clears it.
/// The model cache (`models`) is backend-owned: preserved across an
/// unchanged-connection re-save, cleared when endpoint/credentials change (D66).
#[tauri::command]
fn set_fabrix_config(config: Option<FabrixConfig>) -> Result<Settings, String> {
    let config_dir = ow_home()?;
    let mut s = settings::load(&config_dir);
    let prev = s.fabrix.clone();
    let normalized = config
        .map(|mut c| {
            c.endpoint_url = c.endpoint_url.trim().trim_end_matches('/').to_string();
            c.client = c.client.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
            c.openapi_token = c.openapi_token.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
            // Carry over the cached model list only when the connection is
            // unchanged; save is followed by detectOne(force=true) which refills.
            c.models = match &prev {
                Some(p)
                    if p.endpoint_url == c.endpoint_url
                        && p.client == c.client
                        && p.openapi_token == c.openapi_token =>
                {
                    p.models.clone()
                }
                _ => Vec::new(),
            };
            c
        })
        .filter(|c| !c.endpoint_url.is_empty());
    s.set_fabrix(normalized);
    settings::save(&config_dir, &s)?;
    Ok(s)
}

/// Set (or clear, with `None`/empty endpoint) the AI Pro connection config (D71).
/// Trims the endpoint and Bearer key; an empty endpoint clears it. The model
/// cache (`models`) is backend-owned: preserved across an unchanged-connection
/// re-save, cleared when endpoint/key change (D66).
#[tauri::command]
fn set_aipro_config(config: Option<AiProConfig>) -> Result<Settings, String> {
    let config_dir = ow_home()?;
    let mut s = settings::load(&config_dir);
    let prev = s.aipro.clone();
    let normalized = config
        .map(|mut c| {
            c.endpoint_url = c.endpoint_url.trim().trim_end_matches('/').to_string();
            c.api_key = c.api_key.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
            // Carry over the cached model list only when the connection is
            // unchanged; save is followed by detectOne(force=true) which refills.
            c.models = match &prev {
                Some(p) if p.endpoint_url == c.endpoint_url && p.api_key == c.api_key => {
                    p.models.clone()
                }
                _ => Vec::new(),
            };
            c
        })
        .filter(|c| !c.endpoint_url.is_empty());
    s.set_aipro(normalized);
    settings::save(&config_dir, &s)?;
    Ok(s)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_startup_panic_hook();

    // Debug-only escape hatch to exercise the failure path end-to-end
    // (log line + dialog + exit) without breaking a real WebView2 runtime.
    #[cfg(debug_assertions)]
    if std::env::var_os("OW_SIMULATE_BOOT_FAILURE").is_some() {
        let msg = "simulated boot failure (OW_SIMULATE_BOOT_FAILURE)";
        log_startup_error(msg);
        show_boot_failure_dialog(msg);
        std::process::exit(1);
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(run::RunRegistry::default())
        .manage(confluence::IngestRegistry::default())
        .invoke_handler(tauri::generate_handler![
            list_agents,
            detect_agent,
            get_settings,
            set_agent_bin,
            set_skills,
            set_workflow,
            set_confluence_config,
            set_rag_config,
            set_fabrix_config,
            set_aipro_config,
            run::run_agent,
            run::cancel_run,
            files::list_dir,
            files::read_file,
            files::write_file,
            projects::ensure_project,
            projects::save_session,
            projects::list_sessions,
            projects::load_session,
            projects::list_projects,
            projects::set_project_codebase,
            projects::set_project_title,
            knowledge::list_knowledge,
            knowledge::save_knowledge,
            knowledge::save_knowledge_files,
            knowledge::get_knowledge_root,
            knowledge::delete_knowledge,
            confluence::start_confluence_ingest,
            confluence::cancel_ingest,
            confluence::probe_confluence,
            rag::rag_search,
            rag::probe_rag,
            fabrix::probe_fabrix,
            aipro::probe_aipro
        ])
        // `build` + `run` split (instead of `.run(...).expect(...)`): webview /
        // window creation failures — above all a missing or broken WebView2
        // runtime — surface here as a typed `Err` we can log and explain,
        // instead of an invisible panic in a console-less release build.
        .build(tauri::generate_context!());

    match app {
        Ok(app) => app.run(|_handle, event| {
            if matches!(event, tauri::RunEvent::Ready) {
                BOOT_PHASE.store(false, Ordering::Relaxed);
            }
        }),
        Err(e) => {
            let msg = format!("failed to start: {e}");
            log_startup_error(&msg);
            show_boot_failure_dialog(&msg);
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_utc_known_epochs() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00Z");
        // Leap day of a century leap year.
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00:00Z");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14 22:13:20Z");
    }

    #[test]
    fn append_startup_log_appends_lines() {
        let dir = std::env::temp_dir().join("ow-lib-test-startup-log");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("startup-error.log");

        append_startup_log(&path, "first");
        append_startup_log(&path, "second");

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains(concat!("v", env!("CARGO_PKG_VERSION"))));
        assert!(lines[0].ends_with("— first"));
        assert!(lines[1].ends_with("— second"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

mod agents;
mod confluence;
mod detect;
mod exec;
mod files;
mod knowledge;
mod projects;
mod rag;
mod resolve;
mod run;
mod settings;

use serde::Serialize;
use tauri::Manager;

use detect::DetectedAgent;
use settings::{ConfluenceConfig, RagConfig, Settings, SkillDef, StepDef};

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
async fn detect_agent(app: tauri::AppHandle, agent_id: String) -> Result<DetectedAgent, String> {
    let def = agents::find(&agent_id).ok_or_else(|| format!("unknown agent: {agent_id}"))?;
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let custom = settings::load(&config_dir).agent_custom_bin(&agent_id);
    tauri::async_runtime::spawn_blocking(move || detect::detect_agent_blocking(def, custom))
        .await
        .map_err(|e| format!("{e:?}"))
}

#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> Result<Settings, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(settings::load(&config_dir))
}

/// Set (or clear, with `None`/empty) the custom binary path for one agent.
#[tauri::command]
fn set_agent_bin(
    app: tauri::AppHandle,
    agent_id: String,
    path: Option<String>,
) -> Result<Settings, String> {
    if agents::find(&agent_id).is_none() {
        return Err(format!("unknown agent: {agent_id}"));
    }
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
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
fn set_skills(app: tauri::AppHandle, skills: Option<Vec<SkillDef>>) -> Result<Settings, String> {
    if let Some(list) = &skills {
        settings::validate_skills(list)?;
    }
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let mut s = settings::load(&config_dir);
    s.set_skills(skills);
    settings::save(&config_dir, &s)?;
    Ok(s)
}

/// Replace one category's workflow steps, or reset to the default (`None`).
#[tauri::command]
fn set_workflow(
    app: tauri::AppHandle,
    category: String,
    steps: Option<Vec<StepDef>>,
) -> Result<Settings, String> {
    if !settings::CATEGORIES.contains(&category.as_str()) {
        return Err(format!("unknown category: {category}"));
    }
    if let Some(list) = &steps {
        settings::validate_steps(list)?;
    }
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let mut s = settings::load(&config_dir);
    s.set_workflow(&category, steps);
    settings::save(&config_dir, &s)?;
    Ok(s)
}

/// Set (or clear, with `None`/empty base URL) the Confluence crawl config.
#[tauri::command]
fn set_confluence_config(
    app: tauri::AppHandle,
    config: Option<ConfluenceConfig>,
) -> Result<Settings, String> {
    let normalized = config
        .map(|mut c| {
            c.base_url = c.base_url.trim().trim_end_matches('/').to_string();
            c.token = c.token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
            c.root_page_id = c.root_page_id.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
            c.space_key = c.space_key.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
            c
        })
        .filter(|c| !c.base_url.is_empty());
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let mut s = settings::load(&config_dir);
    s.set_confluence(normalized);
    settings::save(&config_dir, &s)?;
    Ok(s)
}

/// Set (or clear, with `None`/empty endpoint) the RAG service config.
#[tauri::command]
fn set_rag_config(app: tauri::AppHandle, config: Option<RagConfig>) -> Result<Settings, String> {
    let normalized = config
        .map(|mut c| {
            c.endpoint = c.endpoint.trim().trim_end_matches('/').to_string();
            c.api_key = c.api_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
            c
        })
        .filter(|c| !c.endpoint.is_empty());
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let mut s = settings::load(&config_dir);
    s.set_rag(normalized);
    settings::save(&config_dir, &s)?;
    Ok(s)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            run::run_agent,
            run::cancel_run,
            files::list_dir,
            files::read_file,
            projects::ensure_project,
            projects::save_session,
            projects::list_sessions,
            projects::load_session,
            projects::list_projects,
            projects::set_project_codebase,
            knowledge::list_knowledge,
            knowledge::save_knowledge,
            knowledge::delete_knowledge,
            confluence::start_confluence_ingest,
            confluence::cancel_ingest,
            confluence::probe_confluence,
            rag::rag_search
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

import { invoke, Channel } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  AgentInfo,
  DetectedAgent,
  FileEntry,
  ProjectMeta,
  ProjectSummary,
  RunArgs,
  RunEvent,
  SessionMeta,
  Settings,
  SkillDef,
  StepDef,
  StoredSession,
} from "./types";

export function listAgents(): Promise<AgentInfo[]> {
  return invoke<AgentInfo[]>("list_agents");
}

export function detectAgent(agentId: string): Promise<DetectedAgent> {
  return invoke<DetectedAgent>("detect_agent", { agentId });
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export function setAgentBin(agentId: string, path: string | null): Promise<Settings> {
  return invoke<Settings>("set_agent_bin", { agentId, path });
}

/** Replace the user skill registry, or reset to built-in defaults with null. */
export function setSkills(skills: SkillDef[] | null): Promise<Settings> {
  return invoke<Settings>("set_skills", { skills });
}

/** Replace one category's workflow steps, or reset to the default with null. */
export function setWorkflow(category: string, steps: StepDef[] | null): Promise<Settings> {
  return invoke<Settings>("set_workflow", { category, steps });
}

// ── Agent runs ───────────────────────────────────────────────────────────────

/**
 * Start an agent run. The backend streams `RunEvent`s over `onEvent` (a Tauri
 * Channel) until a terminal `end` event; the returned string is the run id
 * (pass it to {@link cancelRun}).
 */
export function runAgent(args: RunArgs, onEvent: Channel<RunEvent>): Promise<string> {
  return invoke<string>("run_agent", { args, onEvent });
}

export function cancelRun(runId: string): Promise<void> {
  return invoke("cancel_run", { runId });
}

// ── Canvas file viewer ───────────────────────────────────────────────────────

export function listDir(path: string): Promise<FileEntry[]> {
  return invoke<FileEntry[]>("list_dir", { path });
}

export function readFile(path: string): Promise<string> {
  return invoke<string>("read_file", { path });
}

// ── Conversation persistence (projects/sessions) ─────────────────────────────

/** Create the project folder + manifest (idempotent). `workdir` empty → the
 * project's own `workspace/` subfolder; otherwise the external folder. Returns
 * the manifest with the resolved `workdir`. */
export function ensureProject(
  projectId: string,
  workdir: string,
  title: string,
  category: string,
): Promise<ProjectMeta> {
  return invoke<ProjectMeta>("ensure_project", { projectId, workdir, title, category });
}

/** Persist one session (project must already exist via {@link ensureProject}). */
export function saveSession(projectId: string, session: StoredSession): Promise<void> {
  return invoke("save_session", { projectId, session });
}

/** List a project's sessions (metadata only), newest-updated first. */
export function listSessions(projectId: string): Promise<SessionMeta[]> {
  return invoke<SessionMeta[]>("list_sessions", { projectId });
}

/** Load one full session (metadata + messages). */
export function loadSession(projectId: string, sessionId: string): Promise<StoredSession> {
  return invoke<StoredSession>("load_session", { projectId, sessionId });
}

/** List all projects for the Home "recent" list. */
export function listProjects(): Promise<ProjectSummary[]> {
  return invoke<ProjectSummary[]>("list_projects");
}

/** Open the native folder picker; returns the chosen path or null if canceled. */
export async function pickFolder(): Promise<string | null> {
  const res = await open({ directory: true, multiple: false });
  return typeof res === "string" ? res : null;
}

// Re-export Channel so components can construct one without importing Tauri directly.
export { Channel };

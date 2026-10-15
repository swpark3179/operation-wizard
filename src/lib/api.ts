import { invoke, Channel } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import type {
  AgentInfo,
  CodebaseLearnings,
  DetectedAgent,
  FabrixConfig,
  FileEntry,
  KnowledgeEntry,
  ProjectMeta,
  ProjectSummary,
  RagConfig,
  RagHit,
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

/** Detect one agent. `force` forces a live model fetch for the remote agent
 * (FabriX); without it a cached model list is used, no network call (D66). */
export function detectAgent(agentId: string, force = false): Promise<DetectedAgent> {
  return invoke<DetectedAgent>("detect_agent", { agentId, force });
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

/** Set (or clear, with null/empty endpoint) the RAG service config. */
export function setRagConfig(config: RagConfig | null): Promise<Settings> {
  return invoke<Settings>("set_rag_config", { config });
}

/** Set (or clear, with null/empty endpoint) the FabriX connection config (D64/D84). */
export function setFabrixConfig(config: FabrixConfig | null): Promise<Settings> {
  return invoke<Settings>("set_fabrix_config", { config });
}

/** FabriX connection test: fetches the model list (`/openapi/llm/v1/models`),
 * caches it and returns a summary ("연결됨 — N개 모델 (기본: …)"). Rejects with a
 * Korean message (+ hint) if unconfigured/unreachable/unauthorized. */
export function probeFabrix(): Promise<string> {
  return invoke<string>("probe_fabrix");
}

// ── RAG / knowledge ─────────────────────────────────────────────────────────

/** Search the in-house RAG service (rag workflow step — Fabrix rag-chat, D65).
 * Rejects with a Korean message when unconfigured/unreachable. */
export function ragSearch(query: string, topK?: number): Promise<RagHit[]> {
  return invoke<RagHit[]>("rag_search", { query, topK: topK ?? null });
}

/** RAG connection test: fetches the rag-chat model list and returns a summary
 * ("연결됨 (N개 모델)"). Rejects with a Korean message if unconfigured/unreachable. */
export function probeRag(): Promise<string> {
  return invoke<string>("probe_rag");
}

/** All knowledge entries (full bodies), newest-updated first. */
export function listKnowledge(): Promise<KnowledgeEntry[]> {
  return invoke<KnowledgeEntry[]>("list_knowledge");
}

/** Upsert one knowledge entry; returns it with stamped timestamps. */
export function saveKnowledge(entry: KnowledgeEntry): Promise<KnowledgeEntry> {
  return invoke<KnowledgeEntry>("save_knowledge", { entry });
}

/** Copy workflow output files into the entry's artifact folder and upsert the
 * entry as kind "artifact" (D59). `sources` are absolute file paths; the
 * returned entry's `files` holds the copied names. */
export function saveKnowledgeFiles(
  entry: KnowledgeEntry,
  sources: string[],
): Promise<KnowledgeEntry> {
  return invoke<KnowledgeEntry>("save_knowledge_files", { entry, sources });
}

/** Absolute knowledge root path (join `artifacts/<id>/<name>` for the
 * injection index and the extraDirs grant — D59). */
export function getKnowledgeRoot(): Promise<string> {
  return invoke<string>("get_knowledge_root");
}

/** Delete one knowledge entry (idempotent; artifact files go with it). */
export function deleteKnowledge(id: string): Promise<void> {
  return invoke("delete_knowledge", { id });
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

/** Write text to `path`, creating parent dirs (D67). Used to persist a remote
 * agent's (FabriX) document-step output — it can only *read* files (read-only
 * tool loop, D84), so the app writes `<workdir>/<step.file>` itself. */
export function writeFile(path: string, contents: string): Promise<void> {
  return invoke("write_file", { path, contents });
}

// ── Code map (D88) ───────────────────────────────────────────────────────────

/** The non-AI repository map of a codebase folder (tech stack, tree, routes,
 * dependency chains down to SQL tables, hubs, key outlines) — injected by the
 * codebase-step preflight. `focusTerms` (identifier-like words from the
 * request) are listed first when they hit statements/types/routes. */
export function codebaseMap(
  root: string,
  focusTerms: string[],
  detail?: "extended" | "focus",
): Promise<string> {
  return invoke<string>("codebase_map", { root, focusTerms, detail: detail ?? null });
}

/** Learnings an earlier project saved for this codebase + category, with the
 * files changed since (D90). `null` when nothing is cached. */
export function getCodebaseLearnings(codebasePath: string, category: string): Promise<CodebaseLearnings | null> {
  return invoke<CodebaseLearnings | null>("get_codebase_learnings", { codebasePath, category });
}

/** Save an orientation step's Learnings for later projects on the same
 * codebase (`~/.operation-wizard/codebases/` — never the codebase folder). */
export function saveCodebaseLearnings(
  codebasePath: string,
  category: string,
  text: string,
  projectId: string,
): Promise<void> {
  return invoke("save_codebase_learnings", { codebasePath, category, text, projectId });
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
  codebasePath?: string | null,
): Promise<ProjectMeta> {
  return invoke<ProjectMeta>("ensure_project", {
    projectId,
    workdir,
    title,
    category,
    codebasePath: codebasePath ?? null,
  });
}

/** Update (or clear with null) an existing project's codebase path. */
export function setProjectCodebase(
  projectId: string,
  codebasePath: string | null,
): Promise<ProjectMeta> {
  return invoke<ProjectMeta>("set_project_codebase", { projectId, codebasePath });
}

/** Rename an existing project (Home recent list inline edit). */
export function setProjectTitle(projectId: string, title: string): Promise<ProjectMeta> {
  return invoke<ProjectMeta>("set_project_title", { projectId, title });
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

/** Open a folder (or file) path in the OS file explorer (D69). */
export function openInExplorer(path: string): Promise<void> {
  return openPath(path);
}

/** Open an http/https/mailto URL in the OS default browser/mail client (D76).
 * Used for links inside previewed HTML/markdown so a click never navigates the
 * app's WebView away. Rides on `opener:default` (allow-open-url) — no new grant. */
export function openExternal(url: string): Promise<void> {
  return openUrl(url);
}

// Re-export Channel so components can construct one without importing Tauri directly.
export { Channel };

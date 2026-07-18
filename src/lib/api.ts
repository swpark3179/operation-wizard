import { invoke, Channel } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import type {
  AgentInfo,
  AiProConfig,
  ConfluenceConfig,
  DetectedAgent,
  FabrixConfig,
  FileEntry,
  IngestEvent,
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
 * (Fabrix); without it a cached model list is used, no network call (D66). */
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

/** Set (or clear, with null/empty URL) the Confluence MCP connection (D82). */
export function setConfluenceConfig(config: ConfluenceConfig | null): Promise<Settings> {
  return invoke<Settings>("set_confluence_config", { config });
}

/** Set (or clear, with null/empty endpoint) the RAG service config. */
export function setRagConfig(config: RagConfig | null): Promise<Settings> {
  return invoke<Settings>("set_rag_config", { config });
}

/** Set (or clear, with null/empty endpoint) the Fabrix connection config (D64). */
export function setFabrixConfig(config: FabrixConfig | null): Promise<Settings> {
  return invoke<Settings>("set_fabrix_config", { config });
}

/** Fabrix connection test: fetches the model list and returns a summary
 * ("연결됨 (N개 모델)"). Rejects with a Korean message if unconfigured/unreachable. */
export function probeFabrix(): Promise<string> {
  return invoke<string>("probe_fabrix");
}

/** Set (or clear, with null/empty endpoint) the AI Pro connection config (D71). */
export function setAiProConfig(config: AiProConfig | null): Promise<Settings> {
  return invoke<Settings>("set_aipro_config", { config });
}

/** AI Pro connection test: fetches the OpenAI-compatible model list and returns a
 * summary ("연결됨 (N개 모델)"). Rejects with a Korean message if unconfigured. */
export function probeAiPro(): Promise<string> {
  return invoke<string>("probe_aipro");
}

// ── RAG / Confluence ingestion / knowledge ───────────────────────────────────

/** Search the user's RAG service (rag workflow step). Rejects with a Korean
 * guidance message while the rag.rs stubs are unimplemented/unconfigured. */
export function ragSearch(query: string, topK?: number): Promise<RagHit[]> {
  return invoke<RagHit[]>("rag_search", { query, topK: topK ?? null });
}

/** RAG connection test: fetches the rag-chat model list and returns a summary
 * ("연결됨 (N개 모델)"). Rejects with a Korean message if unconfigured/unreachable. */
export function probeRag(): Promise<string> {
  return invoke<string>("probe_rag");
}

/** What to collect via the Confluence MCP server (D82): a root page (getChild
 * recursion) and/or a search query (searchContent). Passed per-run, not stored. */
export interface ConfluenceTarget {
  rootPageId?: string | null;
  searchQuery?: string | null;
}

/** Start a Confluence MCP crawl; pages become one knowledge-base artifact entry.
 * Progress streams over `onEvent` until a terminal `end` event. Returns the
 * ingest id (pass to {@link cancelIngest}). */
export function startConfluenceIngest(
  target: ConfluenceTarget,
  onEvent: Channel<IngestEvent>,
): Promise<string> {
  return invoke<string>("start_confluence_ingest", { target, onEvent });
}

export function cancelIngest(ingestId: string): Promise<void> {
  return invoke("cancel_ingest", { ingestId });
}

/** Settings-screen connection test (D82): handshakes with the MCP server and
 * returns a tool summary ("연결됨 — N개 도구 (…)"). */
export function probeConfluence(): Promise<string> {
  return invoke<string>("probe_confluence");
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
 * agent's (Fabrix) document-step output — it streams text but cannot write
 * files, so the app writes `<workdir>/<step.file>` itself. */
export function writeFile(path: string, contents: string): Promise<void> {
  return invoke("write_file", { path, contents });
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

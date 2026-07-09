// Mirrors the serde structs in src-tauri/src/detect.rs, settings.rs and the
// AgentInfo struct in lib.rs.

export interface ModelOption {
  id: string;
  label: string;
}

export type AgentSource = "custom-path" | "path" | "not-found";
export type ModelsSource = "live" | "fallback";
export type Diagnostic =
  | "not-on-path"
  | "not-executable"
  | "missing-target"
  | null;

export interface DetectedAgent {
  id: string;
  name: string;
  available: boolean;
  path: string | null;
  version: string | null;
  source: AgentSource;
  models: ModelOption[];
  modelsSource: ModelsSource;
  diagnostic: Diagnostic;
}

/** Registry metadata for one agent (from `list_agents`). */
export interface AgentInfo {
  id: string;
  name: string;
  /** Env var that overrides the binary path, e.g. "OPENCODE_BIN", or null. */
  envVar: string | null;
}

export interface AgentConfig {
  customBin: string | null;
}

/** One system skill: an instruction pack injected on the turn that runs a
 * workflow step carrying it (mirrors settings.rs `SkillDef`). */
export interface SkillDef {
  id: string;
  name: string;
  body: string;
  /** Optional resource folder (claude-skill style reference files/scripts).
   * Mentioned in the wire prompt and added to `RunArgs.extraDirs` on the turn
   * that injects the skill. */
  dir?: string | null;
}

/** One step of a category's guided workflow (mirrors settings.rs `StepDef`).
 * `kind` is a plain string on the wire; the frontend coerces it to `StepKind`
 * (see `coerceSteps` in lib/workflow.ts). */
export interface StepDef {
  id: string;
  /** Display name (settings editor + derived "N/M단계 · <name> 중…" notes). */
  name: string;
  kind: string;
  instruction: string;
  /** For a `document` step: the file the agent writes (workdir-relative). */
  file?: string | null;
  /** Skills injected on this step's turn (unknown ids are skipped at runtime). */
  skillIds: string[];
  /** Result form: "chat" | "file" | "html". Absent → derived from `kind`
   * (document→"file", else "chat"). "html" expands into an extra render
   * sub-step at runtime (see `expandOutputSteps` in lib/workflow.ts). */
  output?: string | null;
}

/** Confluence crawl source for RAG ingestion (mirrors settings.rs). The token
 * is stored as plain text in settings.json — use a read-only-scope PAT. */
export interface ConfluenceConfig {
  /** Base URL including any context path, e.g. "https://wiki.example.com/confluence". */
  baseUrl: string;
  /** Bearer PAT (Confluence Server/DC). */
  token?: string | null;
  /** Crawl root page id (descendants collected recursively). */
  rootPageId?: string | null;
  /** Alternative: flat listing of one space's pages. */
  spaceKey?: string | null;
  /** Opt-in for corporate TLS-inspection proxies whose CA is not installed. */
  allowInvalidCerts: boolean;
}

/** The user's RAG service endpoint (mirrors settings.rs). The two keys are
 * auth header values for the user's rag.rs implementation (D50). */
export interface RagConfig {
  endpoint: string;
  secretKey?: string | null;
  passKey?: string | null;
  /** Search result count requested by the rag workflow step. */
  topK?: number | null;
}

export interface Settings {
  /** Per-agent config, keyed by agent id. */
  agents: Record<string, AgentConfig>;
  /** User skill registry; absent/null → the app's built-in defaults. */
  skills?: SkillDef[] | null;
  /** Per-category workflow overrides; absent key → built-in default flow. */
  workflows?: Record<string, StepDef[]>;
  /** Confluence crawl source; absent/null → not configured. */
  confluence?: ConfluenceConfig | null;
  /** RAG service endpoint; absent/null → the rag workflow step skips. */
  rag?: RagConfig | null;
}

// ── Agent runs (mirrors src-tauri/src/run.rs) ────────────────────────────────

/** Normalized run event streamed from the backend over a Tauri Channel. */
export type RunEvent =
  | { type: "status"; label: string; model?: string; sessionId?: string }
  | { type: "textDelta"; delta: string }
  | { type: "thinkingDelta"; delta: string }
  | { type: "toolUse"; id: string; name: string; input: unknown }
  | { type: "toolResult"; toolUseId: string; content: string; isError: boolean }
  | { type: "usage"; inputTokens?: number; outputTokens?: number }
  | { type: "stdout"; chunk: string }
  | { type: "error"; message: string }
  | { type: "end"; code: number | null; status: string };

/** Arguments for one agent run (one turn). */
export interface RunArgs {
  agentId: string;
  prompt: string;
  cwd: string;
  model?: string | null;
  /** Session id carried across turns (claude: minted UUID, codex: captured id). */
  sessionId?: string | null;
  /** True when continuing a prior turn (resume) rather than starting fresh. */
  resume?: boolean;
  /** Extra readable directories beyond `cwd` (codebase path + armed skill
   * resource folders). Sent every turn — claude maps each to `--add-dir`,
   * gemini/aipro to `--include-directories` (D52); codex is full-access and
   * plain agents rely on the wire prompt mentions. */
  extraDirs?: string[];
}

// ── RAG / Confluence ingestion / knowledge (mirrors rag.rs, confluence.rs,
//    knowledge.rs) ─────────────────────────────────────────────────────────────

/** One RAG search hit — rendered in the canvas "검색 결과" tab and injected
 * into the rag step's agent turn. */
export interface RagHit {
  title?: string | null;
  url?: string | null;
  snippet: string;
  score?: number | null;
}

/** Progress events streamed while a Confluence crawl+ingest runs. */
export type IngestEvent =
  | { type: "started"; rootId: string }
  | { type: "pageFetched"; pageId: string; title: string; fetched: number }
  | { type: "pageIngested"; pageId: string; title: string; ingested: number }
  | { type: "pageFailed"; pageId: string; title: string; message: string }
  | { type: "error"; message: string }
  | { type: "end"; status: string; ingested: number; failed: number };

/** One knowledge entry (title + how-it-was-done body), injected into the
 * foundation phase's knowledge step. */
export interface KnowledgeEntry {
  id: string;
  title: string;
  body: string;
  createdAt: number;
  updatedAt: number;
}

/** One directory entry (mirrors src-tauri/src/files.rs). */
export interface FileEntry {
  name: string;
  path: string;
  isDir: boolean;
}

// ── Conversation persistence (mirrors src-tauri/src/projects.rs) ──────────────

/** Project manifest — one project per work unit (id minted on the frontend). */
export interface ProjectMeta {
  id: string;
  /** Resolved agent working folder (own `workspace/` subfolder or external). */
  workdir: string;
  title: string;
  category: string;
  createdAt: number;
  /** Codebase folder analyzed in the foundation phase — separate from `workdir`. */
  codebasePath?: string | null;
}

/** A project row for the Home "recent" list (manifest + activity rollup). */
export interface ProjectSummary {
  id: string;
  workdir: string;
  title: string;
  category: string;
  createdAt: number;
  /** Latest session's updatedAt (falls back to createdAt if no sessions). */
  updatedAt: number;
  sessionCount: number;
  /** Most-recently-updated session id, to open when the project is clicked. */
  lastSessionId?: string | null;
  /** The project's analyzed codebase folder (restored when reopening). */
  codebasePath?: string | null;
}

/** Session metadata (header of a stored session; returned by `list_sessions`). */
export interface SessionMeta {
  /** Persistence id (frontend-minted UUID; the session folder name). */
  id: string;
  /** First user prompt, truncated — used as the list title. */
  title: string;
  agentId: string;
  model: string;
  category: string;
  /** Agent CLI session id for resume (claude/codex), or null (gemini/aipro). */
  cliSessionId?: string | null;
  createdAt: number;
  updatedAt: number;
  messageCount: number;
}

/** A full stored session: metadata + messages. `messages` mirrors the backend's
 * opaque `serde_json::Value` (a `ChatMessage[]`); callers cast as needed. */
export interface StoredSession extends SessionMeta {
  messages: unknown[];
}

export const DIAGNOSTIC_HINT: Record<string, string> = {
  "not-on-path":
    "Not found on PATH or known toolchain dirs. Set a custom path below.",
  "not-executable": "The resolved file is not executable.",
  "missing-target": "The shim points to a missing target (e.g. a removed runtime).",
};

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
}

export interface Settings {
  /** Per-agent config, keyed by agent id. */
  agents: Record<string, AgentConfig>;
  /** User skill registry; absent/null → the app's built-in defaults. */
  skills?: SkillDef[] | null;
  /** Per-category workflow overrides; absent key → built-in default flow. */
  workflows?: Record<string, StepDef[]>;
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

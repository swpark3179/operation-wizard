import { useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronLeft,
  ArrowUp,
  ArrowDown,
  AlertTriangle,
  BookmarkPlus,
  Square,
  ClipboardList,
  FilePlus2,
  History,
  X,
} from "lucide-react";
import { ask } from "@tauri-apps/plugin-dialog";
import { AssistantMessage } from "./AssistantMessage";
import { WorkflowStepper, type StepProgress, type StepProgressStatus } from "./WorkflowStepper";
import { categoryLabel, sessionTime, type Category, type ChatMessage } from "./workspace";
import {
  Channel,
  cancelRun,
  codebaseMap,
  ensureProject,
  getCodebaseLearnings,
  getKnowledgeRoot,
  listKnowledge,
  listSessions,
  loadSession,
  ragSearch,
  readFile,
  runAgent,
  saveCodebaseLearnings,
  saveSession,
  setProjectCodebase,
  setProjectTitle,
  writeFile,
} from "../lib/api";
import { useAutoGrow } from "../lib/useAutoGrow";
import type { ClarifyQuestion } from "../lib/clarify";
import { joinWorkdirPath } from "../lib/artifacts";
import { REPO_MAP_FILE, REPO_MAP_NAME, repoMapDocument } from "../lib/repoMap";
import { buildRagQuery, formatKnowledgeContext, formatRagContext, ragUserError } from "../lib/foundation";
import { judgeRagRelevance, type RagVerdict } from "../lib/ragRelevance";
import { optionsFor } from "../lib/options";
import { PROMPT_NOTE, PROMPT_OPTIMIZER_SKILL, parsePromptBlock, stripPromptBlock } from "../lib/promptCraft";
import { heuristicTitle, summarizeTitle } from "../lib/sessionTitle";
import { resolveSkills } from "../lib/skills";
import { codeAnalysisMode, codeMethodBody, codeMethodSkillId, focusTermsFrom } from "../lib/codeMethod";
import {
  cachedLearningsNote,
  extractLearnings,
  formatCachedLearnings,
  formatHandoffContext,
  type ProducedDoc,
} from "../lib/learnings";
import { isGenerative, progressLabel, runtimeWorkflowFor } from "../lib/workflow";
import type {
  AgentInfo,
  DetectedAgent,
  RagHit,
  RunEvent,
  SessionMeta,
  Settings,
  StepDef,
  StoredSession,
} from "../lib/types";

/** Agents that take the prompt as a command-line argument (`-p "<prompt>"`,
 * through a cmd.exe shim with an ~8k limit — agents.rs `RunSpec::plain`): the
 * repository map is cut short for them (D88). */
const PLAIN_AGENTS = ["opencode", "antigravity"];
/** Repository-map cap for plain agents. */
const PLAIN_MAP_MAX = 2000;

/** Agents that keep a CLI session across turns (claude mints a UUID, codex
 * captures its own thread id). Others are sessionless → we re-send the
 * transcript each turn. */
const SESSION_AGENTS = ["claude", "codex"];

/** Pick the default agent: prefer an available Claude Code, else the first
 * available detected agent, else the first known agent. Exported for the
 * manual knowledge-save path (loaded sessions pick a fallback agent — D59). */
export function defaultAgentId(agents: AgentInfo[], detected: Record<string, DetectedAgent>): string {
  if (detected.claude?.available) return "claude";
  const firstAvailable = agents.find((a) => detected[a.id]?.available);
  return firstAvailable?.id ?? agents[0]?.id ?? "claude";
}

/** Agent for a saved session whose agent is no longer registered (AI Pro was
 * removed — D83). AI Pro was sessionless, so prefer the sessionless FabriX
 * (the transcript replay then continues the conversation), else the default. */
function fallbackForRemovedAgent(agents: AgentInfo[], detected: Record<string, DetectedAgent>): string {
  return detected.fabrix?.available ? "fabrix" : defaultAgentId(agents, detected);
}

/** Flatten the conversation into a plain transcript for sessionless agents. */
function buildTranscript(prev: ChatMessage[], latest: string): string {
  const lines: string[] = [];
  for (const m of prev) {
    if (m.system) continue; // skip workflow progress notes
    if (m.role === "user") lines.push(`사용자: ${m.content}`);
    else if (m.content) lines.push(`어시스턴트: ${m.content}`);
  }
  lines.push(`사용자: ${latest}`);
  return lines.join("\n\n");
}

/** The last assistant message's accumulated text (the streamed reply). */
function lastAssistantContent(msgs: ChatMessage[]): string {
  for (let i = msgs.length - 1; i >= 0; i--) {
    if (msgs[i].role === "assistant") return msgs[i].content;
  }
  return "";
}

/** The last assistant message's error text (set from a `RunEvent::Error`, e.g.
 * the CLI's stderr on a failed turn). */
function lastAssistantError(msgs: ChatMessage[]): string {
  for (let i = msgs.length - 1; i >= 0; i--) {
    if (msgs[i].role === "assistant") return msgs[i].error ?? "";
  }
  return "";
}

/** Per-step automatic-retry budget for a workflow turn that failed with a
 * transient error (stream/timeout/network hiccups — first seen as the former
 * AI Pro backend's intermittent >45s first-response timeout), which would
 * otherwise abort a multi-step workflow (D68). */
const MAX_STEP_RETRIES = 2;

/** Does a failed turn's error look transient (worth an automatic retry)? Targets
 * streaming/timeout/network hiccups and explicitly excludes fatal errors (wrong
 * model, TLS/cert distrust, auth/guardrail blocks) that would only retry into the
 * same wall (D68; cf. codex TLS D28). */
function isTransientFailure(msg: string): boolean {
  const m = msg.toLowerCase();
  if (
    /model not found|invalid peer certificate|badsignature|certificate|unauthorized|forbidden|\b40[13]\b|api key|credential/.test(
      m,
    )
  )
    return false;
  return /timeout|timed out|etimedout|econnreset|socket hang up|premature close|network|stream (disconnect|clos|interrupt|error)|streaming request/.test(
    m,
  );
}

/** FabriX's final "empty reply" error — sent only after the backend already
 * re-asked within the turn (forced final + a tool-free recovery request). The
 * workflow skips the step on it instead of halting or re-running it (D93). */
function isEmptyReplyFailure(msg: string): boolean {
  return msg.includes("FabriX가 빈 응답");
}

/** Turn a remote agent's document reply into the file body (D67): if the whole
 * reply is one fenced code block (```md … ```), unwrap it; else trim as-is.
 * The caller passes only the text after the turn's last tool call (D84). */
function extractDocBody(content: string): string {
  const trimmed = content.trim();
  const m = trimmed.match(/^```[^\n]*\n([\s\S]*?)\n?```$/);
  return (m ? m[1] : trimmed).trim();
}

export function ChatPanel({
  projectId,
  onResolveWorkdir,
  category,
  initialAgentId,
  initialModel,
  agents,
  detected,
  settings,
  workdir,
  codebasePath,
  initialSession,
  answerSubmission,
  formPending,
  onHome,
  onNewSession,
  onOpenSession,
  onOpenFile,
  onClarify,
  onRagResult,
  onPromptResult,
  onStreamingChange,
  onStepProgress,
  onSessionTitle,
  onOpenAgents,
  onOpenKnowledgeSave,
}: {
  /** The active project id (folder key for persistence). */
  projectId: string;
  /** Lift the resolved workdir up after the first send creates the project. */
  onResolveWorkdir: (workdir: string) => void;
  category: Category;
  /** Agent/model picked on Home (D60/D95) — seeds a fresh chat's selection
   * (still changeable before the first turn); null → auto default. */
  initialAgentId?: string | null;
  initialModel?: string | null;
  agents: AgentInfo[];
  detected: Record<string, DetectedAgent>;
  /** App settings: user-defined workflows/skills override the built-in
   * defaults (null while loading → defaults). Frozen per mount. */
  settings: Settings | null;
  /** The project's resolved workdir (cwd), or null until the first send. */
  workdir: string | null;
  /** The codebase folder to analyze (foundation phase, D45) — separate from
   * the workdir. Null until the requirements form's folder answer arrives. */
  codebasePath: string | null;
  /** A saved session to rehydrate (view + continue), or null for a fresh chat. */
  initialSession: StoredSession | null;
  /** Requirements-form answers to send as the next turn (nonce-guarded).
   * `requirement` is the raw requirement-field answer (D78); `ragEnabled` =
   * the form's RAG switch (off → the rag step is skipped — D96). */
  answerSubmission: {
    wire: string;
    display: string;
    requirement: string;
    ragEnabled: boolean;
    nonce: number;
  } | null;
  /** True while the requirements form awaits the user — manual chat input is
   * blocked until the form is submitted (auto turns still run). */
  formPending: boolean;
  onHome: () => void;
  /** Start a fresh session (clears the chat, re-enables agent selection). */
  onNewSession: () => void;
  /** Open a saved session (parent remounts this panel with it). */
  onOpenSession: (s: StoredSession) => void;
  onOpenFile: (path: string) => void;
  /** The category's fixed option questions → canvas form (shown on entry). */
  onClarify: (questions: ClarifyQuestion[]) => void;
  /** RAG search results from the rag foundation step → canvas "검색 결과" tab.
   * `verdict` is the relevance judge's curated result (D70), or null when the
   * judge failed/parsed nothing (fail open — render the raw hits). */
  onRagResult: (query: string, hits: RagHit[], verdict: RagVerdict | null) => void;
  /** The optimized prompt from the first work turn → canvas "프롬프트" tab (D78). */
  onPromptResult: (text: string) => void;
  /** Mirror streaming state up (canvas form disables while streaming). */
  onStreamingChange: (streaming: boolean) => void;
  /** Mirror the workflow step progress up (canvas 산출물 tab shows per-artifact
   * status, D58) — ownership stays here, same pattern as onStreamingChange. */
  onStepProgress?: (progress: StepProgress[] | null) => void;
  /** Mirror the session title up (summarized from the requirement — D97). */
  onSessionTitle?: (title: string) => void;
  /** Navigate to the Agents view (undetected-agent onboarding, D57). */
  onOpenAgents?: () => void;
  /** Open the canvas 지식 저장 panel (workflow-completion banner, D59) with the
   * session's agent/model (the summary turn runs on them). */
  onOpenKnowledgeSave?: (ctx: { agentId: string; model: string | null }) => void;
}) {
  const [messages, setMessages] = useState<ChatMessage[]>(
    () => (initialSession?.messages as ChatMessage[] | undefined) ?? [],
  );
  const [input, setInput] = useState("");
  const inputRef = useAutoGrow(input, 160);
  const [streaming, setStreaming] = useState(false);
  // A saved session whose agent no longer exists (`aipro`, removed in D83)
  // reopens on a fallback agent with a reset model instead of failing every
  // turn with "unknown agent".
  const [sessionAgentRemoved] = useState(
    () =>
      !!initialSession?.agentId &&
      agents.length > 0 &&
      !agents.some((a) => a.id === initialSession.agentId),
  );
  const [agentId, setAgentId] = useState<string>(() =>
    sessionAgentRemoved
      ? fallbackForRemovedAgent(agents, detected)
      : (initialSession?.agentId ?? initialAgentId ?? defaultAgentId(agents, detected)),
  );
  const [model, setModel] = useState<string>(() =>
    sessionAgentRemoved ? "default" : (initialSession?.model ?? initialModel ?? "default"),
  );
  // A Home-picked agent is an explicit choice — the detection-sync effect below
  // must not override it with the auto default (D60).
  const agentPinnedRef = useRef(
    (!!initialSession?.agentId && !sessionAgentRemoved) || !!initialAgentId,
  );

  const runIdRef = useRef<string | null>(null);
  const sessionIdRef = useRef<string | null>(initialSession?.cliSessionId ?? null);
  const resumeRef = useRef(!!initialSession?.cliSessionId);
  const scrollRef = useRef<HTMLDivElement>(null);
  // The resolved project workdir (cwd). For a fresh auto project it's null until
  // the first send resolves it; for a chosen-folder / loaded project it's known
  // up front. Set once and reused for every turn.
  const resolvedWorkdirRef = useRef<string | null>(workdir);
  // Whether the project folder + manifest have been created. A loaded session's
  // project already exists; a fresh chat ensures once on the first send (even
  // when the workdir is already known, so the manifest is always written).
  const ensuredRef = useRef(!!initialSession);

  // Persistence: our own session-folder id (distinct from the CLI session id),
  // the conversation's creation time, and a live mirror of `messages` so we can
  // save the full turn from inside the streaming `end` handler.
  const persistIdRef = useRef<string | null>(initialSession?.id ?? null);
  const createdAtRef = useRef<number>(initialSession?.createdAt ?? 0);
  const messagesRef = useRef<ChatMessage[]>(messages);
  // Skip the agent-change reset on the initial mount when a saved session was
  // loaded, so its cli session id / resume state survive.
  const loadedRef = useRef(!!initialSession);

  // Built-in prompt-optimizer skill (D78): injected once per conversation on the
  // first work turn, independent of step arming. A loaded session has no first
  // turn → no injection.
  const promptSkillPendingRef = useRef(!initialSession);
  // Marks the turn whose `end` must parse a ```prompt block (like inflightStepRef).
  const promptInflightRef = useRef(false);
  // The current turn's text since its last tool call (D84). A remote agent's
  // document is its final answer after the read-only tool rounds, so this —
  // not the whole reply (which also holds pre-tool narration) — is what the
  // D67 path writes to `step.file`. Reset per send; cleared on tool events.
  const remoteTailRef = useRef("");

  // Category workflow orchestration (generalizes the clarify one-shot). `WF` is
  // the category's ordered steps (user-configured via settings, else the
  // built-in default — frozen per mount; the sessionNonce remount resets it);
  // `stepIndexRef` is the current step, `armed` once per step to inject its
  // skills + instruction; `inflightStepRef` marks the step whose result we
  // parse on `end`. A loaded session starts past the end (plain chat, no
  // re-injection — transient like clarify). The agent-change reset effect
  // intentionally does not touch these.
  const WF = useRef(runtimeWorkflowFor(category, settings)).current;
  const stepIndexRef = useRef(initialSession ? WF.length : 0);
  const stepArmedRef = useRef(!initialSession && WF.length > 0);
  const inflightStepRef = useRef<number | null>(null);
  const lastAnswerNonceRef = useRef(answerSubmission?.nonce ?? 0);
  // Option-first entry (D36/D95) + per-step skills (D40). The category's fixed
  // option catalog (led by the requirement field) is shown on entry (not a
  // workflow step); each step's skills are injected on the turn that runs it
  // (session agents get each skill once — deduped via `injectedSkillIdsRef` —
  // sessionless agents re-see them in the transcript).
  const optionQuestions = useRef(optionsFor(category, settings)).current;
  const skillMapRef = useRef(resolveSkills(settings));
  const injectedSkillIdsRef = useRef(new Set<string>());
  // The form's requirement answer (the user's request — D95): feeds the RAG
  // query and the repository map's focus terms.
  const requirementRef = useRef("");
  // The form's RAG switch (D96): false → the rag step is skipped.
  const ragEnabledRef = useRef(true);
  const bootedRef = useRef(!!initialSession); // the options boot runs once (never for a loaded session)
  // Session title (D97): the requirement's heuristic title right away, then
  // the isolated summary turn's title. `titlePendingRef` holds the requirement
  // until send() has a cwd to run that turn in; `projectTitleOwnedRef` = this
  // chat created the project (its manifest title is ours to replace — a new
  // session in an existing/renamed project leaves the project title alone).
  const sessionTitleRef = useRef(initialSession?.title ?? "");
  const [sessionTitle, setSessionTitle] = useState(initialSession?.title ?? "");
  const titlePendingRef = useRef<string | null>(null);
  const projectTitleOwnedRef = useRef(false);
  const titleCancelRef = useRef<(() => void) | null>(null);
  const aliveRef = useRef(true);
  // Foundation phase (D44/D45): the codebase folder (form answer, prop-fed) and
  // the extra readable dirs sent with every run (codebase + armed skill dirs —
  // claude's `--add-dir` is per-invocation). The answers wire feeds the RAG
  // query; a preflight fetch can be aborted by Stop before the run spawns.
  const codebasePathRef = useRef<string | null>(codebasePath);
  const codebasePersistedRef = useRef(!!initialSession); // manifest already has it
  const skillDirsRef = useRef(new Set<string>());
  // Stored-artifact read access (D59): once the knowledge preflight injected an
  // artifact entry's document index, the knowledge `artifacts` root rides
  // extraDirs for the rest of the conversation (one root dir — no per-entry
  // CLI-arg bloat) so the agent can read the full originals on demand.
  const knowledgeDirsRef = useRef(new Set<string>());
  const lastAnswersWireRef = useRef("");
  const preflightAbortRef = useRef(false);
  // Cancel handle for the in-flight RAG relevance judge turn (D70) — an isolated
  // run whose id isn't in runIdRef, so Stop must reach it through this ref.
  const ragJudgeCancelRef = useRef<(() => void) | null>(null);
  // Workflow completion (D59): true once any generative step produced a file;
  // reaching the terminal chat step then proposes saving the artifacts as
  // knowledge — once per session, dismissible.
  const producedFileRef = useRef(false);
  const completionFiredRef = useRef(false);
  // Step hand-off (D86): runtime step index → the absolute path of the file it
  // produced (+ its extracted Learnings for code-analysis steps; `undefined` =
  // not read yet — CLI docs are read lazily by the next step's preflight, a
  // remote doc's Learnings come from the in-memory reply). Later generative
  // steps get this index + the Learnings injected, so they reuse the analysis
  // instead of re-exploring (sessionless transcripts drop the step wires).
  const producedDocsRef = useRef(new Map<number, { path: string; learnings?: string | null }>());
  // The codebase step's non-AI repository map, saved as an artifact by its
  // preflight (D94): later steps get its path in the hand-off index.
  const repoMapRef = useRef<{ stepIdx: number; path: string } | null>(null);
  const [kbProposal, setKbProposal] = useState(false);
  // Auto-advance: a generative step queues the next step's turn here; a
  // nonce-guarded effect fires it once (never call send() from the stream
  // handler's closure — it would capture stale `messages`).
  const [autoTurn, setAutoTurn] = useState<{ display: string; nonce: number } | null>(null);
  const lastAutoNonceRef = useRef(0);

  // Persistent workflow progress for the stepper strip (D57): one status per
  // runtime step, kept in lockstep with the step cursor. Null when there is no
  // multi-step workflow to show (loaded sessions, plain chat categories).
  const [stepProgress, setStepProgress] = useState<StepProgress[] | null>(() =>
    initialSession || !WF.some((s) => isGenerative(s.kind))
      ? null
      : WF.map((s) => ({ id: s.id, name: s.name?.trim() || s.id, status: "pending" as const })),
  );
  const setStepStatusAt = (i: number, status: StepProgressStatus) =>
    setStepProgress((prev) =>
      prev ? prev.map((s, idx) => (idx === i ? { ...s, status } : s)) : prev,
    );

  // The last real turn, kept for same-session retry after a
  // failed turn (D57). `stepIndex` restores the workflow cursor on retry.
  const lastTurnRef = useRef<{
    text: string;
    opts?: { display?: string; system?: boolean };
    stepIndex: number | null;
  } | null>(null);

  // Auto-retry counter for the current in-flight step (D68): resets implicitly
  // when a different step starts failing. Survives within a session; a remount
  // (new session) clears it.
  const stepRetryRef = useRef<{ step: number; count: number }>({ step: -1, count: 0 });

  // Transient UI-level failure line (e.g. a session that failed to open) —
  // shown under the header instead of being silently swallowed (D57).
  const [uiError, setUiError] = useState<string | null>(null);

  // Auto-scroll only while the user is pinned near the bottom; otherwise show
  // a jump-to-latest button instead of yanking the view down on every delta.
  const pinnedRef = useRef(true);
  const [showJump, setShowJump] = useState(false);

  const started = messages.length > 0;
  const sessionCapable = SESSION_AGENTS.includes(agentId);
  // Remote (HTTP) agents (FabriX) can read files through the app's read-only
  // tool loop but never write, so the app persists their document-step output
  // itself (D67/D84). `agentId` is locked once a turn starts, so this is
  // stable across the turn.
  const isRemote = detected[agentId]?.source === "remote";

  // History popover.
  const [historyOpen, setHistoryOpen] = useState(false);
  const [history, setHistory] = useState<SessionMeta[]>([]);

  useEffect(() => {
    codebasePathRef.current = codebasePath;
  }, [codebasePath]);

  // The latest onOpenFile: the parent routes workflow files to the 산출물 hub
  // by its resolved workdir, which a send() closure from before the first
  // turn's ensure_project does not see yet (→ a plain file tab instead).
  const onOpenFileRef = useRef(onOpenFile);
  useEffect(() => {
    onOpenFileRef.current = onOpenFile;
  }, [onOpenFile]);

  useEffect(() => {
    onStreamingChange(streaming);
  }, [streaming, onStreamingChange]);

  useEffect(() => {
    onStepProgress?.(stepProgress);
  }, [stepProgress, onStepProgress]);

  // Cancel any in-flight run when this panel unmounts (new session, open a saved
  // session, or leave home) so we don't leak a background agent process. A
  // late title summary must not retitle the next session (`aliveRef`).
  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
      titleCancelRef.current?.();
      if (runIdRef.current) cancelRun(runIdRef.current).catch(() => {});
    };
  }, []);

  // Keep the default agent in sync as detection results arrive (until the
  // conversation starts — after that the agent is locked). Skipped when the
  // agent was explicitly picked (Home start settings / loaded session / user select).
  useEffect(() => {
    if (!started && !agentPinnedRef.current) setAgentId(defaultAgentId(agents, detected));
  }, [agents, detected, started]);

  // Reset session state whenever the (pre-conversation) agent changes: claude
  // mints its own session id up front (so cancel-then-continue still resumes);
  // codex starts null and captures its thread id from the stream. Skipped once
  // on mount for a loaded session (keeps its resume state).
  useEffect(() => {
    if (loadedRef.current) {
      loadedRef.current = false;
      return;
    }
    sessionIdRef.current = agentId === "claude" ? crypto.randomUUID() : null;
    resumeRef.current = false;
  }, [agentId]);

  const models = detected[agentId]?.models ?? [];

  // Ensure a concrete model is selected when the current value isn't valid for
  // the chosen agent. FabriX (D64/D84) lists real modelIds with no "default"
  // option (its configured default first) — snap to the first model (the
  // backend also resolves a missing/unknown id to the default).
  // CLI agents include a "default" option, so this never fires for them.
  useEffect(() => {
    if (started) return;
    if (models.length > 0 && !models.some((m) => m.id === model)) {
      setModel(models[0].id);
    }
  }, [models, model, started]);

  useEffect(() => {
    if (!pinnedRef.current) return; // reading history — don't yank the view down
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages]);

  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const pinned = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
    pinnedRef.current = pinned;
    setShowJump(!pinned);
  };

  const jumpToBottom = () => {
    pinnedRef.current = true;
    setShowJump(false);
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  };

  /** Synchronously commit a messages change: compute from the live ref (never
   * a stale render snapshot), update the ref, then set state. Channel events
   * arrive back-to-back — a passive `useEffect` ref sync lags a commit behind,
   * which used to let the `end` handler clobber the just-streamed reply with an
   * empty snapshot and persist it empty. Every message mutation goes through
   * here so `messagesRef.current` is always current. */
  const mutateMessages = (fn: (prev: ChatMessage[]) => ChatMessage[]): ChatMessage[] => {
    const next = fn(messagesRef.current);
    messagesRef.current = next;
    setMessages(next);
    return next;
  };

  const updateLastAssistant = (fn: (m: ChatMessage) => ChatMessage) => {
    mutateMessages((prev) => {
      const next = [...prev];
      for (let i = next.length - 1; i >= 0; i--) {
        if (next[i].role === "assistant") {
          next[i] = fn(next[i]);
          break;
        }
      }
      return next;
    });
  };

  /** Persist the conversation to disk (creates project/session folders on first
   * save). Called on the first turn and after every completed turn. */
  const persist = (msgs: ChatMessage[] = messagesRef.current) => {
    if (!projectId || msgs.length === 0) return;
    if (!persistIdRef.current) persistIdRef.current = crypto.randomUUID();
    if (createdAtRef.current === 0) createdAtRef.current = Date.now();
    const firstUser = msgs.find((m) => m.role === "user");
    const title =
      sessionTitleRef.current.trim() || (firstUser?.content ?? "").trim().slice(0, 60) || "새 대화";
    // Never persist a mid-stream flag — a reloaded session must not look live.
    const clean = msgs.map((m) => (m.streaming ? { ...m, streaming: false } : m));
    const session: StoredSession = {
      id: persistIdRef.current,
      title,
      agentId,
      model,
      category,
      cliSessionId: sessionIdRef.current,
      createdAt: createdAtRef.current,
      updatedAt: 0, // stamped by the backend
      messageCount: clean.length,
      messages: clean as unknown[],
    };
    void saveSession(projectId, session).catch(() => {});
  };

  /** Set the session title (header, history, saved session — D97). */
  const applySessionTitle = (title: string) => {
    sessionTitleRef.current = title;
    setSessionTitle(title);
    onSessionTitle?.(title);
  };

  /** Summarize the requirement into the session title with an isolated agent
   * turn (D97), next to the first work turn. On success: retitle the session
   * (re-saved now unless a run is streaming — its `end` save picks up the ref)
   * and — when this chat created the project — the project too. Failure keeps
   * the heuristic title. */
  const startTitleSummary = (requirement: string, cwd: string) => {
    const job = summarizeTitle({
      agentId,
      model: model === "default" ? null : model,
      cwd,
      requirement,
      categoryLabel: categoryLabel(category),
    });
    titleCancelRef.current = job.cancel;
    void job.promise.then((title) => {
      titleCancelRef.current = null;
      if (!title || !aliveRef.current || title === sessionTitleRef.current) return;
      applySessionTitle(title);
      if (!runIdRef.current) persist();
      if (projectTitleOwnedRef.current) void setProjectTitle(projectId, title).catch(() => {});
    });
  };

  const handleEvent = (ev: RunEvent) => {
    switch (ev.type) {
      case "status":
        // claude echoes the minted id; codex/gemini surface their own here.
        if (ev.sessionId) sessionIdRef.current = ev.sessionId;
        break;
      case "textDelta":
        remoteTailRef.current += ev.delta;
        updateLastAssistant((m) => ({ ...m, content: m.content + ev.delta }));
        break;
      case "stdout":
        updateLastAssistant((m) => ({ ...m, content: m.content + ev.chunk }));
        break;
      case "thinkingDelta":
        updateLastAssistant((m) => ({ ...m, thinking: m.thinking + ev.delta }));
        break;
      case "toolUse":
        remoteTailRef.current = ""; // the document is what comes after the last tool call
        updateLastAssistant((m) => ({
          ...m,
          events: [...m.events, { kind: "toolUse", id: ev.id, name: ev.name, input: ev.input }],
        }));
        break;
      case "toolResult":
        remoteTailRef.current = "";
        updateLastAssistant((m) => ({
          ...m,
          events: [
            ...m.events,
            { kind: "toolResult", toolUseId: ev.toolUseId, content: ev.content, isError: ev.isError },
          ],
        }));
        break;
      case "usage":
        updateLastAssistant((m) => ({
          ...m,
          events: [
            ...m.events,
            { kind: "usage", inputTokens: ev.inputTokens, outputTokens: ev.outputTokens },
          ],
        }));
        break;
      case "error":
        updateLastAssistant((m) => ({ ...m, error: ev.message }));
        break;
      case "notice":
        // e.g. FabriX pacing to its per-minute call quota (D91). Not part of the
        // reply text (the remote document body is unaffected).
        updateLastAssistant((m) => ({
          ...m,
          events: [...m.events, { kind: "notice", text: ev.message }],
        }));
        break;
      case "end": {
        setStreaming(false);
        runIdRef.current = null;

        const stepIdx = inflightStepRef.current;
        inflightStepRef.current = null;

        // Built-in prompt-optimizer (D78): strip the ```prompt block out of the
        // visible reply → canvas "프롬프트" tab, so it isn't re-sent in the
        // sessionless transcript (which would re-trigger the block) or re-shown.
        // Tag match only — no fallback (the rest of the reply is the real work
        // output). A failed/canceled turn skips parsing (same as skill dedupe —
        // the wire already reached the agent, so we don't rewind on stream fail).
        if (promptInflightRef.current) {
          promptInflightRef.current = false;
          if (ev.status === "succeeded") {
            const msgs = messagesRef.current; // live via mutateMessages
            for (let i = msgs.length - 1; i >= 0; i--) {
              if (msgs[i].role !== "assistant") continue;
              const block = parsePromptBlock(msgs[i].content);
              if (block) {
                const idx = i;
                mutateMessages((prev) =>
                  prev.map((m, j) =>
                    j === idx ? { ...m, content: stripPromptBlock(m.content) } : m,
                  ),
                );
                onPromptResult(block);
              }
              break;
            }
          }
        }

        // Clear the mid-stream flag on the whole list (matches persist()).
        mutateMessages((prev) =>
          prev.map((m) => (m.streaming ? { ...m, streaming: false } : m)),
        );

        if (stepIdx != null && ev.status === "succeeded") {
          const step = WF[stepIdx];
          if (isGenerative(step.kind)) {
            // Generative step (search/document/foundation): open the produced
            // file (if any), then AUTO-ADVANCE to the next generative step;
            // stop before a terminal chat step.
            setStepStatusAt(stepIdx, "done");
            if (step.file && resolvedWorkdirRef.current) {
              producedFileRef.current = true; // an artifact exists → completion may propose saving it (D59)
              const abs = joinWorkdirPath(resolvedWorkdirRef.current, step.file);
              if (isRemote) {
                // FabriX can only *read* files (tool loop, D84) — persist the
                // reply ourselves, then open it (refreshNonce bump → existence
                // probe finds it → FileViewer mounts). Write first, open after
                // (D67). Only the text after the turn's last tool call is the
                // document (pre-tool narration is not), and the first work
                // turn's ```prompt block / PROMPT_NOTE is removed. Never write
                // an empty file.
                const body =
                  extractDocBody(stripPromptBlock(remoteTailRef.current, "")) ||
                  extractDocBody(
                    stripPromptBlock(lastAssistantContent(messagesRef.current), "").replace(PROMPT_NOTE, ""),
                  );
                if (body) {
                  // Hand-off (D86): the reply IS the document — take its
                  // Learnings now (the write below is not awaited, so the next
                  // step must not read the file back).
                  const learnings = codeAnalysisMode(step) ? extractLearnings(body) : null;
                  producedDocsRef.current.set(stepIdx, { path: abs, learnings });
                  if (codeAnalysisMode(step) === "orient") rememberCodebaseLearnings(learnings);
                  void writeFile(abs, body)
                    .then(() => onOpenFileRef.current(abs))
                    .catch((e) =>
                      mutateMessages((prev) => [...prev, note(`산출물 파일 저장 실패 — ${String(e)}`)]),
                    );
                } else {
                  mutateMessages((prev) => [...prev, note("응답이 비어 있어 산출물 파일을 저장하지 않았습니다.")]);
                }
              } else {
                // CLI agents wrote the file with their own tools; its Learnings
                // are read lazily by the next step's preflight (D86).
                const entry: { path: string; learnings?: string | null } = {
                  path: abs,
                  learnings: codeAnalysisMode(step) ? undefined : null,
                };
                producedDocsRef.current.set(stepIdx, entry);
                if (codeAnalysisMode(step) === "orient") {
                  // Cache the orientation for later projects (D90) right away
                  // (the next step may be rag/knowledge, which never reads it).
                  void readFile(abs)
                    .then((md) => {
                      entry.learnings = extractLearnings(md);
                      rememberCodebaseLearnings(entry.learnings);
                    })
                    .catch(() => {});
                }
                onOpenFileRef.current(abs);
              }
            }
            advanceTo(stepIdx + 1);
          }
          // chat kind: terminal — nothing to advance.
        } else if (
          stepIdx != null &&
          ev.status === "failed" &&
          isGenerative(WF[stepIdx].kind) &&
          isEmptyReplyFailure(lastAssistantError(messagesRef.current))
        ) {
          // FabriX came back empty even after its in-turn re-asks (D93). A
          // whole-step retry would redo the exploration against the per-minute
          // call quota into the same wall — skip this step softly (no artifact)
          // and keep the workflow going. Its HTML render sub-step (D47) has
          // nothing to render, so it is skipped with it.
          stepRetryRef.current = { step: -1, count: 0 };
          const skipped = WF[stepIdx];
          setStepStatusAt(stepIdx, "skipped");
          let nextIdx = stepIdx + 1;
          while (WF[nextIdx]?.id === `${skipped.id}-html`) setStepStatusAt(nextIdx++, "skipped");
          // The workflow did not stop: the skip note replaces the red error box.
          updateLastAssistant((m) => ({ ...m, error: undefined }));
          mutateMessages((prev) => [
            ...prev,
            note(
              `'${skipped.name}' 단계의 FabriX 응답이 비어 있어 이 단계는 건너뛰고 다음 단계로 진행합니다${
                skipped.file ? " (산출물 없음)" : ""
              }.`,
            ),
          ]);
          advanceTo(nextIdx);
        } else if (stepIdx != null) {
          // A generative step's turn failed mid-workflow. If the failure looks
          // transient (stream/timeout/network — D68) and the per-step retry
          // budget is left, drop the failed pair and
          // AUTO-RETRY the same step (via the auto-turn nonce) before giving up.
          const attempt = stepRetryRef.current.step === stepIdx ? stepRetryRef.current.count + 1 : 1;
          const retryable =
            ev.status === "failed" &&
            isGenerative(WF[stepIdx].kind) &&
            attempt <= MAX_STEP_RETRIES &&
            isTransientFailure(lastAssistantError(messagesRef.current));
          if (retryable) {
            stepRetryRef.current = { step: stepIdx, count: attempt };
            // Drop the failed pair so the (sessionless) transcript stays clean.
            mutateMessages((prev) => {
              const next = [...prev];
              if (next.length && next[next.length - 1].role === "assistant") next.pop();
              if (next.length && next[next.length - 1].role === "user") next.pop();
              return next;
            });
            stepIndexRef.current = stepIdx;
            stepArmedRef.current = true;
            setStepStatusAt(stepIdx, "pending");
            setAutoTurn((s) => ({
              display: `${progressLabel(stepIdx, WF)} · 일시적 오류로 재시도 중 (${attempt}/${MAX_STEP_RETRIES})`,
              nonce: (s?.nonce ?? 0) + 1,
            }));
          } else {
            // Canceled / non-transient / budget exhausted → halt, drop to plain chat.
            const exhausted = ev.status === "failed" && attempt > MAX_STEP_RETRIES;
            stepRetryRef.current = { step: -1, count: 0 };
            stepIndexRef.current = WF.length;
            stepArmedRef.current = false;
            setStepStatusAt(stepIdx, "halted");
            mutateMessages((prev) => [
              ...prev,
              note(
                ev.status === "canceled"
                  ? "작업을 중지했습니다 — 이후에는 일반 대화로 진행됩니다."
                  : exhausted
                    ? `여러 번(${MAX_STEP_RETRIES}회) 재시도했지만 실패했습니다 — '다시 시도'로 재개하거나 일반 대화로 진행하세요.`
                    : "오류로 단계 진행을 중단했습니다 — '다시 시도'로 재개하거나 일반 대화로 진행하세요.",
              ),
            ]);
          }
        } else if (ev.status === "canceled") {
          // Plain-chat cancel: leave a consistent confirmation (the partial
          // reply above stays as-is).
          mutateMessages((prev) => [...prev, note("응답 생성을 중지했습니다.")]);
        }

        persist(messagesRef.current); // save the completed turn (incl. captured cli session id)
        break;
      }
    }
  };

  // Re-arm the in-flight step (a spawn/ensure failure) so the next message
  // retries it instead of silently skipping ahead.
  const rearmStep = () => {
    if (inflightStepRef.current != null) {
      stepIndexRef.current = inflightStepRef.current;
      stepArmedRef.current = true;
      setStepStatusAt(inflightStepRef.current, "pending");
      inflightStepRef.current = null;
    }
  };

  /** Move the workflow cursor to `nextIdx` after a step ended (done or softly
   * skipped): auto-fire it when it is generative, else stop at the terminal
   * chat and wait for the user. */
  const advanceTo = (nextIdx: number) => {
    stepIndexRef.current = nextIdx;
    const nextStep = WF[nextIdx];
    if (nextStep && isGenerative(nextStep.kind)) {
      stepArmedRef.current = true;
      setAutoTurn((s) => ({
        display: progressLabel(nextIdx, WF),
        nonce: (s?.nonce ?? 0) + 1,
      }));
    } else {
      stepArmedRef.current = false; // terminal chat → wait for the user
      if (nextStep) setStepStatusAt(nextIdx, "active");
      maybeProposeKnowledgeSave();
    }
  };

  /** Terminal chat reached (D59): once per session, when at least one step
   * actually produced a file, offer to save the artifacts as knowledge. Called
   * from both terminal-reach sites — the end handler's auto-advance stop and
   * the preflight-skip chain. */
  const maybeProposeKnowledgeSave = () => {
    if (completionFiredRef.current || !producedFileRef.current) return;
    completionFiredRef.current = true;
    setKbProposal(true);
  };

  /** A centered workflow note (skip/stop announcements). */
  const note = (content: string): ChatMessage => ({
    role: "user",
    content,
    thinking: "",
    events: [],
    system: true,
  });

  /** Surface a turn that failed before it could spawn (e.g. `ensure_project`),
   * as a normal message pair with the error on the assistant side. */
  const failTurn = (display: string, system: boolean | undefined, error: string) => {
    mutateMessages((prev) => [
      ...prev,
      { role: "user", content: display, thinking: "", events: [], system },
      { role: "assistant", content: "", thinking: "", events: [], error },
    ]);
  };

  /** Step preflight (D44/D86): resolve the extra wire context for this turn,
   * or decide to skip the step (unconfigured / empty / failed — the foundation
   * never blocks the flow). `rag` and `knowledge` fetch before the agent turn;
   * other generative steps get the earlier steps' hand-off (never skipped). */
  const stepPreflight = async (
    step: StepDef,
    cwd: string,
    stepIdx: number,
  ): Promise<{ context?: string; skip?: string; note?: string }> => {
    if (step.kind === "rag") {
      if (!settings?.rag?.endpoint?.trim()) {
        return { skip: "RAG가 설정되지 않아 사내 문서 검색 단계를 건너뜁니다. (지식 화면에서 등록)" };
      }
      if (!ragEnabledRef.current) {
        return { skip: "요구사항에서 사내 문서 RAG 검색을 꺼 두어 이 단계를 건너뜁니다." };
      }
      try {
        const query = buildRagQuery(requirementRef.current, lastAnswersWireRef.current);
        const hits = await ragSearch(query);
        if (preflightAbortRef.current) return {};
        if (!hits.length) return { skip: "관련 사내 문서를 찾지 못해 검색 단계를 건너뜁니다." };
        // Relevance judge (D70): an isolated turn decides whether these results
        // actually help this task (and curates them). Not relevant → skip (no
        // panel, no injection). Judge failure (verdict null) → fail open.
        const judge = judgeRagRelevance({
          agentId,
          model: model === "default" ? null : model,
          cwd,
          task: query,
          hits,
        });
        ragJudgeCancelRef.current = judge.cancel;
        const verdict = await judge.promise;
        ragJudgeCancelRef.current = null;
        if (preflightAbortRef.current) return {};
        if (verdict && verdict.relevant === false) {
          return { skip: "검색된 사내 문서가 이번 작업과 관련이 적어 표시하지 않습니다." };
        }
        onRagResult(query, hits, verdict ?? null);
        return { context: formatRagContext(hits) };
      } catch (e) {
        return { skip: `사내 문서 검색 단계를 건너뜁니다 — ${ragUserError(String(e))}` };
      }
    }
    if (step.kind === "knowledge") {
      try {
        const [entries, kbRoot] = await Promise.all([
          listKnowledge(),
          getKnowledgeRoot().catch(() => null), // root fetch failure → summaries without the index
        ]);
        if (preflightAbortRef.current) return {};
        if (!entries.length) {
          return { skip: "등록된 사내 지식이 없어 지식 반영 단계를 건너뜁니다. (지식 화면에서 등록)" };
        }
        // Artifact entries (D59): the injected context lists their documents'
        // absolute paths; grant read access to the store for the rest of the
        // conversation so the agent can open the full originals.
        const artifactsRoot = kbRoot ? `${kbRoot.replace(/[\\/]+$/, "")}\\artifacts` : null;
        if (artifactsRoot && entries.some((e) => e.kind === "artifact" && e.files?.length)) {
          knowledgeDirsRef.current.add(artifactsRoot);
        }
        return { context: formatKnowledgeContext(entries, artifactsRoot) };
      } catch (e) {
        return { skip: `지식 베이스를 읽지 못해 단계를 건너뜁니다 — ${String(e)}` };
      }
    }
    // Every other generative step (D86): hand off what earlier steps produced
    // — the document index + the code-analysis Learnings. Fail-open (never
    // skip): a missing/unreadable doc just contributes no Learnings. The HTML
    // render sub-step only re-presents its predecessor, so it gets none.
    if (isGenerative(step.kind) && !step.skillIds.includes("html-render")) {
      const parts = [formatHandoffContext(await producedDocsBefore(stepIdx))];
      let cacheNote: string | undefined;
      // Codebase step: Learnings an earlier project cached for this codebase
      // (D90) + the app-computed repository map (D88) — the only way CLI agents
      // (which can't call app tools) get the static analysis. Both fetched in
      // parallel; fail-open: no folder / an error just means none.
      const cb = codebasePathRef.current;
      if (step.kind === "codebase" && cb) {
        const terms = focusTermsFrom(`${requirementRef.current}\n${lastAnswersWireRef.current}`);
        // FabriX pays one quota-limited call per lookup round (D91): give it the
        // larger map (more key outlines + the request's SQL bodies).
        const [map, cached] = await Promise.all([
          codebaseMap(cb, terms, isRemote ? "extended" : undefined).catch(() => null),
          getCodebaseLearnings(cb, category).catch(() => null),
        ]);
        if (preflightAbortRef.current) return {};
        if (map) {
          // Keep the map as an artifact too (D94) — users read it alongside the
          // AI's analysis. A write failure only loses the file, not the prompt.
          const mapAbs = joinWorkdirPath(cwd, REPO_MAP_FILE);
          const saved = await writeFile(
            mapAbs,
            repoMapDocument(map, { codebase: cb, terms, extended: isRemote, generatedAt: new Date() }),
          ).then(
            () => true,
            () => false,
          );
          if (preflightAbortRef.current) return {};
          if (saved) {
            repoMapRef.current = { stepIdx, path: mapAbs };
            producedFileRef.current = true;
            onOpenFileRef.current(mapAbs);
          }
          const shown =
            PLAIN_AGENTS.includes(agentId) && map.length > PLAIN_MAP_MAX
              ? `${map.slice(0, PLAIN_MAP_MAX)}\n…(프롬프트 길이 제한으로 저장소 지도 일부만 첨부${saved ? ` — 전체: ${mapAbs}` : ""})`
              : map;
          const savedNote = saved
            ? ` 이 지도는 ${mapAbs}에 별도 산출물로 저장되므로, 산출 문서에 그대로 옮겨 적지 말고 검증·해석한 결과만 쓰세요.`
            : "";
          parts.unshift(
            `[사전 계산된 저장소 지도 — 앱의 비AI 정적 분석 결과(근사치). 탐색 출발점으로 쓰고, 이번 요청의 핵심 경로는 실제 파일로 검증하세요.${savedNote}]\n${shown}`,
          );
        }
        if (cached) {
          parts.unshift(formatCachedLearnings(cached));
          cacheNote = cachedLearningsNote(cached);
        }
      }
      // FabriX source analysis (D91): its sessionless transcript skips the
      // codebase step's wire, so the map is gone — re-attach the request-specific
      // part (hits, chains, SQL bodies, key outlines) instead of re-exploring.
      if (isRemote && cb && codeAnalysisMode(step) === "focus") {
        const terms = focusTermsFrom(`${requirementRef.current}\n${lastAnswersWireRef.current}`);
        const focusMap = await codebaseMap(cb, terms, "focus").catch(() => null);
        if (preflightAbortRef.current) return {};
        if (focusMap) {
          parts.unshift(
            `[사전 계산된 변경 대상 지도 — 앱의 비AI 정적 분석 결과(근사치). 여기 있는 구조·SQL 본문은 다시 조회하지 말고, 필요한 심볼만 도구로 확인하세요]\n${focusMap}`,
          );
        }
      }
      const context = parts.filter(Boolean).join("\n\n");
      return context ? { context, note: cacheNote } : {};
    }
    return {};
  };

  /** Cache an orientation step's Learnings for later projects on the same
   * codebase (D90). Side effect only — failures are ignored. */
  const rememberCodebaseLearnings = (learnings: string | null | undefined) => {
    const cb = codebasePathRef.current;
    if (learnings && cb) void saveCodebaseLearnings(cb, category, learnings, projectId).catch(() => {});
  };

  /** Documents produced by the steps before `stepIdx`, in step order (the
   * repository map right after its codebase step's document — D94). Reads a
   * CLI-written code-analysis doc on first use to extract its Learnings
   * (cached; a read failure caches "none"). */
  const producedDocsBefore = async (stepIdx: number): Promise<ProducedDoc[]> => {
    const out: { order: number; doc: ProducedDoc }[] = [];
    const entries = [...producedDocsRef.current.entries()].filter(([i]) => i < stepIdx);
    for (const [i, doc] of entries) {
      if (doc.learnings === undefined) {
        doc.learnings = await readFile(doc.path)
          .then((md) => extractLearnings(md))
          .catch(() => null);
      }
      out.push({ order: i, doc: { step: WF[i], path: doc.path, learnings: doc.learnings } });
    }
    const map = repoMapRef.current;
    if (map && map.stepIdx < stepIdx) {
      out.push({
        order: map.stepIdx + 0.5,
        doc: { step: WF[map.stepIdx], path: map.path, learnings: null, label: REPO_MAP_NAME },
      });
    }
    return out.sort((a, b) => a.order - b.order).map((e) => e.doc);
  };

  /** Whether the hand-off preflight for `stepIdx` must read a file (→ the
   * preflight runs inside the busy bracket, so a concurrent send can't spawn). */
  const handoffNeedsRead = (stepIdx: number): boolean =>
    [...producedDocsRef.current.entries()].some(([i, d]) => i < stepIdx && d.learnings === undefined);

  /** Absolute-path context injected on armed generative turns (D52): where
   * outputs go (workdir) and — when a codebase folder was selected — where the
   * sources live, with an explicit search-root directive so the agent starts
   * exploring there instead of the (output-only) workdir. */
  const pathContext = (step: StepDef, cwd: string): string => {
    const lines = [`작업 폴더(산출물 저장 위치, 절대경로): ${cwd}`];
    const cb = codebasePathRef.current;
    if (cb) {
      lines.push(`분석 대상 코드베이스 폴더(절대경로): ${cb}`);
      lines.push(
        step.kind === "codebase"
          ? "모든 소스 파일 탐색·검색·읽기를 반드시 위 코드베이스 폴더(절대경로)에서 시작하세요. 작업 폴더에서 소스를 찾지 마세요 — 작업 폴더는 산출물 저장 전용입니다."
          : "소스코드는 위 코드베이스 폴더에서 읽고, 산출물 문서는 작업 폴더에서 읽고 쓰세요.",
      );
    } else if (step.kind === "codebase") {
      lines.push("분석 대상 코드베이스 폴더가 지정되지 않았습니다. 작업 폴더의 소스를 대신 분석하세요.");
    }
    return lines.join("\n");
  };

  /** Send one turn. Resolves to `true` when the turn was handled — spawned,
   * deliberately consumed (skip/stop/nothing-to-send), or attempted and its
   * failure surfaced in the chat. Resolves to `false` only when nothing
   * happened at all (still streaming / blocked by the pending form / empty),
   * so the caller may retry. The nonce-guarded effects only consume their
   * nonce on `true`; a submission racing an in-flight turn is therefore
   * retried when streaming ends instead of being silently lost (D55). */
  const send = async (text: string, opts?: { display?: string; system?: boolean }): Promise<boolean> => {
    const prompt = text.trim();
    if (streaming) return false;
    // While the requirements form awaits the user, block manual sends — only
    // auto-advance turns (system) may run.
    if (formPending && !opts?.system) return false;
    // Block an empty manual send before consuming the step arm; auto-advance
    // turns (system) legitimately carry no user text (the wire is built from
    // the injected instruction).
    if (!prompt && !opts?.system) return false;

    // Resolve the working folder up front (first send of a fresh chat creates
    // the project folder + manifest). Doing this before the preflight/wire
    // assembly lets the path context and preflight see the absolute workdir on
    // the very first turn (D52).
    let cwd = resolvedWorkdirRef.current;
    if (!ensuredRef.current) {
      try {
        // Title a new project by the request (the session title — D97), not
        // the answer/instruction wire. An existing project keeps its title.
        const title = sessionTitleRef.current || categoryLabel(category);
        const project = await ensureProject(
          projectId,
          cwd ?? "",
          title,
          category,
          codebasePathRef.current,
        );
        cwd = project.workdir;
        resolvedWorkdirRef.current = cwd;
        ensuredRef.current = true;
        projectTitleOwnedRef.current = project.title === title;
        onResolveWorkdir(cwd);
      } catch (e) {
        failTurn(opts?.display ?? prompt, opts?.system, String(e));
        return true; // attempted & surfaced — don't auto-retry
      }
    }
    if (!cwd) {
      failTurn(opts?.display ?? prompt, opts?.system, "작업 폴더를 확인할 수 없습니다.");
      return true;
    }

    // The requirement's summary title (D97) runs alongside the first work turn
    // (it needs the cwd resolved above) and never blocks it.
    if (titlePendingRef.current) {
      const requirement = titlePendingRef.current;
      titlePendingRef.current = null;
      startTitleSummary(requirement, cwd);
    }

    // This turn's workflow step + its skills. If armed, prepend the step's skill
    // bodies + instruction to the wire (never shown/stored) and mark the step so
    // we parse its result on `end`. Session agents get each skill once across
    // the whole conversation (the session context retains it); sessionless
    // agents re-see past injections in the transcript. Unknown skill ids are
    // skipped.
    const step = WF[stepIndexRef.current];
    const stepInject = !!step && stepArmedRef.current;
    stepArmedRef.current = false;
    inflightStepRef.current = stepInject ? stepIndexRef.current : null;
    if (stepInject) setStepStatusAt(stepIndexRef.current, "active");

    const skillBodies: string[] = [];
    const injectedNow: string[] = [];
    if (stepInject) {
      for (const id of step!.skillIds) {
        const s = skillMapRef.current[id];
        if (!s || !s.body.trim()) continue;
        if (sessionCapable) {
          if (injectedSkillIdsRef.current.has(s.id)) continue;
          injectedSkillIdsRef.current.add(s.id);
          injectedNow.push(s.id);
        }
        let body = s.body;
        // A skill with a resource folder (D45): tell the agent where it is and
        // grant read access (the dir rides extraDirs on every later turn too).
        if (s.dir?.trim()) {
          const dir = s.dir.trim();
          skillDirsRef.current.add(dir);
          body += `\n\n[스킬 리소스 폴더] 이 스킬의 참고 파일·스크립트가 다음 폴더에 있습니다: ${dir}\n필요한 파일을 직접 읽어 지시에 활용하세요.`;
        }
        skillBodies.push(body);
      }
    }
    // Built-in code-analysis method (D86): on an armed codebase / source-analysis
    // turn, above the step skills — outside the registry so it applies even to
    // saved overrides. Session agents get each mode once (synthetic id rides
    // the same dedupe + unwind as registry skills); sessionless re-see it.
    const codeMode = stepInject ? codeAnalysisMode(step) : null;
    if (codeMode) {
      const id = codeMethodSkillId(codeMode);
      if (!sessionCapable || !injectedSkillIdsRef.current.has(id)) {
        if (sessionCapable) {
          injectedSkillIdsRef.current.add(id);
          injectedNow.push(id);
        }
        skillBodies.unshift(codeMethodBody(codeMode, agentId));
      }
    }
    // Built-in prompt-optimizer skill (D78): inject once on the first real work
    // turn, independent of step arming — so guide's skipped rag step doesn't
    // consume it; it rides the first turn that actually reaches the agent.
    // unshift → wire top (above the step skills).
    let promptInjectedNow = false;
    if (promptSkillPendingRef.current) {
      promptSkillPendingRef.current = false;
      promptInjectedNow = true;
      promptInflightRef.current = true;
      skillBodies.unshift(PROMPT_OPTIMIZER_SKILL);
    }

    // A failed send never reached the agent — forget this turn's skill marks so
    // the retry (rearmStep) injects them again.
    const unwindSkills = () => {
      for (const id of injectedNow) injectedSkillIdsRef.current.delete(id);
      if (promptInjectedNow) {
        promptSkillPendingRef.current = true;
        promptInflightRef.current = false;
      }
    };

    // Foundation preflight (D44): resolve the step's wire context before the
    // agent turn, or skip the step without one (note + advance + chain on).
    let preflightContext = "";
    if (stepInject && step) {
      const stepIdx = stepIndexRef.current;
      const mapsCodebase =
        !!codebasePathRef.current &&
        (step.kind === "codebase" || (isRemote && codeAnalysisMode(step) === "focus"));
      const fetches = step.kind === "rag" || step.kind === "knowledge" || mapsCodebase;
      const fetchNote =
        step.kind === "rag"
          ? "사내 문서 검색·관련성 확인 중…"
          : mapsCodebase
            ? "코드베이스 구조 사전 분석 중…"
            : "지식 베이스 확인 중…";
      // Any awaited I/O runs inside the busy bracket (streaming=true): Stop
      // stays live and a concurrent manual send/retry can't spawn a second
      // run. Only the network fetches show a chat note (D57); a local hand-off
      // read (D86) is quick.
      const busy = fetches || handoffNeedsRead(stepIdx);
      if (busy) setStreaming(true);
      if (fetches) {
        // Visible fetch feedback (D57): the network call can take a while
        // (HTTP timeout 120s) — announce it, then drop the transient note.
        mutateMessages((prev) => [...prev, note(fetchNote)]);
      }
      const pf = await stepPreflight(step, cwd, stepIdx);
      if (busy) setStreaming(false);
      if (fetches) {
        mutateMessages((prev) =>
          prev.length && prev[prev.length - 1].system && prev[prev.length - 1].content === fetchNote
            ? prev.slice(0, -1)
            : prev,
        );
      }
      if (preflightAbortRef.current) {
        // Stopped mid-preflight → same fallback as a canceled step: halt
        // auto-progress, drop to plain chat.
        preflightAbortRef.current = false;
        unwindSkills();
        setStepStatusAt(stepIndexRef.current, "halted");
        inflightStepRef.current = null;
        stepIndexRef.current = WF.length;
        stepArmedRef.current = false;
        mutateMessages((prev) => [
          ...prev,
          note("작업을 중지했습니다 — 이후에는 일반 대화로 진행됩니다."),
        ]);
        return true; // deliberately stopped — consumed
      }
      if (pf.skip) {
        unwindSkills();
        inflightStepRef.current = null;
        setStepStatusAt(stepIndexRef.current, "skipped");
        mutateMessages((prev) => [...prev, note(pf.skip!)]);
        const nextIdx = stepIndexRef.current + 1;
        stepIndexRef.current = nextIdx;
        const nextStep = WF[nextIdx];
        if (nextStep && isGenerative(nextStep.kind)) {
          // Chain into the next generative step (re-enter send so its skills /
          // preflight run). A real user prompt (e.g. the answers turn) is
          // carried along; an auto turn shows the next progress note.
          stepArmedRef.current = true;
          return send(text, prompt ? opts : { ...opts, system: true, display: progressLabel(nextIdx, WF) });
        }
        stepArmedRef.current = false;
        if (nextStep) setStepStatusAt(nextIdx, "active"); // terminal chat reached
        maybeProposeKnowledgeSave();
        if (prompt) return send(text, opts); // never drop a real user prompt
        return true;
      }
      preflightContext = pf.context ?? "";
      if (pf.note) mutateMessages((prev) => [...prev, note(pf.note!)]);
    }

    // Absolute-path context for generative turns (D52): output location + the
    // codebase search root. `cwd` is already resolved above, first turn included.
    const pathCtx = stepInject && step && isGenerative(step.kind) ? pathContext(step, cwd) : "";

    // Remote agents (FabriX) have read-only file tools but cannot write — the
    // app writes their final reply to `step.file` (D67/D84). Tell them to read
    // what they need with the tools, then emit only the clean document body
    // (no tool talk / greetings) so the saved file is the document itself.
    // Only for remote document steps; CLI wire unchanged.
    // Tool calls are billed per round against a per-minute quota (D91): ask for
    // batched lookups, and — once earlier steps produced documents, whose
    // bodies the sessionless transcript already carries — not to re-read them.
    const priorDocs = [...producedDocsRef.current.keys()].some((i) => i < stepIndexRef.current);
    const remoteDocCtx =
      isRemote && stepInject && step && isGenerative(step.kind) && step.file
        ? "이 단계의 최종 답변 전체가 그대로 문서 파일로 저장됩니다(저장은 앱이 처리). 필요하면 먼저 읽기 전용 파일 도구(list_dir·find_files·search_text·outline_file·find_references·read_file)로 근거 파일을 읽은 뒤, 최종 답변에는 파일 저장 도구 사용·저장 여부 언급·인사말·메타설명 없이 문서 본문(마크다운/HTML)만 출력하세요. 도구 호출은 분당 한도가 있으니 필요한 조회는 한 응답에 묶어(files·paths·queries·symbols 배열) 요청하세요." +
          (priorDocs
            ? " 앞 단계 산출 문서의 본문은 위 대화에 이미 포함되어 있으므로 read_file로 다시 읽지 말고(앱이 만든 저장소 지도는 필요할 때만 참고), 필요한 소스코드만 확인하세요."
            : "")
        : "";

    const wire = [...skillBodies, stepInject ? step!.instruction : "", pathCtx, remoteDocCtx, preflightContext, prompt]
      .filter(Boolean)
      .join("\n\n");
    if (!wire.trim()) {
      inflightStepRef.current = null;
      return true; // nothing to send (empty input, no injected instruction)
    }
    if (!opts?.system) setInput("");
    // Remember the turn for same-session retry after a failure (D57).
    lastTurnRef.current = { text, opts, stepIndex: inflightStepRef.current };
    setUiError(null);
    pinnedRef.current = true; // a new turn always scrolls into view

    // Sessionless agents need the whole conversation each turn; capture it from
    // the pre-append messages before we add the new pair.
    const transcript = buildTranscript(messagesRef.current, wire);

    // The chat shows the display text (clean prompt / compact summary / step
    // note), not the instruction-wrapped or answer-wire text. Auto-advance turns
    // render as a subtle centered line (system) instead of a user bubble.
    const user: ChatMessage = {
      role: "user",
      content: opts?.display ?? prompt,
      thinking: "",
      events: [],
      system: opts?.system,
    };
    const assistant: ChatMessage = {
      role: "assistant",
      content: "",
      thinking: "",
      events: [],
      streaming: true,
    };
    const nextMessages = mutateMessages((prev) => [...prev, user, assistant]);

    // Persist the codebase path onto the manifest once known (a new session in
    // an existing project, or a later folder answer — D45).
    if (codebasePathRef.current && !codebasePersistedRef.current) {
      codebasePersistedRef.current = true;
      void setProjectCodebase(projectId, codebasePathRef.current).catch(() => {
        codebasePersistedRef.current = false; // retry on the next turn
      });
    }

    // Create the session folder as soon as the first question is asked.
    persist(nextMessages);

    remoteTailRef.current = "";
    setStreaming(true);
    const channel = new Channel<RunEvent>();
    channel.onmessage = handleEvent;
    try {
      const runId = await runAgent(
        {
          agentId,
          prompt: sessionCapable ? wire : transcript,
          cwd,
          model: model === "default" ? null : model,
          sessionId: sessionCapable ? sessionIdRef.current : null,
          resume: sessionCapable ? resumeRef.current : false,
          // Every turn: claude maps these to --add-dir, gemini to
          // --include-directories (codebase + skill resource folders, D52; +
          // the knowledge artifacts root once its index was injected, D59);
          // FabriX uses them (+ cwd) as its read-only tool sandbox (D84);
          // codex reads broadly, plain agents rely on the prompt mentions.
          extraDirs: [
            ...(codebasePathRef.current ? [codebasePathRef.current] : []),
            ...skillDirsRef.current,
            ...knowledgeDirsRef.current,
          ],
        },
        channel,
      );
      runIdRef.current = runId;
      // From the next turn on, continue the same session.
      if (sessionCapable) resumeRef.current = true;
    } catch (e) {
      rearmStep();
      unwindSkills();
      updateLastAssistant((m) => ({ ...m, streaming: false, error: String(e) }));
      setStreaming(false);
      persist();
    }
    return true; // turn handled (spawned, or its failure surfaced in the chat)
  };

  const stop = () => {
    // Cancel an in-flight relevance judge turn (D70) — its isolated run id isn't
    // in runIdRef, so it won't be reached by the branch below.
    ragJudgeCancelRef.current?.();
    if (runIdRef.current) cancelRun(runIdRef.current).catch(() => {});
    // Streaming with no run id = a foundation preflight fetch is in flight;
    // flag it so send() aborts instead of spawning the turn.
    else preflightAbortRef.current = true;
  };

  /** Re-send the last failed turn in the SAME session (D57): drop the failed
   * message pair (and any trailing halt notes), restore the workflow cursor to
   * the failed step, and fire the identical turn again. */
  const retry = () => {
    if (streaming) return;
    const turn = lastTurnRef.current;
    if (!turn) return;
    mutateMessages((prev) => {
      const next = [...prev];
      while (next.length && next[next.length - 1].system) next.pop(); // halt notes
      if (next.length && next[next.length - 1].role === "assistant") next.pop();
      if (next.length && next[next.length - 1].role === "user") next.pop();
      return next;
    });
    if (turn.stepIndex != null) {
      stepIndexRef.current = turn.stepIndex;
      stepArmedRef.current = true;
      setStepStatusAt(turn.stepIndex, "pending");
    }
    void send(turn.text, turn.opts);
  };

  /** Confirm before an action that would silently kill an in-flight run
   * (leave to Home / new session / open another session — D57). */
  const confirmLeave = async (): Promise<boolean> => {
    if (!streaming) return true;
    return ask("진행 중인 에이전트 작업이 있습니다. 나가면 실행이 중지됩니다.\n계속할까요?", {
      title: "Operation Wizard",
      kind: "warning",
    });
  };

  const guardedHome = () => {
    void confirmLeave().then((ok) => {
      if (ok) onHome();
    });
  };

  const guardedNewSession = () => {
    void confirmLeave().then((ok) => {
      if (ok) onNewSession();
    });
  };

  const openHistory = async () => {
    if (historyOpen) {
      setHistoryOpen(false);
      return;
    }
    if (projectId) {
      try {
        setHistory(await listSessions(projectId));
      } catch {
        setHistory([]);
      }
    } else {
      setHistory([]);
    }
    setHistoryOpen(true);
  };

  const chooseSession = async (id: string) => {
    setHistoryOpen(false);
    if (!projectId) return;
    if (!(await confirmLeave())) return;
    try {
      onOpenSession(await loadSession(projectId, id));
    } catch (e) {
      // Surface instead of silently staying on the current chat (D57).
      setUiError(`세션을 열지 못했습니다 — ${String(e)}`);
    }
  };

  // Boot once (fresh chat): show the category's fixed option form immediately
  // (no agent turn) — work always starts from it, led by the mandatory
  // requirement field (D95). A loaded session skips this → plain chat.
  useEffect(() => {
    if (bootedRef.current) return;
    bootedRef.current = true;
    if (optionQuestions.length > 0) onClarify(optionQuestions);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Guards the two nonce effects below against re-entry: send() is async and
  // toggles `streaming` (a dep) mid-flight, so an effect re-run could otherwise
  // double-fire the same turn before its nonce is consumed.
  const nonceSendInflightRef = useRef(false);

  // Send requirements-form answers as the first work turn (nonce-guarded, once
  // each); this turn is where the first workflow step gets injected. The
  // requirement answer is already in the wire (D78) and becomes the user bubble
  // + the session title (D97). The nonce is only consumed when send() actually
  // handled the turn — a submission racing an in-flight turn is retried via
  // `streaming` in the deps instead of being silently dropped (D55).
  useEffect(() => {
    if (!answerSubmission || answerSubmission.nonce === lastAnswerNonceRef.current) return;
    if (streaming || nonceSendInflightRef.current) return; // deps retry us later
    const submission = answerSubmission;
    lastAnswersWireRef.current = submission.wire; // feeds the RAG query
    const requirement = submission.requirement?.trim() ?? "";
    requirementRef.current = requirement;
    ragEnabledRef.current = submission.ragEnabled;
    if (!submission.ragEnabled) {
      // Show the switched-off RAG step as skipped in the stepper right away (D96).
      WF.forEach((s, i) => {
        if (s.kind === "rag") setStepStatusAt(i, "skipped");
      });
    }
    if (requirement && !sessionTitleRef.current) {
      applySessionTitle(heuristicTitle(requirement)); // instant; the summary refines it
      titlePendingRef.current = requirement;
    }
    nonceSendInflightRef.current = true;
    void send(submission.wire, { display: requirement || submission.display }).then((handled) => {
      nonceSendInflightRef.current = false;
      if (handled) lastAnswerNonceRef.current = submission.nonce;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [answerSubmission, streaming]);

  // Fire a queued auto-advance turn once (a generative step's continuation).
  // Nonce-guarded like answerSubmission so the stream handler never calls send()
  // from a stale closure, and consumed only when the turn was handled (D55).
  useEffect(() => {
    if (!autoTurn || autoTurn.nonce === lastAutoNonceRef.current) return;
    if (streaming || nonceSendInflightRef.current) return; // deps retry us later
    const turn = autoTurn;
    nonceSendInflightRef.current = true;
    void send("", { display: turn.display, system: true }).then((handled) => {
      nonceSendInflightRef.current = false;
      if (handled) lastAutoNonceRef.current = turn.nonce;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoTurn, streaming]);

  const availableAgents = useMemo(
    () => agents.filter((a) => detected[a.id]?.available),
    [agents, detected],
  );

  // The retry action targets the last assistant turn (a halt note may follow
  // it, so "last message" is not enough).
  const lastAssistantIdx = useMemo(() => {
    for (let i = messages.length - 1; i >= 0; i--) {
      if (messages[i].role === "assistant") return i;
    }
    return -1;
  }, [messages]);

  return (
    // Width comes from the WorkspaceView wrapper (user-resizable split, D49).
    <section className="relative flex w-full min-w-0 flex-col border-r border-line bg-panel min-h-0">
      {/* header */}
      <div className="flex shrink-0 items-center gap-2 border-b border-line px-3.5 py-2.5">
        <button
          type="button"
          onClick={guardedHome}
          title="홈"
          className="grid h-7 w-7 place-items-center rounded-lg border border-line bg-panel text-ink-muted transition-colors hover:bg-subtle"
        >
          <ChevronLeft size={15} />
        </button>
        <span className="grid h-[26px] w-[26px] place-items-center rounded-[7px] bg-accent-tint text-accent">
          <ClipboardList size={15} />
        </span>
        {/* session title (summarized requirement — D97), category as a prefix */}
        <span
          title={sessionTitle ? `${categoryLabel(category)} · ${sessionTitle}` : undefined}
          className="min-w-0 flex-1 truncate font-serif text-[14.5px] font-semibold text-ink-strong"
        >
          {sessionTitle ? (
            <>
              <span className="mr-1.5 font-sans text-[11.5px] font-medium text-ink-soft">
                {categoryLabel(category)}
              </span>
              {sessionTitle}
            </>
          ) : (
            categoryLabel(category)
          )}
        </span>
        <button
          type="button"
          onClick={openHistory}
          title="대화 기록"
          className="grid h-7 w-7 place-items-center rounded-lg border border-line bg-panel text-ink-muted transition-colors hover:bg-subtle"
        >
          <History size={15} />
        </button>
        <button
          type="button"
          onClick={guardedNewSession}
          title="새 세션"
          className="grid h-7 w-7 place-items-center rounded-lg border border-line bg-panel text-ink-muted transition-colors hover:bg-subtle"
        >
          <FilePlus2 size={15} />
        </button>
      </div>

      {/* persistent workflow progress (D57) */}
      {stepProgress && <WorkflowStepper steps={stepProgress} />}

      {/* transient UI failure line (e.g. session open failure — D57) */}
      {uiError && (
        <div className="flex shrink-0 items-center gap-2 border-b border-bad-border bg-bad-bg px-3.5 py-1.5 text-[12px] text-bad">
          <AlertTriangle size={13} className="shrink-0" />
          <span className="min-w-0 flex-1 truncate" title={uiError}>
            {uiError}
          </span>
          <button
            type="button"
            onClick={() => setUiError(null)}
            aria-label="닫기"
            className="grid shrink-0 place-items-center rounded p-0.5 transition-opacity hover:opacity-70"
          >
            <X size={13} />
          </button>
        </div>
      )}

      {/* history popover */}
      {historyOpen && (
        <>
          <button
            type="button"
            aria-label="닫기"
            onClick={() => setHistoryOpen(false)}
            className="absolute inset-0 z-10 cursor-default"
          />
          <div className="absolute right-3 top-[52px] z-20 max-h-[60%] w-[320px] overflow-auto rounded-xl border border-line-strong bg-elevated p-1.5 shadow-lg">
            <div className="px-2 py-1 text-[11px] font-bold uppercase tracking-[0.08em] text-ink-soft">
              이 프로젝트의 세션
            </div>
            {history.length === 0 ? (
              <div className="px-2 py-3 text-center text-[12px] text-ink-soft">
                저장된 세션이 없습니다.
              </div>
            ) : (
              history.map((s) => {
                const agentName = agents.find((a) => a.id === s.agentId)?.name ?? s.agentId;
                return (
                  <button
                    key={s.id}
                    type="button"
                    onClick={() => void chooseSession(s.id)}
                    className={
                      "block w-full rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-subtle " +
                      (s.id === persistIdRef.current ? "bg-subtle" : "")
                    }
                  >
                    <div className="truncate text-[12.5px] text-ink-strong">{s.title}</div>
                    <div className="mt-0.5 flex items-center gap-1.5 text-[11px] text-ink-soft">
                      <span>{agentName}</span>
                      <span>·</span>
                      <span>{s.messageCount}개 메시지</span>
                      <span>·</span>
                      <span>{sessionTime(s.updatedAt)}</span>
                    </div>
                  </button>
                );
              })
            )}
          </div>
        </>
      )}

      {/* messages */}
      <div className="relative flex min-h-0 flex-1 flex-col">
        <div
          ref={scrollRef}
          onScroll={handleScroll}
          className="flex flex-1 flex-col gap-4 overflow-auto px-4 py-[18px]"
        >
          {messages.length === 0 && (
            <div className="mt-6 text-center text-[12.5px] leading-[1.6] text-ink-soft">
              {formPending ? (
                <>
                  오른쪽 '요구사항' 탭에 하고 싶은 작업을 입력하고 제출하면 시작합니다.
                  <br />
                  아래에서 실행할 에이전트와 모델을 바꿀 수 있습니다.
                </>
              ) : (
                <>
                  메시지를 입력해 작업을 시작하세요.
                  <br />
                  에이전트가 선택한 작업 폴더에서 실행됩니다.
                </>
              )}
            </div>
          )}
          {messages.map((m, i) =>
            m.system ? (
              <div
                key={i}
                className="self-center whitespace-pre-wrap rounded-full bg-subtle px-3 py-1 text-center text-[11.5px] font-medium text-ink-soft"
              >
                {m.content}
              </div>
            ) : m.role === "user" ? (
              <div
                key={i}
                className="max-w-[88%] self-end whitespace-pre-wrap rounded-[14px_14px_4px_14px] bg-muted px-3.5 py-2.5 text-[13.5px] leading-[1.5] text-ink-strong"
              >
                {m.content}
              </div>
            ) : (
              <AssistantMessage
                key={i}
                message={m}
                speaker={agents.find((a) => a.id === agentId)?.name}
                onNewSession={onNewSession}
                onRetry={i === lastAssistantIdx && lastTurnRef.current ? retry : undefined}
              />
            ),
          )}
        </div>
        {showJump && (
          <button
            type="button"
            onClick={jumpToBottom}
            title="최신 메시지로 이동"
            className="absolute bottom-3 right-4 z-10 inline-flex items-center gap-1.5 rounded-full border border-line-strong bg-elevated px-2.5 py-1.5 text-[11.5px] font-medium text-ink-muted shadow-md transition-colors hover:bg-subtle"
          >
            <ArrowDown size={13} />
            최신으로
          </button>
        )}
      </div>

      {/* workflow-completion banner (D59): offer to save the artifacts as
          knowledge. Dismissible; never auto-switches the canvas (the end
          handler just routed it to the 산출물 tab — D58). */}
      {kbProposal && onOpenKnowledgeSave && (
        <div className="flex shrink-0 items-center gap-2 border-t border-line bg-accent-tint px-3.5 py-2 text-[12px] text-ink-muted">
          <BookmarkPlus size={14} className="shrink-0 text-accent" />
          <span className="min-w-0 flex-1">
            작업이 완료되었습니다 — 산출물을 지식으로 저장해 이후 작업에서 참고할 수 있습니다.
          </span>
          <button
            type="button"
            onClick={() => {
              setKbProposal(false);
              onOpenKnowledgeSave({ agentId, model: model === "default" ? null : model });
            }}
            className="shrink-0 rounded-md border border-accent px-2 py-1 font-medium text-accent transition-colors hover:bg-accent hover:text-white"
          >
            지식으로 저장
          </button>
          <button
            type="button"
            onClick={() => setKbProposal(false)}
            aria-label="닫기"
            className="grid shrink-0 place-items-center rounded p-0.5 transition-opacity hover:opacity-70"
          >
            <X size={13} />
          </button>
        </div>
      )}

      {/* composer */}
      <div className="shrink-0 border-t border-line px-3.5 py-3">
        {/* Undetected-agent onboarding (D57): sending would fail at runtime —
            warn up front and route to the Agents view. */}
        {detected[agentId] && !detected[agentId].available && (
          <div className="mb-2 flex items-center gap-1.5 rounded-lg border border-line bg-warn-bg px-2.5 py-1.5 text-[11.5px] text-warn">
            <AlertTriangle size={13} className="shrink-0" />
            <span className="min-w-0 flex-1">
              선택한 에이전트가 탐지되지 않았습니다 — 실행이 실패할 수 있습니다.
            </span>
            {onOpenAgents && (
              <button
                type="button"
                onClick={onOpenAgents}
                className="shrink-0 font-medium underline hover:opacity-80"
              >
                Agents에서 경로 설정
              </button>
            )}
          </div>
        )}
        <div className="mb-2 flex items-center gap-2 text-[11.5px] text-ink-soft">
          <select
            value={agentId}
            onChange={(e) => {
              agentPinnedRef.current = true; // manual pick — stop auto-defaulting
              setAgentId(e.target.value);
            }}
            disabled={started}
            title={started ? "대화 중에는 에이전트를 바꿀 수 없습니다 (새 세션에서 변경)" : undefined}
            className="rounded-md border border-line bg-panel px-1.5 py-1 text-ink-muted outline-none disabled:opacity-60"
          >
            {(availableAgents.length ? availableAgents : agents).map((a) => (
              <option key={a.id} value={a.id}>
                {a.name}
                {detected[a.id] ? (detected[a.id].available ? "" : " (미탐지)") : " (탐지 중…)"}
              </option>
            ))}
          </select>
          <select
            value={model}
            onChange={(e) => setModel(e.target.value)}
            className="min-w-0 flex-1 truncate rounded-md border border-line bg-panel px-1.5 py-1 text-ink-muted outline-none"
          >
            {models.length === 0 && <option value="default">Default</option>}
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label}
              </option>
            ))}
          </select>
        </div>
        <div className="flex items-end gap-2 rounded-[11px] border border-line-strong bg-panel px-2.5 py-2">
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              // Enter = 전송, Shift+Enter / Ctrl+Enter = 줄바꿈(기본 동작 허용)
              if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
                e.preventDefault();
                void send(input);
              }
            }}
            rows={1}
            disabled={formPending}
            placeholder={
              formPending
                ? "오른쪽 '요구사항' 항목을 먼저 작성해 제출해 주세요."
                : "메시지를 입력하세요… (Shift+Enter 줄바꿈)"
            }
            className="max-h-[160px] min-h-[24px] flex-1 resize-none overflow-y-auto bg-transparent text-[13px] leading-[1.5] text-ink outline-none placeholder:text-ink-faint disabled:opacity-60"
          />
          {streaming ? (
            <button
              type="button"
              onClick={stop}
              title="중지"
              className="grid h-[30px] w-[30px] shrink-0 place-items-center rounded-lg bg-bad text-white transition-colors hover:opacity-90"
            >
              <Square size={13} />
            </button>
          ) : (
            <button
              type="button"
              onClick={() => void send(input)}
              disabled={formPending}
              title={formPending ? "요구사항 제출 후 대화할 수 있습니다" : "전송"}
              className="grid h-[30px] w-[30px] shrink-0 place-items-center rounded-lg bg-accent text-white transition-colors hover:bg-accent-strong disabled:opacity-50 disabled:hover:bg-accent"
            >
              <ArrowUp size={15} />
            </button>
          )}
        </div>
      </div>
    </section>
  );
}

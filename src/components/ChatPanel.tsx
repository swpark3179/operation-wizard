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
  ensureProject,
  getKnowledgeRoot,
  listKnowledge,
  listSessions,
  loadSession,
  ragSearch,
  runAgent,
  saveSession,
  setProjectCodebase,
} from "../lib/api";
import { useAutoGrow } from "../lib/useAutoGrow";
import {
  prefillInstruction,
  parsePrefill,
  type ClarifyQuestion,
} from "../lib/clarify";
import { joinWorkdirPath } from "../lib/artifacts";
import { buildRagQuery, formatKnowledgeContext, formatRagContext, ragUserError } from "../lib/foundation";
import { optionsFor } from "../lib/options";
import { resolveSkills } from "../lib/skills";
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

export function ChatPanel({
  projectId,
  onResolveWorkdir,
  category,
  seedPrompt,
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
  onPrefill,
  onRagResult,
  onStreamingChange,
  onStepProgress,
  onOpenAgents,
  onOpenKnowledgeSave,
}: {
  /** The active project id (folder key for persistence). */
  projectId: string;
  /** Lift the resolved workdir up after the first send creates the project. */
  onResolveWorkdir: (workdir: string) => void;
  category: Category;
  seedPrompt: string;
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
  /** Requirements-form answers to send as the next turn (nonce-guarded). */
  answerSubmission: { wire: string; display: string; nonce: number } | null;
  /** True while the requirements form awaits the user — manual chat input is
   * blocked until the form is submitted (hidden prefill / auto turns still run). */
  formPending: boolean;
  onHome: () => void;
  /** Start a fresh session (clears the chat, re-enables agent selection). */
  onNewSession: () => void;
  /** Open a saved session (parent remounts this panel with it). */
  onOpenSession: (s: StoredSession) => void;
  onOpenFile: (path: string) => void;
  /** The category's fixed option questions → canvas form (shown on entry). */
  onClarify: (questions: ClarifyQuestion[]) => void;
  /** Option answers inferred from the launcher prompt (prefill pass) → form. */
  onPrefill: (answers: Record<string, string | string[]>) => void;
  /** RAG search results from the rag foundation step → canvas "검색 결과" tab. */
  onRagResult: (query: string, hits: RagHit[]) => void;
  /** Mirror streaming state up (canvas form disables while streaming). */
  onStreamingChange: (streaming: boolean) => void;
  /** Mirror the workflow step progress up (canvas 산출물 tab shows per-artifact
   * status, D58) — ownership stays here, same pattern as onStreamingChange. */
  onStepProgress?: (progress: StepProgress[] | null) => void;
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
  const [agentId, setAgentId] = useState<string>(
    () => initialSession?.agentId ?? defaultAgentId(agents, detected),
  );
  const [model, setModel] = useState<string>(() => initialSession?.model ?? "default");

  const runIdRef = useRef<string | null>(null);
  const sessionIdRef = useRef<string | null>(initialSession?.cliSessionId ?? null);
  const resumeRef = useRef(!!initialSession?.cliSessionId);
  const scrollRef = useRef<HTMLDivElement>(null);
  const seededRef = useRef(!!initialSession); // loaded sessions never auto-send a seed
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
  // Option-first entry (D36) + per-step skills (D40). The category's fixed
  // option catalog is shown on entry (not a workflow step); each step's skills
  // are injected on the turn that runs it (session agents get each skill once —
  // deduped via `injectedSkillIdsRef` — sessionless agents re-see them in the
  // transcript). A hidden "prefill" turn fills known options from the prompt.
  const optionQuestions = useRef(optionsFor(category, settings)).current;
  const skillMapRef = useRef(resolveSkills(settings));
  const injectedSkillIdsRef = useRef(new Set<string>());
  const seedPromptRef = useRef(seedPrompt); // original launcher prompt (folded into the first work turn)
  const prefillInflightRef = useRef(false); // this turn is the hidden prefill pass
  const bootedRef = useRef(false); // options/prefill/seed boot runs once
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
  // Workflow completion (D59): true once any generative step produced a file;
  // reaching the terminal chat step then proposes saving the artifacts as
  // knowledge — once per session, dismissible.
  const producedFileRef = useRef(false);
  const completionFiredRef = useRef(false);
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

  // The last real (non-prefill) turn, kept for same-session retry after a
  // failed turn (D57). `stepIndex` restores the workflow cursor on retry.
  const lastTurnRef = useRef<{
    text: string;
    opts?: { display?: string; system?: boolean };
    stepIndex: number | null;
  } | null>(null);

  // Transient UI-level failure line (e.g. a session that failed to open) —
  // shown under the header instead of being silently swallowed (D57).
  const [uiError, setUiError] = useState<string | null>(null);

  // Auto-scroll only while the user is pinned near the bottom; otherwise show
  // a jump-to-latest button instead of yanking the view down on every delta.
  const pinnedRef = useRef(true);
  const [showJump, setShowJump] = useState(false);

  const started = messages.length > 0;
  const sessionCapable = SESSION_AGENTS.includes(agentId);

  // History popover.
  const [historyOpen, setHistoryOpen] = useState(false);
  const [history, setHistory] = useState<SessionMeta[]>([]);

  useEffect(() => {
    codebasePathRef.current = codebasePath;
  }, [codebasePath]);

  useEffect(() => {
    onStreamingChange(streaming);
  }, [streaming, onStreamingChange]);

  useEffect(() => {
    onStepProgress?.(stepProgress);
  }, [stepProgress, onStepProgress]);

  // Cancel any in-flight run when this panel unmounts (new session, open a saved
  // session, or leave home) so we don't leak a background agent process.
  useEffect(() => {
    return () => {
      if (runIdRef.current) cancelRun(runIdRef.current).catch(() => {});
    };
  }, []);

  // Keep the default agent in sync as detection results arrive (until the
  // conversation starts — after that the agent is locked).
  useEffect(() => {
    if (!started) setAgentId(defaultAgentId(agents, detected));
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
    const title = (firstUser?.content ?? "").trim().slice(0, 60) || "새 대화";
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

  const handleEvent = (ev: RunEvent) => {
    switch (ev.type) {
      case "status":
        // claude echoes the minted id; codex/gemini surface their own here.
        // Ignore during a prefill turn — it runs isolated and must not hijack
        // the real conversation's session id.
        if (ev.sessionId && !prefillInflightRef.current) sessionIdRef.current = ev.sessionId;
        break;
      case "textDelta":
        updateLastAssistant((m) => ({ ...m, content: m.content + ev.delta }));
        break;
      case "stdout":
        updateLastAssistant((m) => ({ ...m, content: m.content + ev.chunk }));
        break;
      case "thinkingDelta":
        updateLastAssistant((m) => ({ ...m, thinking: m.thinking + ev.delta }));
        break;
      case "toolUse":
        updateLastAssistant((m) => ({
          ...m,
          events: [...m.events, { kind: "toolUse", id: ev.id, name: ev.name, input: ev.input }],
        }));
        break;
      case "toolResult":
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
      case "end": {
        setStreaming(false);
        runIdRef.current = null;

        // Prefill turn (hidden analysis): parse the option answers, lift them to
        // the form, and drop the turn's messages entirely — it is not part of the
        // conversation. Never touches the step cursor or persistence.
        if (prefillInflightRef.current) {
          prefillInflightRef.current = false;
          const msgs = messagesRef.current; // live via mutateMessages
          if (ev.status === "succeeded") {
            for (let i = msgs.length - 1; i >= 0; i--) {
              if (msgs[i].role === "assistant") {
                const answers = parsePrefill(msgs[i].content, optionQuestions);
                if (Object.keys(answers).length) onPrefill(answers);
                break;
              }
            }
          }
          // Drop the hidden prefill pair (the only content on a fresh chat).
          mutateMessages(() => []);
          break;
        }

        const stepIdx = inflightStepRef.current;
        inflightStepRef.current = null;

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
              onOpenFile(joinWorkdirPath(resolvedWorkdirRef.current, step.file));
            }
            const nextIdx = stepIdx + 1;
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
          }
          // chat kind: terminal — nothing to advance.
        } else if (stepIdx != null) {
          // Canceled/failed mid-workflow → halt auto-progress, drop to plain chat.
          stepIndexRef.current = WF.length;
          stepArmedRef.current = false;
          setStepStatusAt(stepIdx, "halted");
          mutateMessages((prev) => [
            ...prev,
            note(
              ev.status === "canceled"
                ? "작업을 중지했습니다 — 이후에는 일반 대화로 진행됩니다."
                : "오류로 단계 진행을 중단했습니다 — '다시 시도'로 재개하거나 일반 대화로 진행하세요.",
            ),
          ]);
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

  /** Foundation-step preflight (D44): resolve the extra wire context for this
   * turn, or decide to skip the step (unconfigured / empty / failed — the
   * foundation never blocks the flow). `codebase` is synchronous; `rag` and
   * `knowledge` fetch before the agent turn. */
  const stepPreflight = async (step: StepDef): Promise<{ context?: string; skip?: string }> => {
    if (step.kind === "rag") {
      if (!settings?.rag?.endpoint?.trim()) {
        return { skip: "RAG가 설정되지 않아 사내 문서 검색 단계를 건너뜁니다. (지식 화면에서 등록)" };
      }
      try {
        const query = buildRagQuery(seedPromptRef.current, lastAnswersWireRef.current);
        const hits = await ragSearch(query);
        if (preflightAbortRef.current) return {};
        if (!hits.length) return { skip: "관련 사내 문서를 찾지 못해 검색 단계를 건너뜁니다." };
        onRagResult(query, hits);
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
    return {};
  };

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
   * nonce on `true`; a submission racing the in-flight prefill is therefore
   * retried when streaming ends instead of being silently lost (D55). */
  const send = async (
    text: string,
    opts?: { display?: string; system?: boolean; prefill?: boolean },
  ): Promise<boolean> => {
    const prompt = text.trim();
    if (streaming) return false;
    // While the requirements form awaits the user, block manual sends — only
    // the hidden prefill pass and auto-advance turns (system) may run.
    if (formPending && !opts?.system && !opts?.prefill) return false;
    // Block an empty manual send before consuming the step arm; auto-advance /
    // prefill turns (system) legitimately carry no user text (the wire is built
    // from the injected instruction).
    if (!prompt && !opts?.system) return false;

    const isPrefill = !!opts?.prefill;
    prefillInflightRef.current = isPrefill;

    // Resolve the working folder up front (first send of a fresh chat creates
    // the project folder + manifest). Doing this before the preflight/wire
    // assembly lets the path context and preflight see the absolute workdir on
    // the very first turn (D52).
    let cwd = resolvedWorkdirRef.current;
    if (!ensuredRef.current) {
      try {
        // Title the project by the user's actual request (known even during a
        // prefill turn), not the answer/instruction wire.
        const title = seedPromptRef.current.trim().slice(0, 60) || categoryLabel(category);
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
        onResolveWorkdir(cwd);
      } catch (e) {
        prefillInflightRef.current = false;
        failTurn(opts?.display ?? prompt, opts?.system, String(e));
        return true; // attempted & surfaced — don't auto-retry
      }
    }
    if (!cwd) {
      prefillInflightRef.current = false;
      failTurn(opts?.display ?? prompt, opts?.system, "작업 폴더를 확인할 수 없습니다.");
      return true;
    }

    // This turn's workflow step + its skills (both skipped for a prefill turn,
    // which is isolated analysis). If armed, prepend the step's skill bodies +
    // instruction to the wire (never shown/stored) and mark the step so we
    // parse its result on `end`. Session agents get each skill once across the
    // whole conversation (the session context retains it); sessionless agents
    // re-see past injections in the transcript. Unknown skill ids are skipped.
    const step = isPrefill ? undefined : WF[stepIndexRef.current];
    const stepInject = !!step && stepArmedRef.current;
    if (!isPrefill) stepArmedRef.current = false;
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
    // A failed send never reached the agent — forget this turn's skill marks so
    // the retry (rearmStep) injects them again.
    const unwindSkills = () => {
      for (const id of injectedNow) injectedSkillIdsRef.current.delete(id);
    };

    // Foundation preflight (D44): resolve the step's wire context before the
    // agent turn, or skip the step without one (note + advance + chain on).
    let preflightContext = "";
    if (!isPrefill && stepInject && step) {
      const fetches = step.kind === "rag" || step.kind === "knowledge";
      const fetchNote = step.kind === "rag" ? "사내 문서 검색 중…" : "지식 베이스 확인 중…";
      if (fetches) {
        setStreaming(true); // Stop stays live during the fetch
        // Visible fetch feedback (D57): the network call can take a while
        // (HTTP timeout 120s) — announce it, then drop the transient note.
        mutateMessages((prev) => [...prev, note(fetchNote)]);
      }
      const pf = await stepPreflight(step);
      if (fetches) {
        setStreaming(false);
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
    }

    // Absolute-path context for generative turns (D52): output location + the
    // codebase search root. `cwd` is already resolved above, first turn included.
    const pathCtx =
      !isPrefill && stepInject && step && isGenerative(step.kind) ? pathContext(step, cwd) : "";

    const wire = isPrefill
      ? prefillInstruction(optionQuestions, seedPromptRef.current)
      : [...skillBodies, stepInject ? step!.instruction : "", pathCtx, preflightContext, prompt]
          .filter(Boolean)
          .join("\n\n");
    if (!wire.trim()) {
      inflightStepRef.current = null;
      prefillInflightRef.current = false;
      return true; // nothing to send (empty input, no injected instruction)
    }
    if (!opts?.system) setInput("");
    if (!isPrefill) {
      // Remember the turn for same-session retry after a failure (D57).
      lastTurnRef.current = { text, opts, stepIndex: inflightStepRef.current };
      setUiError(null);
    }
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

    // Persist the codebase path onto the manifest once known — the folder
    // answer usually arrives after `ensure_project` already ran (on the hidden
    // prefill turn), so this update path is the normal one (D45).
    if (!isPrefill && codebasePathRef.current && !codebasePersistedRef.current) {
      codebasePersistedRef.current = true;
      void setProjectCodebase(projectId, codebasePathRef.current).catch(() => {
        codebasePersistedRef.current = false; // retry on the next turn
      });
    }

    // Create the session folder as soon as the first question is asked (never
    // for the hidden prefill turn — it is not a saved conversation).
    if (!isPrefill) persist(nextMessages);

    setStreaming(true);
    const channel = new Channel<RunEvent>();
    channel.onmessage = handleEvent;
    // A prefill turn runs isolated (no session id / resume) so it never pollutes
    // the real conversation's session; only work turns continue the session.
    const useSession = sessionCapable && !isPrefill;
    try {
      const runId = await runAgent(
        {
          agentId,
          prompt: useSession || isPrefill ? wire : transcript,
          cwd,
          model: model === "default" ? null : model,
          sessionId: useSession ? sessionIdRef.current : null,
          resume: useSession ? resumeRef.current : false,
          // Every turn: claude maps these to --add-dir, gemini/aipro to
          // --include-directories (codebase + skill resource folders, D52; +
          // the knowledge artifacts root once its index was injected, D59);
          // codex is full-access, plain agents rely on the prompt mentions.
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
      if (useSession) resumeRef.current = true;
    } catch (e) {
      rearmStep();
      unwindSkills();
      prefillInflightRef.current = false;
      updateLastAssistant((m) => ({ ...m, streaming: false, error: String(e) }));
      setStreaming(false);
      if (!isPrefill) persist();
    }
    return true; // turn handled (spawned, or its failure surfaced in the chat)
  };

  const stop = () => {
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
  // (no agent turn); if the launcher carried a prompt, run a hidden prefill pass
  // to auto-fill the options it can infer. Categories without an option catalog
  // fall back to auto-sending the seed as the first work turn. A loaded session
  // (seededRef true) skips all of this → plain chat.
  useEffect(() => {
    if (bootedRef.current || seededRef.current) return;
    bootedRef.current = true;
    seededRef.current = true;
    if (optionQuestions.length > 0) {
      onClarify(optionQuestions);
      if (seedPromptRef.current.trim()) {
        void send("", {
          system: true,
          prefill: true,
          display: "요청 내용을 분석해 선택 항목을 자동으로 채우는 중…",
        });
      }
    } else if (seedPromptRef.current.trim()) {
      void send(seedPromptRef.current);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Guards the two nonce effects below against re-entry: send() is async and
  // toggles `streaming` (a dep) mid-flight, so an effect re-run could otherwise
  // double-fire the same turn before its nonce is consumed.
  const nonceSendInflightRef = useRef(false);

  // Send requirements-form answers as the first work turn (nonce-guarded, once
  // each). Folds the original launcher request into the wire as context; this
  // turn is where the category skill + first workflow step get injected.
  // The nonce is only consumed when send() actually handled the turn — if the
  // hidden prefill pass is still streaming, `streaming` in the deps retries the
  // submission when it ends instead of silently dropping it (D55).
  useEffect(() => {
    if (!answerSubmission || answerSubmission.nonce === lastAnswerNonceRef.current) return;
    if (streaming || nonceSendInflightRef.current) return; // deps retry us later
    const submission = answerSubmission;
    lastAnswersWireRef.current = submission.wire; // feeds the RAG query
    const seed = seedPromptRef.current.trim();
    const wire = seed ? `${submission.wire}\n\n원래 요청:\n${seed}` : submission.wire;
    nonceSendInflightRef.current = true;
    void send(wire, { display: seed || submission.display }).then((handled) => {
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
        <span className="min-w-0 flex-1 truncate font-serif text-[14.5px] font-semibold text-ink-strong">
          {categoryLabel(category)}
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
              메시지를 입력해 작업을 시작하세요.
              <br />
              에이전트가 선택한 작업 폴더에서 실행됩니다.
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
            onChange={(e) => setAgentId(e.target.value)}
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

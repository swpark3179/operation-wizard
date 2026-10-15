import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChatPanel, defaultAgentId } from "./ChatPanel";
import { CanvasPanel } from "./CanvasPanel";
import type { KnowledgeSaveRequest } from "./KnowledgeSavePanel";
import type { StepProgress } from "./WorkflowStepper";
import { categoryLabel, type Category, type ChatMessage } from "./workspace";
import { artifactsFor, joinWorkdirPath, normalizePathKey } from "../lib/artifacts";
import { formatClarifyAnswers, type ClarifyAnswer, type ClarifyQuestion } from "../lib/clarify";
import { ragCuratedHtml, ragResultHtml } from "../lib/foundation";
import type { RagVerdict } from "../lib/ragRelevance";
import { workflowFor } from "../lib/workflow";
import type { AgentInfo, DetectedAgent, RagHit, Settings, StoredSession } from "../lib/types";

/** Canvas views: the fixed tabs (산출물/다이어그램 aggregate the workflow's
 * document artifacts — D58; 지식 저장 turns them into a knowledge entry — D59)
 * plus one `file:<path>` viewer tab per open file
 * (the 파일 tab is the list; opening a file spawns its own closable tab — D49). */
export type CanvasTab =
  | "files"
  | "requirements"
  | "rag"
  | "prompt"
  | "artifacts"
  | "diagrams"
  | "knowledge-save"
  | `file:${string}`;

export function fileTabId(path: string): CanvasTab {
  return `file:${path}`;
}

/** The open file's path when `tab` is a file viewer tab, else null. */
export function fileTabPath(tab: CanvasTab): string | null {
  return tab.startsWith("file:") ? tab.slice("file:".length) : null;
}

// Chat panel width (D49): user-draggable, remembered across sessions as a pure
// UI preference (localStorage — not part of settings.json).
const CHAT_WIDTH_KEY = "ow.chatWidth";
const CHAT_MIN_WIDTH = 320;
const CHAT_MAX_WIDTH = 720;
/** Keep a canvas remainder at least this wide when clamping the drag. */
const CANVAS_MIN_WIDTH = 360;

function initialChatWidth(): number {
  // try/catch: localStorage access can throw (SecurityError) in a restricted
  // WebView2 — this runs in a useState initializer, so an uncaught throw
  // would take down the workspace render (D56).
  let stored = 0;
  try {
    stored = Number(localStorage.getItem(CHAT_WIDTH_KEY));
  } catch {
    // fall through to the default width
  }
  return Number.isFinite(stored) && stored > 0
    ? Math.min(CHAT_MAX_WIDTH, Math.max(CHAT_MIN_WIDTH, Math.round(stored)))
    : 412;
}

/** A saved session's request: its first real user message (the requirement —
 * the form submit's chat bubble, D78/D95). */
function sessionRequest(s: StoredSession | null): string {
  const msgs = (s?.messages as ChatMessage[] | undefined) ?? [];
  return msgs.find((m) => m.role === "user" && !m.system)?.content?.trim() ?? "";
}

/** The workspace: left conversation panel + right canvas panel. */
export function WorkspaceView({
  projectId,
  initialWorkdir,
  initialCodebasePath,
  initialAgentId,
  initialModel,
  category,
  agents,
  detected,
  settings,
  initialSession,
  onHome,
  onOpenAgents,
  onBusyChange,
}: {
  /** The active project's id (stable across the D27 remount). */
  projectId: string;
  /** The project's resolved workdir: the folder chosen on Home, the stored
   * workdir when opening a recent project, or null for a fresh auto project
   * (resolved lazily on the first send). */
  initialWorkdir: string | null;
  /** The project's stored codebase path when reopening (else null — chosen in
   * the requirements form's folder question, D45). */
  initialCodebasePath: string | null;
  /** Agent/model picked in Home's start settings (fresh chats — D60), or null. */
  initialAgentId?: string | null;
  initialModel?: string | null;
  category: Category;
  agents: AgentInfo[];
  detected: Record<string, DetectedAgent>;
  /** App settings (user workflows/skills), consumed by ChatPanel. */
  settings: Settings | null;
  /** A saved session to open on entry (from Home's recent list), or null. */
  initialSession: StoredSession | null;
  onHome: () => void;
  /** Navigate to the Agents view (undetected-agent onboarding, D57). */
  onOpenAgents?: () => void;
  /** Mirror the busy (streaming) state up so app-level navigation can confirm
   * before killing an in-flight run (D57). */
  onBusyChange?: (busy: boolean) => void;
}) {
  // Paths with an open file viewer tab (insertion order = tab order, D49).
  const [openFiles, setOpenFiles] = useState<string[]>([]);
  // Bumped to reload the canvas file tree (e.g. after a workflow document step
  // writes a new file).
  const [refreshNonce, setRefreshNonce] = useState(0);
  // Draggable chat/canvas split (D49). The width applies to the ChatPanel
  // wrapper; the canvas takes the remainder.
  const [chatWidth, setChatWidth] = useState(initialChatWidth);
  const [resizing, setResizing] = useState(false);
  const layoutRef = useRef<HTMLDivElement | null>(null);
  // The project's working folder (cwd + canvas root). Resolved lazily by
  // ChatPanel on the first send for a fresh chat; known up front when opening a
  // recent project. Survives the sessionNonce remount (same project).
  const [resolvedWorkdir, setResolvedWorkdir] = useState<string | null>(initialWorkdir);
  // The foundation phase's analyzed codebase folder — project-scoped like the
  // workdir (survives the sessionNonce remount). Set by the form's folder
  // answer; delivered to ChatPanel structurally (never as wire prose).
  const [codebasePath, setCodebasePath] = useState<string | null>(initialCodebasePath);
  // The latest RAG search result, rendered as a canvas tab (in-memory HTML via
  // sandboxed iframe srcdoc — never written to disk, D46).
  const [ragResult, setRagResult] = useState<{ query: string; html: string } | null>(null);
  // The optimized prompt the agent produced on the first work turn (D78),
  // rendered as the "프롬프트" canvas tab (plain text — no iframe needed).
  const [promptResult, setPromptResult] = useState<string | null>(null);
  // Remount key: bumping it starts a fresh ChatPanel (new session) or swaps in a
  // loaded session, resetting all of ChatPanel's state/refs in one shot.
  const [sessionNonce, setSessionNonce] = useState(0);
  const [loadedSession, setLoadedSession] = useState<StoredSession | null>(initialSession);
  const [activeCategory, setActiveCategory] = useState<Category>(
    (initialSession?.category as Category) ?? category,
  );
  // The session's title (summarized from the requirement — D97), mirrored up
  // from ChatPanel, and the requirement itself; both feed the 지식 저장 panel.
  const [sessionTitle, setSessionTitle] = useState<string | null>(initialSession?.title ?? null);
  const [request, setRequest] = useState(() => sessionRequest(initialSession));

  // Requirements options: the category's fixed catalog is shown on entry →
  // canvas form → answers back to the chat as the first work turn. Transient
  // (not persisted).
  const [clarify, setClarify] = useState<ClarifyQuestion[] | null>(null);
  const [canvasTab, setCanvasTab] = useState<CanvasTab>("files");
  const [streaming, setStreaming] = useState(false);
  // Workflow step progress mirrored up from ChatPanel (ownership stays there —
  // D57/D58); feeds the 산출물 tab's per-artifact status chips.
  const [stepProgress, setStepProgress] = useState<StepProgress[] | null>(null);
  // The 산출물 tab's selected artifact (stepId), or null → auto-pick.
  const [artifactSel, setArtifactSel] = useState<string | null>(null);
  // The 지식 저장 tab's request (D59), or null → tab hidden. The entry id is
  // minted once per session and survives a save, so saving twice in the same
  // session upserts one entry (staged-swap file replace) instead of
  // duplicating; new/opened sessions reset it.
  const [knowledgeSave, setKnowledgeSave] = useState<KnowledgeSaveRequest | null>(null);
  const kbEntryIdRef = useRef<string | null>(null);
  // The category workflow's document artifacts. Frozen per session like
  // ChatPanel's WF (settings intentionally omitted from the deps — both
  // recompute together on the sessionNonce remount).
  const artifacts = useMemo(
    () => artifactsFor(activeCategory, settings),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [activeCategory, sessionNonce],
  );
  // The form's RAG step switch (D96): only when the (frozen) workflow has a rag
  // step; off/unconfigured → ChatPanel skips that step.
  const ragConfigured = !!settings?.rag?.endpoint?.trim();
  const ragOption = useMemo(
    () =>
      workflowFor(activeCategory, settings).some((s) => s.kind === "rag")
        ? { configured: ragConfigured }
        : null,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [activeCategory, sessionNonce, ragConfigured],
  );

  // Mirror the streaming state up (D57): app-level navigation asks before
  // unmounting a busy workspace; the cleanup clears the flag on leave.
  const handleStreamingChange = useCallback(
    (s: boolean) => {
      setStreaming(s);
      onBusyChange?.(s);
    },
    [onBusyChange],
  );
  useEffect(() => () => onBusyChange?.(false), [onBusyChange]);

  const [answerSubmission, setAnswerSubmission] = useState<{
    wire: string;
    display: string;
    /** The raw requirement-field answer (D78) — the user bubble and the source
     * of the session title (D97). */
    requirement: string;
    /** False when the form's RAG switch is off → skip the rag step (D96). */
    ragEnabled: boolean;
    nonce: number;
  } | null>(null);

  const resetClarify = () => {
    setClarify(null);
    setCanvasTab("files");
    setAnswerSubmission(null);
  };

  const handleNewSession = () => {
    setLoadedSession(null);
    setSessionTitle(null);
    setRequest("");
    resetClarify();
    setRagResult(null); // codebasePath stays — it is project-scoped
    setPromptResult(null);
    setOpenFiles([]);
    setArtifactSel(null);
    setKnowledgeSave(null); // next save is a new entry (D59)
    kbEntryIdRef.current = null;
    setStepProgress(null); // the remounted ChatPanel re-mirrors its fresh value
    setSessionNonce((n) => n + 1);
  };

  const handleOpenSession = (s: StoredSession) => {
    setLoadedSession(s);
    setActiveCategory((s.category as Category) ?? activeCategory);
    setSessionTitle(s.title || null);
    setRequest(sessionRequest(s));
    resetClarify();
    setRagResult(null);
    setPromptResult(null);
    setOpenFiles([]);
    setArtifactSel(null);
    setKnowledgeSave(null);
    kbEntryIdRef.current = null;
    setStepProgress(null);
    setSessionNonce((n) => n + 1);
  };

  // RAG search results from the rag foundation step: build the (escaped)
  // result document and surface it as the "검색 결과" tab. When the relevance
  // judge returned curated sections (D70) render those; otherwise (fail open)
  // fall back to the raw hit list.
  const handleRagResult = (query: string, hits: RagHit[], verdict: RagVerdict | null) => {
    const html = verdict?.sections?.length
      ? ragCuratedHtml(query, verdict, hits)
      : ragResultHtml(query, hits);
    setRagResult({ query, html });
    setCanvasTab("rag");
  };

  // The optimized prompt from the first work turn (D78) → "프롬프트" tab. Auto-
  // switch on arrival (D46 precedent) — surfacing it is the educational point.
  const handlePromptResult = (text: string) => {
    setPromptResult(text);
    setCanvasTab("prompt");
  };

  const handleClarify = (questions: ClarifyQuestion[]) => {
    setClarify(questions);
    setCanvasTab("requirements"); // auto-switch canvas to the form
  };

  // Open a file in its own viewer tab (D49). Tree clicks just open the tab;
  // workflow-produced files also bump the nonce so the tree shows the new file
  // and an already-open tab re-reads the rewritten content.
  const openFile = (path: string, opts?: { refresh?: boolean }) => {
    setOpenFiles((f) => (f.includes(path) ? f : [...f, path]));
    setCanvasTab(fileTabId(path));
    if (opts?.refresh) setRefreshNonce((n) => n + 1);
  };

  // Workflow-produced files route to the 산출물 hub instead of spawning a file
  // tab (D58, amends D49) — the hub is where the "구획별 여러 계획" documents
  // live. Non-artifact files (and all tree clicks) keep their D49 file tabs.
  const handleOpenFile = (path: string) => {
    const hit = resolvedWorkdir
      ? artifacts.find(
          (a) => normalizePathKey(joinWorkdirPath(resolvedWorkdir, a.file)) === normalizePathKey(path),
        )
      : undefined;
    if (hit) {
      setArtifactSel(hit.stepId);
      setCanvasTab("artifacts");
      setRefreshNonce((n) => n + 1); // tree reload + viewer re-read + probe/scan re-run
    } else {
      openFile(path, { refresh: true });
    }
  };

  const handleCloseFile = (path: string) => {
    setOpenFiles((f) => f.filter((p) => p !== path));
    setCanvasTab((t) => (t === fileTabId(path) ? "files" : t));
  };

  // ── 지식 저장 (D59) ─────────────────────────────────────────────────────────
  // Shared opener for both entry points: the ChatPanel completion banner
  // (agent/model of the running session, all artifacts pre-checked) and the
  // 산출물-tab row action (that artifact pre-checked; loaded sessions pick a
  // fallback agent for the summary turn).
  const openKnowledgeSave = (ctx: {
    agentId: string;
    model: string | null;
    preselect: string[] | null;
  }) => {
    if (!kbEntryIdRef.current) kbEntryIdRef.current = crypto.randomUUID();
    const baseTitle = sessionTitle?.trim() || request || categoryLabel(activeCategory);
    setKnowledgeSave({
      entryId: kbEntryIdRef.current,
      agentId: ctx.agentId,
      model: ctx.model,
      preselect: ctx.preselect,
      defaultTitle: `${baseTitle.slice(0, 40)} — 작업 정리`,
      request: request || baseTitle,
      source: {
        projectId,
        category: activeCategory,
        title: baseTitle.slice(0, 60),
      },
    });
    setCanvasTab("knowledge-save");
  };

  const handleOpenKnowledgeSave = (ctx: { agentId: string; model: string | null }) =>
    openKnowledgeSave({ ...ctx, preselect: null });

  const handleSaveArtifact = (stepId: string) => {
    const model = loadedSession?.model;
    openKnowledgeSave({
      agentId: loadedSession?.agentId ?? defaultAgentId(agents, detected),
      model: model && model !== "default" ? model : null,
      preselect: [stepId],
    });
  };

  const closeKnowledgeSave = () => {
    setKnowledgeSave(null);
    setCanvasTab((t) => (t === "knowledge-save" ? "artifacts" : t));
  };

  // Drag-to-resize the chat/canvas split (D49). Pointer capture keeps the
  // events on the handle even over the canvas iframes; the width is clamped so
  // both panels stay usable and persisted on release.
  const handleResizeStart = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    setResizing(true);
  };

  const handleResizeMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!resizing) return;
    const rect = layoutRef.current?.getBoundingClientRect();
    if (!rect) return;
    const max = Math.min(CHAT_MAX_WIDTH, rect.width - CANVAS_MIN_WIDTH);
    setChatWidth(Math.min(max, Math.max(CHAT_MIN_WIDTH, Math.round(e.clientX - rect.left))));
  };

  const handleResizeEnd = () => {
    if (!resizing) return;
    setResizing(false);
    setChatWidth((w) => {
      try {
        localStorage.setItem(CHAT_WIDTH_KEY, String(w));
      } catch {
        // storage unavailable — the width still applies in-memory
      }
      return w;
    });
  };

  const handleSubmitAnswers = (answers: ClarifyAnswer[], opts: { ragEnabled: boolean }) => {
    // The folder answer (codebasePath) is delivered structurally — lifted into
    // state (→ ChatPanel prop → extraDirs + preflight context) and excluded
    // from the answers wire (D45).
    const folder = answers.find((a) => a.type === "folder" && a.id === "codebasePath");
    if (typeof folder?.value === "string" && folder.value.trim()) {
      setCodebasePath(folder.value.trim());
    }
    const rest = answers.filter((a) => a.type !== "folder");
    const { wire, display } = formatClarifyAnswers(rest);
    // The requirement answer stays IN the wire (it is the prompt optimizer's key
    // input, D78) but is also carried out so ChatPanel can bubble/dedupe it.
    const req = answers.find((a) => a.id === "userRequest");
    const requirement = typeof req?.value === "string" ? req.value.trim() : "";
    setRequest(requirement);
    setAnswerSubmission((s) => ({
      wire,
      display,
      requirement,
      ragEnabled: opts.ragEnabled,
      nonce: (s?.nonce ?? 0) + 1,
    }));
    setClarify(null);
    setCanvasTab("files"); // back to the file explorer while the agent works
  };

  return (
    <div ref={layoutRef} className={"flex h-full min-h-0" + (resizing ? " select-none" : "")}>
      <div style={{ width: chatWidth }} className="flex min-h-0 shrink-0">
        <ChatPanel
          key={sessionNonce}
          projectId={projectId}
          onResolveWorkdir={setResolvedWorkdir}
          category={activeCategory}
          initialAgentId={initialAgentId}
          initialModel={initialModel}
          agents={agents}
          detected={detected}
          settings={settings}
          workdir={resolvedWorkdir}
          codebasePath={codebasePath}
          initialSession={loadedSession}
          answerSubmission={answerSubmission}
          formPending={!!clarify?.length}
          onHome={onHome}
          onNewSession={handleNewSession}
          onOpenSession={handleOpenSession}
          onOpenFile={handleOpenFile}
          onClarify={handleClarify}
          onRagResult={handleRagResult}
          onPromptResult={handlePromptResult}
          onStreamingChange={handleStreamingChange}
          onStepProgress={setStepProgress}
          onSessionTitle={setSessionTitle}
          onOpenAgents={onOpenAgents}
          onOpenKnowledgeSave={handleOpenKnowledgeSave}
        />
      </div>
      <div
        role="separator"
        aria-orientation="vertical"
        title="드래그하여 크기 조정"
        onPointerDown={handleResizeStart}
        onPointerMove={handleResizeMove}
        onPointerUp={handleResizeEnd}
        onPointerCancel={handleResizeEnd}
        className={
          // Zero net width (negative margins) so the grab strip sits on the
          // panel seam instead of adding a gap.
          "relative z-10 -mx-0.5 w-1 shrink-0 cursor-col-resize transition-colors " +
          (resizing ? "bg-accent" : "bg-transparent hover:bg-accent")
        }
      />
      <CanvasPanel
        workdir={resolvedWorkdir}
        codebasePath={codebasePath}
        refreshNonce={refreshNonce}
        openFiles={openFiles}
        onOpenFile={openFile}
        onCloseFile={handleCloseFile}
        tab={canvasTab}
        onTabChange={setCanvasTab}
        clarify={clarify}
        ragOption={ragOption}
        onSubmitAnswers={handleSubmitAnswers}
        ragResult={ragResult}
        promptResult={promptResult}
        artifacts={artifacts}
        stepProgress={stepProgress}
        artifactSel={artifactSel}
        onSelectArtifact={setArtifactSel}
        knowledgeSave={knowledgeSave}
        onCloseKnowledgeSave={closeKnowledgeSave}
        onKnowledgeSaved={closeKnowledgeSave}
        onSaveArtifact={handleSaveArtifact}
        streaming={streaming}
      />
      {/* While dragging, a full-window shield keeps the canvas iframes (HTML/
          rag previews) from swallowing pointer events. */}
      {resizing && <div className="fixed inset-0 z-50 cursor-col-resize" />}
    </div>
  );
}

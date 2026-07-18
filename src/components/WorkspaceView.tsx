import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { ChatPanel, defaultAgentId } from "./ChatPanel";
import { CanvasPanel } from "./CanvasPanel";
import type { KnowledgeSaveRequest } from "./KnowledgeSavePanel";
import type { StepProgress } from "./WorkflowStepper";
import { categoryLabel, type Category } from "./workspace";
import { ensureProject } from "../lib/api";
import { artifactsFor, joinWorkdirPath, normalizePathKey } from "../lib/artifacts";
import { classifyCategory } from "../lib/categorize";
import { formatClarifyAnswers, type ClarifyAnswer, type ClarifyQuestion } from "../lib/clarify";
import { ragCuratedHtml, ragResultHtml } from "../lib/foundation";
import type { RagVerdict } from "../lib/ragRelevance";
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

/** The workspace: left conversation panel + right canvas panel. */
export function WorkspaceView({
  projectId,
  initialWorkdir,
  initialCodebasePath,
  initialAgentId,
  initialModel,
  category,
  autoCategory,
  seedPrompt,
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
  /** Agent/model picked in the Home composer (fresh chats — D60), or null. */
  initialAgentId?: string | null;
  initialModel?: string | null;
  category: Category;
  /** True when `category` is only a provisional default and the real category
   * should be classified from `seedPrompt` before ChatPanel mounts (D81). */
  autoCategory?: boolean;
  seedPrompt: string;
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
  // Only the very first chat (from the launcher) auto-sends the seed prompt.
  const [activeSeed, setActiveSeed] = useState(seedPrompt);
  // Category auto-classification (D81): a composer start arrives with a
  // provisional category ("plan") + autoCategory=true. While classifying, the
  // chat column shows a placeholder; ChatPanel mounts once for the chosen
  // category when it clears. Loaded sessions / explicit category cards (and an
  // empty prompt) never classify — ChatPanel just mounts for `category`.
  const [classifying, setClassifying] = useState(
    () => !!autoCategory && !initialSession && !!seedPrompt.trim(),
  );
  const classifyStartedRef = useRef(false);
  const classifyCancelRef = useRef<(() => void) | null>(null);

  // Requirements options: the category's fixed catalog is shown on entry →
  // canvas form → answers back to the chat as the first work turn. Optionally
  // pre-filled from the launcher prompt (prefill pass). Transient (not persisted).
  const [clarify, setClarify] = useState<ClarifyQuestion[] | null>(null);
  const [clarifyPrefill, setClarifyPrefill] = useState<Record<string, string | string[]> | null>(
    null,
  );
  const [prefillNonce, setPrefillNonce] = useState(0);
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

  // Run the classification turn once (D81). It needs a cwd (run.rs requires a
  // non-empty working folder), so resolve/create the project first (idempotent
  // ensureProject — writes the provisional "plan" once; the real category is
  // persisted per-session by the mounted ChatPanel). Guarded by a ref like
  // ChatPanel's boot effect (no cleanup → StrictMode double-invoke is a no-op).
  // Any failure falls back to "plan", the historical default.
  useEffect(() => {
    if (!classifying || classifyStartedRef.current) return;
    classifyStartedRef.current = true;
    void (async () => {
      let best: Category = "plan";
      try {
        const title = seedPrompt.trim().slice(0, 60) || categoryLabel(category);
        const project = await ensureProject(projectId, initialWorkdir ?? "", title, "plan", null);
        setResolvedWorkdir(project.workdir);
        const judge = classifyCategory({
          agentId: initialAgentId ?? defaultAgentId(agents, detected),
          model: initialModel && initialModel !== "default" ? initialModel : null,
          cwd: project.workdir,
          seed: seedPrompt,
        });
        classifyCancelRef.current = judge.cancel;
        best = (await judge.promise) ?? "plan";
        classifyCancelRef.current = null;
      } catch {
        best = "plan";
      }
      setActiveCategory(best);
      setClassifying(false);
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const [answerSubmission, setAnswerSubmission] = useState<{
    wire: string;
    display: string;
    /** The raw requirement-field answer (D78) — lets ChatPanel show it as the
     * user bubble and skip re-appending the launcher seed. */
    requirement: string;
    nonce: number;
  } | null>(null);

  const resetClarify = () => {
    setClarify(null);
    setClarifyPrefill(null);
    setCanvasTab("files");
    setAnswerSubmission(null);
  };

  const handleNewSession = () => {
    setLoadedSession(null);
    setActiveSeed("");
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
    setActiveSeed("");
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

  // Prefill answers inferred from the launcher prompt → pre-fill the form (bump
  // the nonce so RequirementsForm re-initializes with them).
  const handlePrefill = (answers: Record<string, string | string[]>) => {
    setClarifyPrefill(answers);
    setPrefillNonce((n) => n + 1);
    setCanvasTab("requirements");
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
    const baseTitle =
      activeSeed.trim() || loadedSession?.title?.trim() || categoryLabel(activeCategory);
    setKnowledgeSave({
      entryId: kbEntryIdRef.current,
      agentId: ctx.agentId,
      model: ctx.model,
      preselect: ctx.preselect,
      defaultTitle: `${baseTitle.slice(0, 40)} — 작업 정리`,
      request: baseTitle,
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

  const handleSubmitAnswers = (answers: ClarifyAnswer[]) => {
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
    setAnswerSubmission((s) => ({ wire, display, requirement, nonce: (s?.nonce ?? 0) + 1 }));
    setClarify(null);
    setClarifyPrefill(null);
    setCanvasTab("files"); // back to the file explorer while the agent works
  };

  return (
    <div ref={layoutRef} className={"flex h-full min-h-0" + (resizing ? " select-none" : "")}>
      <div style={{ width: chatWidth }} className="flex min-h-0 shrink-0">
        {classifying ? (
          <div className="flex h-full w-full flex-col items-center justify-center gap-3 border-r border-line bg-panel px-6 text-center">
            <Loader2 size={22} className="animate-spin text-accent" />
            <div className="text-[13.5px] font-medium text-ink-strong">
              요청을 분석해 작업 유형을 선택하는 중…
            </div>
            <div className="text-[12px] leading-[1.5] text-ink-soft">
              입력하신 요청에 가장 알맞은 업무 카테고리를 정하고 있습니다.
            </div>
            <button
              type="button"
              onClick={() => classifyCancelRef.current?.()}
              className="mt-1 rounded-md border border-line bg-panel px-3 py-1.5 text-[12px] text-ink-muted transition-colors hover:bg-subtle"
            >
              중지
            </button>
          </div>
        ) : (
          <ChatPanel
          key={sessionNonce}
        projectId={projectId}
        onResolveWorkdir={setResolvedWorkdir}
        category={activeCategory}
        seedPrompt={activeSeed}
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
          onPrefill={handlePrefill}
          onRagResult={handleRagResult}
          onPromptResult={handlePromptResult}
          onStreamingChange={handleStreamingChange}
          onStepProgress={setStepProgress}
          onOpenAgents={onOpenAgents}
          onOpenKnowledgeSave={handleOpenKnowledgeSave}
          />
        )}
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
        clarifyPrefill={clarifyPrefill}
        prefillNonce={prefillNonce}
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

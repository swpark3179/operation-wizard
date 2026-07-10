import { useRef, useState } from "react";
import { ChatPanel } from "./ChatPanel";
import { CanvasPanel } from "./CanvasPanel";
import type { Category } from "./workspace";
import { formatClarifyAnswers, type ClarifyAnswer, type ClarifyQuestion } from "../lib/clarify";
import { ragResultHtml } from "../lib/foundation";
import type { AgentInfo, DetectedAgent, RagHit, Settings, StoredSession } from "../lib/types";

/** Canvas views: the fixed tabs plus one `file:<path>` viewer tab per open file
 * (the 파일 tab is the list; opening a file spawns its own closable tab — D49). */
export type CanvasTab = "files" | "requirements" | "rag" | `file:${string}`;

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
  category,
  seedPrompt,
  agents,
  detected,
  settings,
  initialSession,
  onHome,
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
  category: Category;
  seedPrompt: string;
  agents: AgentInfo[];
  detected: Record<string, DetectedAgent>;
  /** App settings (user workflows/skills), consumed by ChatPanel. */
  settings: Settings | null;
  /** A saved session to open on entry (from Home's recent list), or null. */
  initialSession: StoredSession | null;
  onHome: () => void;
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
  // Remount key: bumping it starts a fresh ChatPanel (new session) or swaps in a
  // loaded session, resetting all of ChatPanel's state/refs in one shot.
  const [sessionNonce, setSessionNonce] = useState(0);
  const [loadedSession, setLoadedSession] = useState<StoredSession | null>(initialSession);
  const [activeCategory, setActiveCategory] = useState<Category>(
    (initialSession?.category as Category) ?? category,
  );
  // Only the very first chat (from the launcher) auto-sends the seed prompt.
  const [activeSeed, setActiveSeed] = useState(seedPrompt);

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
  const [answerSubmission, setAnswerSubmission] = useState<{
    wire: string;
    display: string;
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
    setOpenFiles([]);
    setSessionNonce((n) => n + 1);
  };

  const handleOpenSession = (s: StoredSession) => {
    setLoadedSession(s);
    setActiveCategory((s.category as Category) ?? activeCategory);
    setActiveSeed("");
    resetClarify();
    setRagResult(null);
    setOpenFiles([]);
    setSessionNonce((n) => n + 1);
  };

  // RAG search results from the rag foundation step: build the (escaped)
  // result document and surface it as the "검색 결과" tab.
  const handleRagResult = (query: string, hits: RagHit[]) => {
    setRagResult({ query, html: ragResultHtml(query, hits) });
    setCanvasTab("rag");
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

  const handleOpenFile = (path: string) => openFile(path, { refresh: true });

  const handleCloseFile = (path: string) => {
    setOpenFiles((f) => f.filter((p) => p !== path));
    setCanvasTab((t) => (t === fileTabId(path) ? "files" : t));
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
    setAnswerSubmission((s) => ({ wire, display, nonce: (s?.nonce ?? 0) + 1 }));
    setClarify(null);
    setClarifyPrefill(null);
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
        seedPrompt={activeSeed}
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
          onStreamingChange={setStreaming}
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
        clarifyPrefill={clarifyPrefill}
        prefillNonce={prefillNonce}
        onSubmitAnswers={handleSubmitAnswers}
        ragResult={ragResult}
        streaming={streaming}
      />
      {/* While dragging, a full-window shield keeps the canvas iframes (HTML/
          rag previews) from swallowing pointer events. */}
      {resizing && <div className="fixed inset-0 z-50 cursor-col-resize" />}
    </div>
  );
}

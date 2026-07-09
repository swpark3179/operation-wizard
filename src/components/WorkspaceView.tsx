import { useState } from "react";
import { ChatPanel } from "./ChatPanel";
import { CanvasPanel } from "./CanvasPanel";
import type { Category } from "./workspace";
import { formatClarifyAnswers, type ClarifyAnswer, type ClarifyQuestion } from "../lib/clarify";
import { ragResultHtml } from "../lib/foundation";
import type { AgentInfo, DetectedAgent, RagHit, Settings, StoredSession } from "../lib/types";

export type CanvasTab = "files" | "requirements" | "rag";

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
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  // Bumped to reload the canvas file tree (e.g. after a workflow document step
  // writes a new file).
  const [refreshNonce, setRefreshNonce] = useState(0);
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
    setSessionNonce((n) => n + 1);
  };

  const handleOpenSession = (s: StoredSession) => {
    setLoadedSession(s);
    setActiveCategory((s.category as Category) ?? activeCategory);
    setActiveSeed("");
    resetClarify();
    setRagResult(null);
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

  // Open a produced file in the canvas (e.g. a workflow document step): switch
  // to the files tab, reload the tree so the new file appears, and select it.
  const handleOpenFile = (path: string) => {
    setSelectedFile(path);
    setCanvasTab("files");
    setRefreshNonce((n) => n + 1);
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
    <div className="flex h-full min-h-0">
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
      <CanvasPanel
        workdir={resolvedWorkdir}
        codebasePath={codebasePath}
        refreshNonce={refreshNonce}
        selectedFile={selectedFile}
        onSelectFile={setSelectedFile}
        tab={canvasTab}
        onTabChange={setCanvasTab}
        clarify={clarify}
        clarifyPrefill={clarifyPrefill}
        prefillNonce={prefillNonce}
        onSubmitAnswers={handleSubmitAnswers}
        ragResult={ragResult}
        streaming={streaming}
      />
    </div>
  );
}

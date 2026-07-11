import { useCallback, useEffect, useState } from "react";
import {
  Folder,
  FolderOpen,
  FileText,
  FolderTree,
  RefreshCw,
  ChevronRight,
  X,
} from "lucide-react";
import { ArtifactsPanel } from "./ArtifactsPanel";
import { DiagramGallery } from "./DiagramGallery";
import { FileViewer } from "./FileViewer";
import { RequirementsForm } from "./RequirementsForm";
import type { StepProgress } from "./WorkflowStepper";
import { fileTabId, fileTabPath, type CanvasTab } from "./WorkspaceView";
import { listDir } from "../lib/api";
import type { ArtifactDef } from "../lib/artifacts";
import type { ClarifyAnswer, ClarifyQuestion } from "../lib/clarify";
import type { FileEntry } from "../lib/types";

function basename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

/** One tree row; directories lazily load their children on first expand. */
function TreeNode({
  entry,
  depth,
  selected,
  onSelect,
}: {
  entry: FileEntry;
  depth: number;
  selected: string | null;
  onSelect: (path: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [children, setChildren] = useState<FileEntry[] | null>(null);
  const [loading, setLoading] = useState(false);

  const toggle = async () => {
    if (!entry.isDir) {
      onSelect(entry.path);
      return;
    }
    const next = !open;
    setOpen(next);
    if (next && children === null) {
      setLoading(true);
      try {
        setChildren(await listDir(entry.path));
      } catch {
        setChildren([]);
      } finally {
        setLoading(false);
      }
    }
  };

  const isSelected = selected === entry.path;

  return (
    <div>
      <button
        type="button"
        onClick={toggle}
        title={entry.name}
        style={{ paddingLeft: 8 + depth * 14 }}
        className={
          "flex w-full items-center gap-1.5 rounded-md py-1 pr-2 text-left text-[12.5px] transition-colors " +
          (isSelected ? "bg-accent-tint text-accent" : "text-ink-muted hover:bg-subtle")
        }
      >
        {entry.isDir ? (
          <>
            <ChevronRight
              size={13}
              className={"shrink-0 transition-transform " + (open ? "rotate-90" : "")}
            />
            {open ? (
              <FolderOpen size={14} className="shrink-0 text-accent" />
            ) : (
              <Folder size={14} className="shrink-0" />
            )}
          </>
        ) : (
          <>
            <span className="w-[13px] shrink-0" />
            <FileText size={14} className="shrink-0" />
          </>
        )}
        <span className="truncate font-mono">{entry.name}</span>
      </button>
      {entry.isDir && open && (
        <div>
          {loading && (
            <div className="py-1 text-[11.5px] text-ink-faint" style={{ paddingLeft: 8 + (depth + 1) * 14 }}>
              …
            </div>
          )}
          {children?.map((c) => (
            <TreeNode key={c.path} entry={c} depth={depth + 1} selected={selected} onSelect={onSelect} />
          ))}
        </div>
      )}
    </div>
  );
}

export function CanvasPanel({
  workdir,
  codebasePath,
  refreshNonce,
  openFiles,
  onOpenFile,
  onCloseFile,
  tab,
  onTabChange,
  clarify,
  clarifyPrefill,
  prefillNonce,
  onSubmitAnswers,
  ragResult,
  artifacts,
  stepProgress,
  artifactSel,
  onSelectArtifact,
  streaming,
}: {
  workdir: string | null;
  /** The project's analyzed codebase folder (D45) — adds a tree-root toggle. */
  codebasePath?: string | null;
  /** Bumped to force a file-tree reload (e.g. after a document step writes a file). */
  refreshNonce?: number;
  /** Paths with an open `file:<path>` viewer tab (tab order, D49). */
  openFiles: string[];
  /** Open (or re-activate) a file's viewer tab. */
  onOpenFile: (path: string) => void;
  /** Close a file's viewer tab. */
  onCloseFile: (path: string) => void;
  /** Which canvas view is active. */
  tab: CanvasTab;
  onTabChange: (tab: CanvasTab) => void;
  /** Pending requirements-clarification questions, or null. */
  clarify: ClarifyQuestion[] | null;
  /** Answers pre-filled from the launcher prompt (prefill pass), or null. */
  clarifyPrefill?: Record<string, string | string[]> | null;
  /** Bumped when a fresh prefill arrives (re-inits the form). */
  prefillNonce?: number;
  onSubmitAnswers: (answers: ClarifyAnswer[]) => void;
  /** The latest RAG search result (in-memory HTML for the "검색 결과" tab, D46). */
  ragResult: { query: string; html: string } | null;
  /** The workflow's document artifacts (D58) — gates the 산출물/다이어그램 tabs. */
  artifacts: ArtifactDef[];
  /** Live workflow status mirrored from ChatPanel (null → existence-only). */
  stepProgress: StepProgress[] | null;
  /** The 산출물 tab's selected artifact (stepId), or null → auto-pick. */
  artifactSel: string | null;
  onSelectArtifact: (stepId: string) => void;
  /** True while a run is streaming (form submit disabled). */
  streaming: boolean;
}) {
  const [root, setRoot] = useState<FileEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Which folder the file tree shows: the workdir (outputs) or the analyzed
  // codebase. Falls back to the workdir while no codebase is set.
  const [rootChoice, setRootChoice] = useState<"workdir" | "codebase">("workdir");
  const treeRoot = rootChoice === "codebase" && codebasePath ? codebasePath : workdir;

  const load = useCallback(async () => {
    if (!treeRoot) {
      setRoot(null);
      return;
    }
    setError(null);
    try {
      setRoot(await listDir(treeRoot));
    } catch (e) {
      setError(String(e));
      setRoot([]);
    }
  }, [treeRoot]);

  // Reload on root change or when a produced file bumps refreshNonce.
  useEffect(() => {
    void load();
  }, [load, refreshNonce]);

  // The 산출물/다이어그램 tabs exist while the workflow has document artifacts
  // and the workdir is resolved (D58 — plain-chat categories never show them).
  const hasArtifacts = !!workdir && artifacts.length > 0;

  // The requirements tab exists only while the form awaits the user (a stale
  // `tab === "requirements"` can never render a pill-less view); the rag tab
  // exists once a search result arrived and stays for the session; the
  // artifacts/diagrams tabs exist while `hasArtifacts` (D58); a file tab
  // exists only while its path is in `openFiles` (D49).
  const activeFilePath = fileTabPath(tab);
  const effectiveTab: CanvasTab =
    tab === "requirements"
      ? clarify?.length
        ? "requirements"
        : "files"
      : tab === "rag"
        ? ragResult
          ? "rag"
          : "files"
        : tab === "artifacts" || tab === "diagrams"
          ? hasArtifacts
            ? tab
            : "files"
          : activeFilePath
            ? openFiles.includes(activeFilePath)
              ? tab
              : "files"
            : tab;
  const effectiveFilePath = fileTabPath(effectiveTab);

  const tabBtn = (id: CanvasTab, label: string, badge?: boolean) => (
    <button
      type="button"
      onClick={() => onTabChange(id)}
      className={
        "relative shrink-0 rounded-md px-2.5 py-1 text-[12.5px] font-medium transition-colors " +
        (effectiveTab === id
          ? "bg-panel text-ink-strong shadow-xs"
          : "text-ink-soft hover:text-ink-muted")
      }
    >
      {label}
      {badge && (
        <span className="absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full bg-accent" />
      )}
    </button>
  );

  // One closable pill per open file (nested controls, so a div — not a button).
  const fileTabPill = (path: string) => {
    const id = fileTabId(path);
    const active = effectiveTab === id;
    return (
      <div
        key={path}
        className={
          "flex shrink-0 items-center gap-1 rounded-md pl-2.5 pr-1 py-1 text-[12.5px] font-medium transition-colors " +
          (active ? "bg-panel text-ink-strong shadow-xs" : "text-ink-soft hover:text-ink-muted")
        }
      >
        <button
          type="button"
          onClick={() => onTabChange(id)}
          title={path}
          className="max-w-[160px] truncate font-mono text-[11.5px]"
        >
          {basename(path)}
        </button>
        <button
          type="button"
          onClick={() => onCloseFile(path)}
          title="탭 닫기"
          className="grid h-4 w-4 shrink-0 place-items-center rounded text-ink-faint transition-colors hover:bg-subtle hover:text-ink"
        >
          <X size={11} />
        </button>
      </div>
    );
  };

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col bg-app">
      {/* toolbar */}
      <div className="flex h-[46px] shrink-0 items-center gap-2 border-b border-line bg-panel px-3.5">
        <div className="flex min-w-0 items-center gap-0.5 overflow-x-auto rounded-lg border border-line bg-subtle p-0.5">
          {!!clarify?.length && tabBtn("requirements", "요구사항", true)}
          {!!ragResult && tabBtn("rag", "검색 결과")}
          {hasArtifacts && tabBtn("artifacts", "산출물")}
          {hasArtifacts && tabBtn("diagrams", "다이어그램")}
          {tabBtn("files", "파일")}
          {openFiles.map(fileTabPill)}
        </div>
        <div className="flex-1" />
        {effectiveTab === "files" && treeRoot && (
          <>
            {codebasePath && (
              <div className="flex items-center gap-0.5 rounded-lg border border-line bg-subtle p-0.5 text-[11.5px]">
                {(
                  [
                    ["workdir", "작업 폴더"],
                    ["codebase", "코드베이스"],
                  ] as const
                ).map(([id, label]) => (
                  <button
                    key={id}
                    type="button"
                    onClick={() => setRootChoice(id)}
                    className={
                      "rounded-md px-2 py-0.5 font-medium transition-colors " +
                      (rootChoice === id
                        ? "bg-panel text-ink-strong shadow-xs"
                        : "text-ink-soft hover:text-ink-muted")
                    }
                  >
                    {label}
                  </button>
                ))}
              </div>
            )}
            <button
              type="button"
              onClick={() => void load()}
              title="새로고침"
              className="grid h-7 w-7 place-items-center rounded-md text-ink-soft transition-colors hover:bg-subtle hover:text-ink"
            >
              <RefreshCw size={14} />
            </button>
            <span
              title={treeRoot}
              className="inline-flex max-w-[220px] items-center gap-1.5 rounded-md border border-line bg-subtle px-2 py-1 font-mono text-[11.5px] text-ink-soft"
            >
              <Folder size={12} className="shrink-0" />
              <span className="truncate">{basename(treeRoot)}</span>
            </span>
          </>
        )}
      </div>

      {effectiveTab === "requirements" && clarify?.length ? (
        <RequirementsForm
          key={clarify.map((q) => q.id).join("|") + ":" + (prefillNonce ?? 0)}
          questions={clarify}
          initialAnswers={clarifyPrefill ?? undefined}
          disabled={streaming}
          onSubmit={onSubmitAnswers}
        />
      ) : effectiveTab === "rag" && ragResult ? (
        // In-memory result document, sandboxed exactly like the file viewer's
        // HTML preview (allow-scripts, no same-origin) — D46.
        <iframe
          title="사내 문서 검색 결과"
          sandbox="allow-scripts"
          srcDoc={ragResult.html}
          className="h-full w-full flex-1 border-0 bg-white"
        />
      ) : effectiveTab === "artifacts" && workdir ? (
        <ArtifactsPanel
          workdir={workdir}
          artifacts={artifacts}
          stepProgress={stepProgress}
          refreshNonce={refreshNonce}
          selected={artifactSel}
          onSelect={onSelectArtifact}
        />
      ) : effectiveTab === "diagrams" && workdir ? (
        <DiagramGallery workdir={workdir} artifacts={artifacts} refreshNonce={refreshNonce} />
      ) : !treeRoot ? (
        <div className="flex flex-1 flex-col items-center justify-center px-6 text-center text-ink-soft">
          <div className="mb-4 grid h-16 w-16 place-items-center rounded-[18px] bg-subtle text-ink-faint">
            <FolderTree size={30} />
          </div>
          <div className="mb-1.5 font-serif text-[16px] font-semibold text-ink-muted">
            작업 폴더 준비 중
          </div>
          <div className="max-w-[320px] text-[12.5px] leading-[1.5]">
            첫 메시지를 보내면 이 프로젝트 전용 폴더가 자동으로 만들어집니다. 특정 로컬 폴더에서
            작업하려면 홈 화면에서 작업 폴더를 지정하세요.
          </div>
        </div>
      ) : effectiveFilePath ? (
        // One viewer tab per open file (the 파일 tab stays a pure list — D49).
        <FileViewer path={effectiveFilePath} refreshNonce={refreshNonce} />
      ) : (
        <div className="min-h-0 flex-1 overflow-auto py-2" key={treeRoot}>
          {error && <div className="px-3 py-2 text-[12px] text-bad">{error}</div>}
          {root?.length === 0 && !error && (
            <div className="px-3 py-2 text-[12px] text-ink-faint">빈 폴더</div>
          )}
          {root?.map((e) => (
            <TreeNode
              key={e.path}
              entry={e}
              depth={0}
              selected={activeFilePath}
              onSelect={onOpenFile}
            />
          ))}
        </div>
      )}
    </section>
  );
}

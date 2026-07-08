import { useCallback, useEffect, useState } from "react";
import {
  Folder,
  FolderOpen,
  FileText,
  FolderTree,
  RefreshCw,
  ChevronRight,
} from "lucide-react";
import { FileViewer } from "./FileViewer";
import { RequirementsForm } from "./RequirementsForm";
import type { CanvasTab } from "./WorkspaceView";
import { listDir } from "../lib/api";
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
  refreshNonce,
  selectedFile,
  onSelectFile,
  tab,
  onTabChange,
  clarify,
  clarifyPrefill,
  prefillNonce,
  onSubmitAnswers,
  streaming,
}: {
  workdir: string | null;
  /** Bumped to force a file-tree reload (e.g. after a document step writes a file). */
  refreshNonce?: number;
  selectedFile: string | null;
  onSelectFile: (path: string) => void;
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
  /** True while a run is streaming (form submit disabled). */
  streaming: boolean;
}) {
  const [root, setRoot] = useState<FileEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!workdir) {
      setRoot(null);
      return;
    }
    setError(null);
    try {
      setRoot(await listDir(workdir));
    } catch (e) {
      setError(String(e));
      setRoot([]);
    }
  }, [workdir]);

  // Reload on workdir change or when a produced file bumps refreshNonce.
  useEffect(() => {
    void load();
  }, [load, refreshNonce]);

  // The requirements tab exists only while the form awaits the user; once the
  // answers are submitted (clarify cleared) the pill disappears entirely, so a
  // stale `tab === "requirements"` can never render a pill-less view.
  const effectiveTab: CanvasTab = clarify?.length ? tab : "files";

  const tabBtn = (id: CanvasTab, label: string, badge?: boolean) => (
    <button
      type="button"
      onClick={() => onTabChange(id)}
      className={
        "relative rounded-md px-2.5 py-1 text-[12.5px] font-medium transition-colors " +
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

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col bg-app">
      {/* toolbar */}
      <div className="flex h-[46px] shrink-0 items-center gap-2 border-b border-line bg-panel px-3.5">
        <div className="flex items-center gap-0.5 rounded-lg border border-line bg-subtle p-0.5">
          {!!clarify?.length && tabBtn("requirements", "요구사항", true)}
          {tabBtn("files", "파일")}
        </div>
        <div className="flex-1" />
        {effectiveTab === "files" && workdir && (
          <>
            <button
              type="button"
              onClick={() => void load()}
              title="새로고침"
              className="grid h-7 w-7 place-items-center rounded-md text-ink-soft transition-colors hover:bg-subtle hover:text-ink"
            >
              <RefreshCw size={14} />
            </button>
            <span
              title={workdir}
              className="inline-flex max-w-[220px] items-center gap-1.5 rounded-md border border-line bg-subtle px-2 py-1 font-mono text-[11.5px] text-ink-soft"
            >
              <Folder size={12} className="shrink-0" />
              <span className="truncate">{basename(workdir)}</span>
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
      ) : !workdir ? (
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
      ) : (
        <div className="flex min-h-0 flex-1">
          <div className="w-[240px] shrink-0 overflow-auto border-r border-line py-2">
            {error && <div className="px-3 py-2 text-[12px] text-bad">{error}</div>}
            {root?.length === 0 && !error && (
              <div className="px-3 py-2 text-[12px] text-ink-faint">빈 폴더</div>
            )}
            {root?.map((e) => (
              <TreeNode
                key={e.path}
                entry={e}
                depth={0}
                selected={selectedFile}
                onSelect={onSelectFile}
              />
            ))}
          </div>
          <FileViewer path={selectedFile} refreshNonce={refreshNonce} />
        </div>
      )}
    </section>
  );
}

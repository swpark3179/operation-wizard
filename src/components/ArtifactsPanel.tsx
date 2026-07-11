// Canvas 산출물 tab (design D58): one hub for every document the category
// workflow produces (계획서·분석서 등) — a left artifact list with per-step
// status and a right preview, so the "구획별 여러 계획" documents are flipped
// through in place instead of hunting individual file tabs. The list derives
// from the runtime workflow (WorkspaceView), the live status is mirrored from
// ChatPanel's step cursor, and file existence is probed with listDir so loaded
// sessions (no live workflow) still show what's on disk.

import { useMemo } from "react";
import { BookmarkPlus, FileText } from "lucide-react";
import { FileViewer } from "./FileViewer";
import { StatusIcon, type StepProgress, type StepProgressStatus } from "./WorkflowStepper";
import { joinWorkdirPath, normalizePathKey, type ArtifactDef } from "../lib/artifacts";
import { useArtifactExistence } from "../lib/useArtifactExistence";

/** Row status: the live workflow status when a workflow is running, else an
 * existence-derived one (loaded sessions / reopened projects). */
type RowStatus = StepProgressStatus | "exists" | "missing";

const STATUS_LABEL: Record<RowStatus, string> = {
  pending: "대기",
  active: "생성 중",
  done: "완료",
  skipped: "건너뜀",
  halted: "중단",
  exists: "생성됨",
  missing: "미생성",
};

const STATUS_CHIP: Record<RowStatus, string> = {
  pending: "bg-subtle text-ink-soft",
  active: "bg-accent-tint text-accent",
  done: "bg-ok-bg text-ok",
  skipped: "bg-subtle text-ink-soft",
  halted: "bg-warn-bg text-warn",
  exists: "bg-ok-bg text-ok",
  missing: "bg-subtle text-ink-faint",
};

/** Map the existence-derived statuses onto the stepper icon set. */
function iconStatus(status: RowStatus): StepProgressStatus {
  if (status === "exists") return "done";
  if (status === "missing") return "pending";
  return status;
}

function basename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

/** Status-aware placeholder line for a not-yet-created artifact. */
function placeholderNote(status: RowStatus): string {
  switch (status) {
    case "active":
      return "문서를 생성하는 중입니다 — 완료되면 여기에 표시됩니다.";
    case "skipped":
      return "이 단계는 건너뛰어 문서가 생성되지 않았습니다.";
    case "halted":
      return "단계가 중단되어 문서가 생성되지 않았습니다.";
    default:
      return "이 단계가 실행되면 문서가 여기에 표시됩니다.";
  }
}

export function ArtifactsPanel({
  workdir,
  artifacts,
  stepProgress,
  refreshNonce,
  selected,
  onSelect,
  onSaveToKnowledge,
}: {
  workdir: string;
  /** The workflow's document artifacts, in step order (≥1 — gated by the tab pill). */
  artifacts: ArtifactDef[];
  /** Live workflow status (mirrored from ChatPanel), or null for loaded
   * sessions / no running workflow → existence-only statuses. */
  stepProgress: StepProgress[] | null;
  /** Bumped when a workflow step writes a file → re-probe existence. */
  refreshNonce?: number;
  /** The selected artifact's stepId, or null → auto-pick. */
  selected: string | null;
  onSelect: (stepId: string) => void;
  /** Open the 지식 저장 panel pre-checked with this artifact (D59). Only
   * offered for artifacts that exist on disk. */
  onSaveToKnowledge?: (stepId: string) => void;
}) {
  // Which artifact files exist on disk (shared probe hook — D59). A missing
  // folder just means "nothing there yet". Null while the first probe runs.
  const existing = useArtifactExistence(workdir, artifacts, refreshNonce);

  const rows = useMemo(
    () =>
      artifacts.map((a) => {
        const abs = joinWorkdirPath(workdir, a.file);
        const exists = !!existing?.has(normalizePathKey(abs));
        const live = stepProgress?.find((s) => s.id === a.stepId)?.status;
        const status: RowStatus = live ?? (exists ? "exists" : "missing");
        return { artifact: a, abs, exists, status };
      }),
    [artifacts, workdir, existing, stepProgress],
  );

  // Effective selection (derived — the panel stays controlled): the requested
  // artifact when valid, else the first existing one, else the first row.
  const effective =
    rows.find((r) => r.artifact.stepId === selected) ?? rows.find((r) => r.exists) ?? rows[0];

  return (
    <div className="flex min-h-0 flex-1">
      {/* artifact list */}
      <div className="flex w-[190px] shrink-0 flex-col overflow-y-auto border-r border-line bg-panel/40 p-1.5">
        {rows.map((r) => {
          const active = effective?.artifact.stepId === r.artifact.stepId;
          return (
            // A div with nested buttons (the fileTabPill precedent) — the row
            // selects, the hover action saves to knowledge (D59).
            <div
              key={r.artifact.stepId}
              title={r.abs}
              className={
                "group relative rounded-md transition-colors " +
                (active ? "bg-accent-tint" : "hover:bg-subtle")
              }
            >
              <button
                type="button"
                onClick={() => onSelect(r.artifact.stepId)}
                className="flex w-full flex-col gap-0.5 px-2 py-1.5 text-left"
              >
                <span className="flex items-center gap-1.5">
                  <StatusIcon status={iconStatus(r.status)} />
                  <span
                    className={
                      "min-w-0 flex-1 truncate text-[12px] font-medium " +
                      (active ? "text-accent" : "text-ink-muted")
                    }
                  >
                    {r.artifact.name}
                  </span>
                </span>
                <span className="flex min-w-0 items-center gap-1.5 pl-[20px]">
                  <span className="min-w-0 flex-1 truncate font-mono text-[10.5px] text-ink-faint">
                    {basename(r.artifact.file)}
                  </span>
                  <span
                    className={
                      "shrink-0 rounded-full px-1.5 py-px text-[10px] font-medium " +
                      STATUS_CHIP[r.status]
                    }
                  >
                    {STATUS_LABEL[r.status]}
                  </span>
                </span>
              </button>
              {onSaveToKnowledge && r.exists && (
                <button
                  type="button"
                  onClick={() => onSaveToKnowledge(r.artifact.stepId)}
                  title="지식으로 저장"
                  className="absolute right-1 top-1 grid h-5 w-5 place-items-center rounded border border-line bg-panel text-ink-soft opacity-0 shadow-xs transition-opacity hover:text-accent group-hover:opacity-100"
                >
                  <BookmarkPlus size={12} />
                </button>
              )}
            </div>
          );
        })}
      </div>

      {/* preview (min-h-0 so the nested FileViewer scrolls instead of growing) */}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {effective?.exists ? (
          <FileViewer path={effective.abs} refreshNonce={refreshNonce} />
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center px-6 text-center text-ink-soft">
            <div className="mb-4 grid h-16 w-16 place-items-center rounded-[18px] bg-subtle text-ink-faint">
              <FileText size={30} />
            </div>
            <div className="mb-1.5 font-serif text-[16px] font-semibold text-ink-muted">
              아직 생성되지 않았습니다
            </div>
            <div className="max-w-[320px] text-[12.5px] leading-[1.5]">
              {effective ? placeholderNote(effective.status) : "표시할 산출물이 없습니다."}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

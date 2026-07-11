// Canvas "지식 저장" tab (D59): checkbox list of the workflow's artifacts +
// title + an agent-generated (editable) summary → one artifact-kind knowledge
// entry whose files are copied into the knowledge store. Opened by the
// completion banner (all existing artifacts pre-checked) or a 산출물-tab row
// action (that artifact pre-checked). Re-opening in the same session keeps the
// same entry id → saving again upserts instead of duplicating.

import { useEffect, useRef, useState } from "react";
import { BookmarkPlus, Loader2, RefreshCw } from "lucide-react";
import { joinWorkdirPath, normalizePathKey, type ArtifactDef } from "../lib/artifacts";
import {
  generateKnowledgeSummary,
  saveArtifactKnowledge,
} from "../lib/knowledgeSave";
import { useArtifactExistence } from "../lib/useArtifactExistence";
import type { KnowledgeEntry } from "../lib/types";

/** Everything the panel needs that the opener (WorkspaceView) decides:
 * the stable entry id (upsert key), the agent that generates the summary,
 * pre-checked artifacts, and the entry's title/provenance defaults. */
export interface KnowledgeSaveRequest {
  entryId: string;
  agentId: string;
  model: string | null;
  /** StepIds to pre-check (manual save), or null → every existing artifact. */
  preselect: string[] | null;
  defaultTitle: string;
  /** The original request (seed prompt / project title) for the summary turn. */
  request: string;
  source: { projectId: string; category: string; title: string };
}

function basename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

export function KnowledgeSavePanel({
  workdir,
  artifacts,
  refreshNonce,
  request,
  onClose,
  onSaved,
}: {
  workdir: string;
  /** The workflow's document artifacts, in step order. */
  artifacts: ArtifactDef[];
  /** Bumped when a workflow step writes a file → re-probe existence. */
  refreshNonce?: number;
  request: KnowledgeSaveRequest;
  onClose: () => void;
  onSaved: (entry: KnowledgeEntry) => void;
}) {
  const existing = useArtifactExistence(workdir, artifacts, refreshNonce);
  const rows = artifacts.map((a) => {
    const abs = joinWorkdirPath(workdir, a.file);
    return { artifact: a, abs, exists: !!existing?.has(normalizePathKey(abs)) };
  });

  const [checked, setChecked] = useState<Set<string> | null>(null); // stepIds; null until the probe lands
  const [title, setTitle] = useState(request.defaultTitle);
  const [summary, setSummary] = useState("");
  const [genState, setGenState] = useState<"idle" | "running" | "done" | "failed">("idle");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const cancelRef = useRef<(() => void) | null>(null);

  // Initialize the selection once the existence probe lands: the requested
  // artifacts (manual save), else every existing one (completion banner).
  useEffect(() => {
    if (checked !== null || existing === null) return;
    const existingIds = rows.filter((r) => r.exists).map((r) => r.artifact.stepId);
    const pre = request.preselect?.filter((id) => existingIds.includes(id));
    setChecked(new Set(pre?.length ? pre : existingIds));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [existing]);

  const checkedFiles = rows
    .filter((r) => r.exists && checked?.has(r.artifact.stepId))
    .map((r) => r.abs);

  const generate = (files: string[]) => {
    cancelRef.current?.();
    setGenState("running");
    setSummary("");
    const runner = generateKnowledgeSummary({
      agentId: request.agentId,
      model: request.model,
      cwd: workdir,
      files,
      request: request.request,
      onText: setSummary, // live raw feedback; replaced by the parsed body below
    });
    cancelRef.current = runner.cancel;
    void runner.promise.then((parsed) => {
      cancelRef.current = null;
      if (parsed) {
        setSummary(parsed);
        setGenState("done");
      } else {
        setGenState("failed");
      }
    });
  };

  // Auto-start the summary turn once (when the initial selection has files) —
  // the user reviews/edits the result before saving. Unmount cancels it.
  const autoStartedRef = useRef(false);
  useEffect(() => {
    if (autoStartedRef.current || checked === null) return;
    autoStartedRef.current = true;
    if (checkedFiles.length) generate(checkedFiles);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [checked]);
  useEffect(() => () => cancelRef.current?.(), []);

  const toggle = (stepId: string) => {
    setChecked((prev) => {
      const next = new Set(prev ?? []);
      if (next.has(stepId)) next.delete(stepId);
      else next.add(stepId);
      return next;
    });
  };

  const save = async () => {
    if (saving || !title.trim() || checkedFiles.length === 0) return;
    setSaving(true);
    setError(null);
    try {
      const entry = await saveArtifactKnowledge({
        entryId: request.entryId,
        title: title.trim(),
        summary: summary.trim(),
        files: checkedFiles,
        source: request.source,
      });
      onSaved(entry);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-[26px] w-[26px] place-items-center rounded-[7px] bg-accent-tint text-accent">
          <BookmarkPlus size={15} />
        </span>
        <div className="min-w-0">
          <div className="font-serif text-[14.5px] font-semibold text-ink-strong">지식으로 저장</div>
          <div className="text-[11.5px] text-ink-soft">
            선택한 산출물이 지식 저장소로 복사되고, 아래 요약이 이후 작업의 지식 단계에 주입됩니다.
          </div>
        </div>
      </div>

      {/* artifact checkboxes */}
      <div className="mb-3 rounded-xl border border-line bg-panel p-2">
        <div className="px-1 pb-1 text-[11px] font-bold uppercase tracking-[0.08em] text-ink-soft">
          저장할 산출물
        </div>
        {existing === null ? (
          <div className="px-1 py-2 text-[12px] text-ink-faint">산출물 확인 중…</div>
        ) : (
          rows.map((r) => (
            <label
              key={r.artifact.stepId}
              className={
                "flex items-center gap-2 rounded-md px-1.5 py-1.5 text-[12.5px] " +
                (r.exists ? "cursor-pointer text-ink-muted hover:bg-subtle" : "text-ink-faint")
              }
            >
              <input
                type="checkbox"
                disabled={!r.exists}
                checked={r.exists && !!checked?.has(r.artifact.stepId)}
                onChange={() => toggle(r.artifact.stepId)}
                className="accent-[var(--accent)]"
              />
              <span className="min-w-0 flex-1 truncate font-medium">{r.artifact.name}</span>
              <span className="shrink-0 font-mono text-[10.5px] text-ink-faint">
                {basename(r.artifact.file)}
              </span>
              {!r.exists && (
                <span className="shrink-0 rounded-full bg-subtle px-1.5 py-px text-[10px] font-medium text-ink-faint">
                  미생성
                </span>
              )}
            </label>
          ))
        )}
      </div>

      {/* title */}
      <label className="mb-3 block">
        <div className="mb-1 text-[11px] font-bold uppercase tracking-[0.08em] text-ink-soft">제목</div>
        <input
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="지식 제목"
          className="w-full rounded-lg border border-line bg-panel px-2.5 py-2 text-[13px] text-ink outline-none focus:border-line-strong"
        />
      </label>

      {/* summary */}
      <div className="mb-3 flex min-h-0 flex-1 flex-col">
        <div className="mb-1 flex items-center gap-2">
          <div className="text-[11px] font-bold uppercase tracking-[0.08em] text-ink-soft">
            작업 요약 (이후 작업에 주입)
          </div>
          {genState === "running" && (
            <span className="inline-flex items-center gap-1 text-[11px] text-accent">
              <Loader2 size={11} className="animate-spin" /> 에이전트가 요약 생성 중…
            </span>
          )}
          {genState === "failed" && (
            <span className="text-[11px] text-warn">요약 생성 실패 — 직접 작성하거나 다시 생성하세요.</span>
          )}
          <div className="flex-1" />
          <button
            type="button"
            onClick={() => checkedFiles.length && generate(checkedFiles)}
            disabled={genState === "running" || checkedFiles.length === 0}
            className="inline-flex items-center gap-1 rounded-md border border-line px-1.5 py-0.5 text-[11px] font-medium text-ink-muted transition-colors hover:bg-subtle disabled:opacity-50"
          >
            <RefreshCw size={11} /> 다시 생성
          </button>
        </div>
        <textarea
          value={summary}
          onChange={(e) => setSummary(e.target.value)}
          readOnly={genState === "running"}
          placeholder="무엇을 요청받아 어떤 접근으로 작업했고, 각 문서에 무엇이 담겼는지 — 이후 작업이 참고할 요약을 작성하세요."
          className="min-h-[160px] w-full flex-1 resize-none rounded-lg border border-line bg-panel px-2.5 py-2 font-mono text-[12px] leading-[1.6] text-ink outline-none focus:border-line-strong"
        />
      </div>

      {error && <div className="mb-2 text-[12px] text-bad">{error}</div>}

      <div className="flex shrink-0 items-center gap-2">
        <button
          type="button"
          onClick={() => void save()}
          disabled={saving || !title.trim() || checkedFiles.length === 0}
          className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-[12.5px] font-medium text-white transition-colors hover:bg-accent-strong disabled:opacity-50"
        >
          {saving ? <Loader2 size={13} className="animate-spin" /> : <BookmarkPlus size={13} />}
          지식 베이스에 저장
        </button>
        <button
          type="button"
          onClick={onClose}
          className="rounded-lg border border-line px-3 py-1.5 text-[12.5px] font-medium text-ink-muted transition-colors hover:bg-subtle"
        >
          닫기
        </button>
        {checkedFiles.length === 0 && existing !== null && (
          <span className="text-[11.5px] text-ink-faint">저장할 산출물을 1개 이상 선택하세요.</span>
        )}
      </div>
    </div>
  );
}

// Flows settings: per-category workflow steps + the skill registry, editable
// and persisted to settings.json (design D39/D40). The built-in defaults are
// shown as the initial (sample) content; "기본값으로 되돌리기" clears the
// override (sends `null`) so future app defaults apply again.

import { useEffect, useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronUp,
  FolderOpen,
  Pin,
  Plus,
  RotateCcw,
  Trash2,
  X,
} from "lucide-react";
import { ask } from "@tauri-apps/plugin-dialog";
import { CATEGORIES, type Category } from "./workspace";
import { pickFolder, setSkills, setWorkflow } from "../lib/api";
import { skillList } from "../lib/skills";
import {
  CATEGORY_FOUNDATION,
  DEFAULT_FOUNDATION_STEPS,
  DEFAULT_WORKFLOWS,
  FOUNDATION_KINDS,
  stepOutput,
  workflowFor,
} from "../lib/workflow";
import { useAutoGrow } from "../lib/useAutoGrow";
import type { Settings, SkillDef, StepDef } from "../lib/types";

const KIND_LABEL: Record<string, string> = {
  search: "조사",
  document: "문서 생성",
  chat: "대화",
  codebase: "코드베이스 분석",
  rag: "RAG 검색",
  knowledge: "지식 주입",
};

/** Kinds offered by the step-kind select — the foundation kinds are pinned
 * cards only (added/removed via the foundation toggle, never per-step). */
const EDITABLE_KINDS = ["search", "document", "chat"] as const;

const OUTPUT_LABEL: Record<string, string> = {
  chat: "대화만",
  file: "파일",
  html: "HTML 캔버스",
};

/** Kinds whose result form is configurable (rag/knowledge inject context only;
 * chat is terminal). */
const OUTPUT_KINDS = ["search", "document", "codebase"] as const;

const isFoundationKind = (kind: string) => FOUNDATION_KINDS.includes(kind);

function cloneSteps(steps: StepDef[]): StepDef[] {
  return steps.map((s) => ({ ...s, skillIds: [...s.skillIds] }));
}

/** Client-side mirror of settings.rs `validate_steps` (backend re-validates),
 * plus the foundation pinning rule (D44): foundation steps, when present, must
 * be the leading prefix in canonical order (the pinned UI already enforces
 * this; the mirror guards pasted/legacy drafts). */
function stepsError(steps: StepDef[]): string | null {
  if (steps.length === 0) return "단계가 최소 1개 필요합니다.";
  if (steps.some((s) => !s.name.trim())) return "모든 단계에 이름을 입력하세요.";
  if (steps[steps.length - 1].kind !== "chat")
    return "마지막 단계는 '대화'여야 합니다 (사용자 입력 대기 지점).";
  const foundationIdx = steps
    .map((s, i) => (isFoundationKind(s.kind) ? i : -1))
    .filter((i) => i >= 0);
  if (foundationIdx.length > 0) {
    const inPrefix = foundationIdx.every((idx, n) => idx === n);
    // Foundation steps need not be the full trio (guide uses rag+knowledge only,
    // D63) — but the kinds present must follow the canonical order as a
    // subsequence, i.e. their FOUNDATION_KINDS indices strictly increase.
    const order = foundationIdx.map((idx) => FOUNDATION_KINDS.indexOf(steps[idx].kind));
    const canonical = order.every((v, n) => n === 0 || v > order[n - 1]);
    if (!inPrefix || !canonical)
      return "기반 단계(코드베이스 분석 → RAG 검색 → 지식 주입)는 맨 앞에 순서대로 있어야 합니다.";
  }
  return null;
}

/** One workflow step editor card. Foundation steps render pinned: no delete /
 * move / kind change, but instruction, skills, file and output stay editable. */
function StepCard({
  step,
  index,
  total,
  skills,
  disabled,
  onChange,
  onMove,
  onRemove,
}: {
  step: StepDef;
  index: number;
  total: number;
  /** The effective (saved) skill registry to pick from. */
  skills: SkillDef[];
  disabled: boolean;
  onChange: (next: StepDef) => void;
  onMove: (dir: -1 | 1) => void;
  onRemove: () => void;
}) {
  const instructionRef = useAutoGrow(step.instruction, 220);
  const pinned = isFoundationKind(step.kind);
  const knownIds = new Set(skills.map((s) => s.id));
  // Dangling ids (skill deleted after the step referenced it) stay visible so
  // the user can unlink them; the runtime skips them silently.
  const dangling = step.skillIds.filter((id) => !knownIds.has(id));

  const toggleSkill = (id: string) => {
    const has = step.skillIds.includes(id);
    onChange({
      ...step,
      skillIds: has ? step.skillIds.filter((x) => x !== id) : [...step.skillIds, id],
    });
  };

  return (
    <div className="rounded-[12px] border border-line bg-panel p-4 shadow-[var(--shadow-sm)]">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-6 w-6 shrink-0 place-items-center rounded-full bg-accent-tint text-[12px] font-semibold text-accent">
          {index + 1}
        </span>
        <input
          value={step.name}
          onChange={(e) => onChange({ ...step, name: e.target.value })}
          disabled={disabled}
          placeholder="단계 이름 (진행 표시에 사용)"
          className="min-w-0 flex-1 rounded-[6px] border border-line bg-elevated px-2.5 py-1.5 text-[13px] text-ink outline-none focus:border-accent"
        />
        {pinned ? (
          <>
            <span className="shrink-0 rounded-[6px] border border-line bg-subtle px-2 py-1.5 text-[12.5px] text-ink-muted">
              {KIND_LABEL[step.kind] ?? step.kind}
            </span>
            <span
              title="필수 기반 단계 — 삭제·이동·종류 변경 불가 (지시문·스킬은 편집 가능)"
              className="inline-flex shrink-0 items-center gap-1 rounded-full bg-accent-tint px-2 py-0.5 text-[11px] font-medium text-accent"
            >
              <Pin size={11} /> 기반 단계
            </span>
          </>
        ) : (
          <>
            <select
              value={step.kind}
              onChange={(e) => onChange({ ...step, kind: e.target.value })}
              disabled={disabled}
              title="단계 종류: 조사·문서 생성은 자동 진행, 대화는 사용자 입력 대기"
              className="rounded-[6px] border border-line bg-panel px-1.5 py-1.5 text-[12.5px] text-ink-muted outline-none"
            >
              {EDITABLE_KINDS.map((k) => (
                <option key={k} value={k}>
                  {KIND_LABEL[k]}
                </option>
              ))}
            </select>
            <div className="flex shrink-0 items-center">
              <button
                type="button"
                onClick={() => onMove(-1)}
                disabled={disabled || index === 0}
                title="위로"
                className="grid h-7 w-7 place-items-center rounded-md text-ink-soft transition-colors hover:bg-subtle hover:text-ink disabled:opacity-30 disabled:hover:bg-transparent"
              >
                <ChevronUp size={15} />
              </button>
              <button
                type="button"
                onClick={() => onMove(1)}
                disabled={disabled || index === total - 1}
                title="아래로"
                className="grid h-7 w-7 place-items-center rounded-md text-ink-soft transition-colors hover:bg-subtle hover:text-ink disabled:opacity-30 disabled:hover:bg-transparent"
              >
                <ChevronDown size={15} />
              </button>
              <button
                type="button"
                onClick={onRemove}
                disabled={disabled}
                title="단계 삭제"
                className="grid h-7 w-7 place-items-center rounded-md text-ink-soft transition-colors hover:bg-bad-bg hover:text-bad disabled:opacity-30"
              >
                <Trash2 size={14} />
              </button>
            </div>
          </>
        )}
      </div>

      {(step.kind === "document" || step.kind === "codebase") && (
        <input
          value={step.file ?? ""}
          onChange={(e) => onChange({ ...step, file: e.target.value || null })}
          disabled={disabled}
          placeholder="산출물 파일 경로 (예: docs/plan.md) — 완료 시 캔버스에 열림"
          className="mb-2.5 w-full rounded-[6px] border border-line bg-elevated px-2.5 py-1.5 font-mono text-[12px] text-ink outline-none focus:border-accent"
        />
      )}

      <textarea
        ref={instructionRef}
        value={step.instruction}
        onChange={(e) => onChange({ ...step, instruction: e.target.value })}
        disabled={disabled}
        rows={2}
        placeholder="이 단계의 지시문 (해당 턴의 프롬프트 앞에 보이지 않게 주입됩니다)"
        className="mb-2.5 max-h-[220px] w-full resize-none overflow-y-auto rounded-[6px] border border-line bg-elevated px-2.5 py-2 font-mono text-[12px] leading-[1.55] text-ink outline-none focus:border-accent"
      />

      {(OUTPUT_KINDS as readonly string[]).includes(step.kind) && (
        <div className="mb-2.5 flex items-center gap-2">
          <span className="text-[11.5px] font-medium text-ink-soft">결과 형태:</span>
          <select
            value={stepOutput(step)}
            onChange={(e) => onChange({ ...step, output: e.target.value })}
            disabled={disabled}
            title="대화만: 산출물 없음 / 파일: 지정한 파일을 캔버스에 열기 / HTML 캔버스: 산출물을 보기 좋은 HTML로 재생성해 캔버스에 표시"
            className="rounded-[6px] border border-line bg-panel px-1.5 py-1 text-[12px] text-ink-muted outline-none"
          >
            {Object.entries(OUTPUT_LABEL).map(([k, label]) => (
              <option key={k} value={k}>
                {label}
              </option>
            ))}
          </select>
          {stepOutput(step) === "html" && (
            <span className="text-[11px] text-ink-faint">
              html-render 스킬 턴이 자동 추가되어 .html로 재생성됩니다
            </span>
          )}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-[11.5px] font-medium text-ink-soft">주입 스킬:</span>
        {skills.map((s) => {
          const on = step.skillIds.includes(s.id);
          return (
            <button
              key={s.id}
              type="button"
              onClick={() => toggleSkill(s.id)}
              disabled={disabled}
              title={s.id}
              className={
                "rounded-full border px-2 py-0.5 text-[11.5px] font-medium transition-colors " +
                (on
                  ? "border-accent bg-accent-tint text-accent"
                  : "border-line text-ink-soft hover:bg-subtle")
              }
            >
              {s.name}
            </button>
          );
        })}
        {dangling.map((id) => (
          <button
            key={id}
            type="button"
            onClick={() => toggleSkill(id)}
            disabled={disabled}
            title="삭제된 스킬 — 실행 시 무시됩니다. 클릭해 연결 해제"
            className="rounded-full border border-line bg-warn-bg px-2 py-0.5 text-[11.5px] font-medium text-warn"
          >
            {id} (삭제됨)
          </button>
        ))}
        {skills.length === 0 && (
          <span className="text-[11.5px] text-ink-faint">등록된 스킬 없음</span>
        )}
      </div>
    </div>
  );
}

/** One category's workflow editor (remounted per category via `key`). */
function WorkflowSection({
  category,
  settings,
  onSettingsChange,
  onDirtyChange,
}: {
  category: Category;
  settings: Settings | null;
  onSettingsChange: (s: Settings) => void;
  /** Unsaved-edit flag mirror: the parent guards the category tab switch (the
   * `key={category}` remount would otherwise silently discard the draft, D57). */
  onDirtyChange?: (dirty: boolean) => void;
}) {
  const [draft, setDraft] = useState<StepDef[]>(() =>
    cloneSteps(workflowFor(category, settings)),
  );
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);

  const skills = skillList(settings);
  const isCustom = !!settings?.workflows?.[category];
  const validation = stepsError(draft);
  // Foundation state (D44): presence of foundation steps in the draft IS the
  // flag. plan always carries them (workflowFor pins them); other categories
  // opt in via the toggle below.
  const foundationOn = draft.some((s) => isFoundationKind(s.kind));
  const foundationCount = draft.filter((s) => isFoundationKind(s.kind)).length;
  // The foundation kinds this category pins (guide omits codebase — D63);
  // drives the toggle label so it matches what turning it on inserts.
  const foundationLabel = CATEGORY_FOUNDATION[category].map((k) => KIND_LABEL[k]).join(" · ");

  const touch = () => {
    setSaved(false);
    setDirty(true);
  };
  const update = (i: number, next: StepDef) => {
    setDraft((d) => d.map((s, idx) => (idx === i ? next : s)));
    touch();
  };
  const move = (i: number, dir: -1 | 1) => {
    setDraft((d) => {
      const j = i + dir;
      // Never move into (or out of) the pinned foundation prefix.
      if (j < foundationCount || j >= d.length) return d;
      const next = [...d];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });
    touch();
  };
  const toggleFoundation = () => {
    setDraft((d) => {
      if (foundationOn) return d.filter((s) => !isFoundationKind(s.kind));
      // Seed with the category's own foundation steps so guide gets rag+knowledge
      // (its tuned instructions) rather than the full trio (D63); fall back to the
      // generic defaults filtered by CATEGORY_FOUNDATION.
      const fromDefault = (DEFAULT_WORKFLOWS[category] ?? []).filter((s) => isFoundationKind(s.kind));
      const seed = fromDefault.length
        ? fromDefault
        : DEFAULT_FOUNDATION_STEPS.filter((s) => CATEGORY_FOUNDATION[category].includes(s.kind));
      return [...cloneSteps(seed), ...d];
    });
    touch();
  };
  const remove = (i: number) => {
    setDraft((d) => d.filter((_, idx) => idx !== i));
    touch();
  };
  const add = () => {
    const step: StepDef = {
      id: crypto.randomUUID(),
      name: "",
      kind: "search",
      instruction: "",
      file: null,
      skillIds: [],
    };
    // Keep the terminal chat step last: insert new steps just before it.
    setDraft((d) =>
      d.length && d[d.length - 1].kind === "chat"
        ? [...d.slice(0, -1), step, d[d.length - 1]]
        : [...d, step],
    );
    touch();
  };

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const next = await setWorkflow(category, draft);
      onSettingsChange(next);
      setSaved(true);
      setDirty(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const reset = async () => {
    // One click used to wipe the whole custom workflow — confirm first (D57).
    const ok = await ask(
      "이 카테고리의 단계 설정을 앱 기본값으로 되돌릴까요?\n저장된 사용자 정의 단계는 삭제됩니다.",
      { title: "기본값으로 되돌리기", kind: "warning" },
    );
    if (!ok) return;
    setSaving(true);
    setError(null);
    try {
      const next = await setWorkflow(category, null);
      onSettingsChange(next);
      setDraft(cloneSteps(workflowFor(category, next)));
      setSaved(true);
      setDirty(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div>
      <div className="mb-2.5 flex items-center gap-2">
        <span className="text-[12.5px] text-ink-muted">
          이 카테고리로 새 작업을 시작하면 아래 단계가 순서대로 진행됩니다. 조사·문서 생성
          단계는 자동으로 이어지고, 대화 단계에서 멈춰 사용자 입력을 기다립니다.
        </span>
        <span
          className={
            "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium " +
            (isCustom ? "bg-accent-tint text-accent" : "bg-subtle text-ink-soft")
          }
        >
          {isCustom ? "사용자 정의" : "기본값"}
        </span>
      </div>

      {category === "plan" ? (
        <div className="mb-3 rounded-[10px] border border-line bg-subtle px-3 py-2 text-[12px] text-ink-muted">
          이 카테고리는 기반 3단계(코드베이스 분석 → RAG 검색 → 지식 주입)가 항상 먼저
          실행됩니다. 지시문과 스킬 연결은 아래 카드에서 편집할 수 있습니다.
        </div>
      ) : (
        <label className="mb-3 flex cursor-pointer items-center gap-2 rounded-[10px] border border-line bg-subtle px-3 py-2 text-[12.5px] text-ink-muted">
          <input
            type="checkbox"
            checked={foundationOn}
            onChange={toggleFoundation}
            disabled={saving}
            className="accent-[var(--accent)]"
          />
          기반 단계 사용 — {foundationLabel}을 먼저 실행
        </label>
      )}

      <div className="flex flex-col gap-2.5">
        {draft.map((s, i) => (
          <StepCard
            key={s.id}
            step={s}
            index={i}
            total={draft.length}
            skills={skills}
            disabled={saving}
            onChange={(next) => update(i, next)}
            onMove={(dir) => move(i, dir)}
            onRemove={() => remove(i)}
          />
        ))}
      </div>

      <div className="mt-3 flex items-center gap-2">
        <button
          type="button"
          onClick={add}
          disabled={saving}
          className="inline-flex items-center gap-1.5 rounded-[6px] border border-line px-2.5 py-1.5 text-[12.5px] font-medium text-ink-muted transition-colors hover:bg-subtle"
        >
          <Plus size={14} /> 단계 추가
        </button>
        <div className="flex-1" />
        {dirty && !validation && (
          <span className="text-[12px] text-warn">저장되지 않은 변경</span>
        )}
        {validation && <span className="text-[12px] text-bad">{validation}</span>}
        {!validation && saved && <span className="text-[12.5px] text-ok">Saved.</span>}
        {error && <span className="max-w-[280px] truncate text-[12px] text-bad" title={error}>{error}</span>}
        <button
          type="button"
          onClick={() => void reset()}
          disabled={saving || !isCustom}
          title="이 카테고리의 단계를 앱 기본값으로 되돌립니다"
          className="inline-flex items-center gap-1.5 rounded-[6px] border border-line px-2.5 py-1.5 text-[12.5px] font-medium text-ink-muted transition-colors hover:bg-subtle disabled:opacity-40"
        >
          <RotateCcw size={13} /> 기본값으로
        </button>
        <button
          type="button"
          onClick={() => void save()}
          disabled={saving || !!validation}
          className="rounded-[6px] bg-accent px-3.5 py-1.5 text-[13px] font-medium text-white transition-colors hover:bg-accent-strong disabled:opacity-50"
        >
          {saving ? "저장 중…" : "저장"}
        </button>
      </div>
    </div>
  );
}

/** One collapsible skill editor card. */
function SkillCard({
  skill,
  usedBy,
  disabled,
  initiallyOpen,
  onChange,
  onRemove,
}: {
  skill: SkillDef;
  usedBy: string[];
  disabled: boolean;
  initiallyOpen: boolean;
  onChange: (next: SkillDef) => void;
  onRemove: () => void;
}) {
  const [open, setOpen] = useState(initiallyOpen);
  const bodyRef = useAutoGrow(skill.body, 320);

  return (
    <div className="rounded-[12px] border border-line bg-panel p-4 shadow-[var(--shadow-sm)]">
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          className="flex min-w-0 flex-1 items-center gap-1.5 text-left text-[13px] font-medium text-ink-muted hover:text-ink"
        >
          <ChevronDown
            size={15}
            className={
              "shrink-0 transition-transform duration-[120ms] " + (open ? "rotate-0" : "-rotate-90")
            }
          />
          <span className="truncate text-ink-strong">{skill.name.trim() || "(이름 없음)"}</span>
        </button>
        {usedBy.length > 0 ? (
          <span
            title={usedBy.join(", ")}
            className="max-w-[300px] truncate rounded-full bg-subtle px-2 py-0.5 text-[11px] font-medium text-ink-soft"
          >
            사용: {usedBy.join(", ")}
          </span>
        ) : (
          <span className="rounded-full bg-subtle px-2 py-0.5 text-[11px] font-medium text-ink-faint">
            미사용
          </span>
        )}
        <button
          type="button"
          onClick={onRemove}
          disabled={disabled}
          title={
            usedBy.length
              ? "사용 중인 스킬입니다 — 삭제하면 해당 단계에서 무시됩니다"
              : "스킬 삭제"
          }
          className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-ink-soft transition-colors hover:bg-bad-bg hover:text-bad disabled:opacity-30"
        >
          <Trash2 size={14} />
        </button>
      </div>

      {open && (
        <div className="mt-3">
          <input
            value={skill.name}
            onChange={(e) => onChange({ ...skill, name: e.target.value })}
            disabled={disabled}
            placeholder="스킬 이름"
            className="mb-2 w-full rounded-[6px] border border-line bg-elevated px-2.5 py-1.5 text-[13px] text-ink outline-none focus:border-accent"
          />
          <textarea
            ref={bodyRef}
            value={skill.body}
            onChange={(e) => onChange({ ...skill, body: e.target.value })}
            disabled={disabled}
            rows={4}
            placeholder={"스킬 지시문 (마크다운). 관례: 첫 줄을 [시스템 스킬: 이름] 헤더로 시작"}
            className="max-h-[320px] w-full resize-none overflow-y-auto rounded-[6px] border border-line bg-elevated px-2.5 py-2 font-mono text-[12px] leading-[1.55] text-ink outline-none focus:border-accent"
          />
          {/* Resource folder (claude-skill style, D45): the path is mentioned in
              the prompt on injection and granted read access (--add-dir). */}
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <span className="text-[11.5px] font-medium text-ink-soft">리소스 폴더:</span>
            <button
              type="button"
              disabled={disabled}
              onClick={() => {
                void pickFolder().then((dir) => {
                  if (dir) onChange({ ...skill, dir });
                });
              }}
              className="inline-flex items-center gap-1.5 rounded-[6px] border border-line px-2 py-1 text-[12px] font-medium text-ink-muted transition-colors hover:bg-subtle disabled:opacity-40"
            >
              <FolderOpen size={13} />
              {skill.dir ? "변경…" : "폴더 선택…"}
            </button>
            {skill.dir && (
              <span
                title={skill.dir}
                className="inline-flex min-w-0 items-center gap-1.5 rounded-md border border-line bg-subtle px-2 py-1 font-mono text-[11px] text-ink-muted"
              >
                <span className="max-w-[280px] truncate">{skill.dir}</span>
                <button
                  type="button"
                  disabled={disabled}
                  onClick={() => onChange({ ...skill, dir: null })}
                  title="폴더 해제"
                  className="shrink-0 text-ink-faint transition-colors hover:text-ink"
                >
                  <X size={11} />
                </button>
              </span>
            )}
            <span className="text-[11px] text-ink-faint">
              주입 시 경로가 에이전트에 전달되고 읽기 권한(--add-dir)이 추가됩니다
            </span>
          </div>
          <div className="mt-1 font-mono text-[11px] text-ink-faint">id: {skill.id}</div>
        </div>
      )}
    </div>
  );
}

/** The skill registry editor (global across categories). */
function SkillLibrary({
  settings,
  onSettingsChange,
}: {
  settings: Settings | null;
  onSettingsChange: (s: Settings) => void;
}) {
  const [draft, setDraft] = useState<SkillDef[]>(() => skillList(settings).map((s) => ({ ...s })));
  const [freshIds, setFreshIds] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isCustom = !!settings?.skills;
  const validation = draft.some((s) => !s.name.trim())
    ? "모든 스킬에 이름을 입력하세요."
    : null;

  // Where each skill is referenced, across every category's effective workflow.
  const usedBy = useMemo(() => {
    const map: Record<string, string[]> = {};
    for (const c of CATEGORIES) {
      workflowFor(c.id, settings).forEach((step, i) => {
        for (const id of step.skillIds) {
          (map[id] ??= []).push(`${c.label} ${i + 1}단계`);
        }
      });
    }
    return map;
  }, [settings]);

  const add = () => {
    const skill: SkillDef = { id: crypto.randomUUID(), name: "", body: "" };
    setDraft((d) => [...d, skill]);
    setFreshIds((s) => new Set(s).add(skill.id));
    setSaved(false);
    setDirty(true);
  };

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const next = await setSkills(draft);
      onSettingsChange(next);
      setSaved(true);
      setDirty(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const reset = async () => {
    // One click used to wipe the whole custom registry — confirm first (D57).
    const ok = await ask(
      "스킬 목록을 앱 기본값으로 되돌릴까요?\n저장된 사용자 정의 스킬은 삭제됩니다.",
      { title: "기본값으로 되돌리기", kind: "warning" },
    );
    if (!ok) return;
    setSaving(true);
    setError(null);
    try {
      const next = await setSkills(null);
      onSettingsChange(next);
      setDraft(skillList(next).map((s) => ({ ...s })));
      setFreshIds(new Set());
      setSaved(true);
      setDirty(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div>
      <div className="mb-2.5 flex items-center gap-2">
        <span className="text-[12.5px] text-ink-muted">
          스킬은 단계에 연결되는 지시문 묶음입니다. 해당 단계가 실행되는 턴의 프롬프트 앞에
          보이지 않게 주입됩니다.
        </span>
        <span
          className={
            "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium " +
            (isCustom ? "bg-accent-tint text-accent" : "bg-subtle text-ink-soft")
          }
        >
          {isCustom ? "사용자 정의" : "기본값"}
        </span>
      </div>

      <div className="flex flex-col gap-2.5">
        {draft.map((s, i) => (
          <SkillCard
            key={s.id}
            skill={s}
            usedBy={usedBy[s.id] ?? []}
            disabled={saving}
            initiallyOpen={freshIds.has(s.id)}
            onChange={(next) => {
              setDraft((d) => d.map((x, idx) => (idx === i ? next : x)));
              setSaved(false);
              setDirty(true);
            }}
            onRemove={() => {
              setDraft((d) => d.filter((_, idx) => idx !== i));
              setSaved(false);
              setDirty(true);
            }}
          />
        ))}
      </div>

      <div className="mt-3 flex items-center gap-2">
        <button
          type="button"
          onClick={add}
          disabled={saving}
          className="inline-flex items-center gap-1.5 rounded-[6px] border border-line px-2.5 py-1.5 text-[12.5px] font-medium text-ink-muted transition-colors hover:bg-subtle"
        >
          <Plus size={14} /> 스킬 추가
        </button>
        <div className="flex-1" />
        {dirty && !validation && (
          <span className="text-[12px] text-warn">저장되지 않은 변경</span>
        )}
        {validation && <span className="text-[12px] text-bad">{validation}</span>}
        {!validation && saved && <span className="text-[12.5px] text-ok">Saved.</span>}
        {error && <span className="max-w-[280px] truncate text-[12px] text-bad" title={error}>{error}</span>}
        <button
          type="button"
          onClick={() => void reset()}
          disabled={saving || !isCustom}
          title="스킬 목록을 앱 기본값으로 되돌립니다"
          className="inline-flex items-center gap-1.5 rounded-[6px] border border-line px-2.5 py-1.5 text-[12.5px] font-medium text-ink-muted transition-colors hover:bg-subtle disabled:opacity-40"
        >
          <RotateCcw size={13} /> 기본값으로
        </button>
        <button
          type="button"
          onClick={() => void save()}
          disabled={saving || !!validation}
          className="rounded-[6px] bg-accent px-3.5 py-1.5 text-[13px] font-medium text-white transition-colors hover:bg-accent-strong disabled:opacity-50"
        >
          {saving ? "저장 중…" : "저장"}
        </button>
      </div>
    </div>
  );
}

export function FlowSettingsView({
  settings,
  onSettingsChange,
}: {
  settings: Settings | null;
  /** Mutations return the full new Settings; App replaces its state with it. */
  onSettingsChange: (s: Settings) => void;
}) {
  const [category, setCategory] = useState<Category>("plan");
  // The category switch remounts WorkflowSection (key), which would silently
  // discard an unsaved draft — confirm when it is dirty (D57).
  const [workflowDirty, setWorkflowDirty] = useState(false);

  const switchCategory = async (id: Category) => {
    if (id === category) return;
    if (workflowDirty) {
      const ok = await ask(
        "저장하지 않은 단계 변경이 있습니다. 카테고리를 이동하면 변경 내용이 사라집니다.\n계속할까요?",
        { title: "Flows", kind: "warning" },
      );
      if (!ok) return;
    }
    setWorkflowDirty(false);
    setCategory(id);
  };

  return (
    <div className="mx-auto max-w-[760px] px-6 py-7">
      <div className="mb-4">
        <h2 className="font-serif text-[22px] font-semibold tracking-[-0.02em] text-ink-strong">
          Flows
        </h2>
        <p className="mt-0.5 text-[13px] text-ink-muted">
          카테고리별 작업 단계와 단계에 주입할 스킬을 설정합니다. 저장하지 않으면 앱 기본값이
          사용되며, 진행 중인 워크스페이스에는 새 작업부터 적용됩니다.
        </p>
      </div>

      {/* category tabs */}
      <div className="mb-4 inline-flex items-center gap-0.5 rounded-lg border border-line bg-subtle p-0.5">
        {CATEGORIES.map((c) => (
          <button
            key={c.id}
            type="button"
            onClick={() => void switchCategory(c.id)}
            className={
              "rounded-md px-2.5 py-1 text-[12.5px] font-medium transition-colors " +
              (category === c.id
                ? "bg-panel text-ink-strong shadow-xs"
                : "text-ink-soft hover:text-ink-muted")
            }
          >
            {c.label}
          </button>
        ))}
      </div>

      <WorkflowSection
        key={category}
        category={category}
        settings={settings}
        onSettingsChange={onSettingsChange}
        onDirtyChange={setWorkflowDirty}
      />

      <div className="my-6 border-t border-line" />

      <h3 className="mb-2 font-serif text-[17px] font-semibold text-ink-strong">스킬</h3>
      <SkillLibrary settings={settings} onSettingsChange={onSettingsChange} />
    </div>
  );
}

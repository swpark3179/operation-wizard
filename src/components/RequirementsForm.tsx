import { useState } from "react";
import {
  ClipboardCheck,
  Circle,
  CheckCircle2,
  Square,
  CheckSquare,
  Send,
  FolderOpen,
  X,
} from "lucide-react";
import type { ClarifyAnswer, ClarifyQuestion } from "../lib/clarify";
import { pickFolder } from "../lib/api";
import { useAutoGrow } from "../lib/useAutoGrow";

/** Folder answer (type "folder"): a native picker button + the chosen path as
 * a clearable mono chip. The value is the absolute path string, so the shared
 * required-validation works unchanged (D45). */
function FolderAnswer({
  value,
  onChange,
  disabled,
}: {
  value: string;
  onChange: (v: string) => void;
  disabled?: boolean;
}) {
  const [error, setError] = useState<string | null>(null);
  const choose = async () => {
    try {
      const folder = await pickFolder();
      if (folder) onChange(folder);
      setError(null);
    } catch (e) {
      setError(`폴더 선택에 실패했습니다 — ${String(e)}`);
    }
  };
  return (
    <div className="flex flex-wrap items-center gap-2">
      <button
        type="button"
        disabled={disabled}
        onClick={() => void choose()}
        className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-panel px-3 py-2 text-[12.5px] font-medium text-ink-muted transition-colors hover:border-line-strong hover:bg-subtle disabled:opacity-60"
      >
        <FolderOpen size={14} className="text-accent" />
        {value ? "다른 폴더 선택…" : "폴더 선택…"}
      </button>
      {error && <span className="w-full text-[11.5px] text-bad">{error}</span>}
      {value && (
        <span
          title={value}
          className="inline-flex min-w-0 max-w-full items-center gap-1.5 rounded-md border border-line bg-subtle px-2 py-1.5 font-mono text-[11.5px] text-ink-muted"
        >
          <span className="truncate">{value}</span>
          <button
            type="button"
            disabled={disabled}
            onClick={() => onChange("")}
            title="선택 해제"
            className="shrink-0 text-ink-faint transition-colors hover:text-ink disabled:opacity-60"
          >
            <X size={12} />
          </button>
        </span>
      )}
    </div>
  );
}

/** Auto-growing text answer (own component so the hook is called once per field). */
function TextAnswer({
  value,
  onChange,
  disabled,
}: {
  value: string;
  onChange: (v: string) => void;
  disabled?: boolean;
}) {
  const ref = useAutoGrow(value, 160);
  return (
    <textarea
      ref={ref}
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value)}
      rows={2}
      placeholder="답변을 입력하세요…"
      className="max-h-[160px] min-h-[44px] w-full resize-none overflow-y-auto rounded-lg border border-line bg-panel px-2.5 py-2 text-[13px] leading-[1.5] text-ink outline-none placeholder:text-ink-faint focus:border-line-strong disabled:opacity-60"
    />
  );
}

/** One selectable option, rendered as a card button (single = radio, multi =
 * checkbox). Selected uses the app's accent convention; hover lifts the card. */
function OptionCard({
  label,
  selected,
  multi,
  disabled,
  onClick,
}: {
  label: string;
  selected: boolean;
  multi: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  const SelIcon = multi ? CheckSquare : CheckCircle2;
  const EmptyIcon = multi ? Square : Circle;
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      aria-pressed={selected}
      className={
        "flex items-start gap-2.5 rounded-[12px] border p-3 text-left transition-[transform,box-shadow,border-color,background-color] duration-[120ms] disabled:opacity-60 disabled:hover:translate-y-0 disabled:hover:shadow-none " +
        (selected
          ? "border-accent bg-accent-tint text-accent shadow-sm"
          : "border-line bg-panel text-ink hover:-translate-y-0.5 hover:border-line-strong hover:shadow-md")
      }
    >
      {selected ? (
        <SelIcon size={16} className="mt-px shrink-0 text-accent" />
      ) : (
        <EmptyIcon size={16} className="mt-px shrink-0 text-ink-faint" />
      )}
      <span className="text-[13px] font-medium leading-[1.4]">{label}</span>
    </button>
  );
}

/** Interactive requirements-clarification form rendered in the canvas. */
export function RequirementsForm({
  questions,
  initialAnswers,
  disabled,
  onSubmit,
}: {
  questions: ClarifyQuestion[];
  /** Pre-filled answers (e.g. from the prefill pass); missing ids start empty. */
  initialAnswers?: Record<string, string | string[]>;
  /** True while a run is streaming (blocks submit). */
  disabled?: boolean;
  onSubmit: (answers: ClarifyAnswer[]) => void;
}) {
  const [answers, setAnswers] = useState<Record<string, string | string[]>>(() => {
    const init: Record<string, string | string[]> = {};
    for (const q of questions) {
      const pre = initialAnswers?.[q.id];
      if (q.type === "multi") init[q.id] = Array.isArray(pre) ? pre : [];
      else init[q.id] = typeof pre === "string" ? pre : "";
    }
    return init;
  });
  const [attempted, setAttempted] = useState(false);
  const [submitted, setSubmitted] = useState(false);

  const missing = (q: ClarifyQuestion): boolean => {
    if (!q.required) return false;
    const v = answers[q.id];
    return q.type === "multi" ? (v as string[]).length === 0 : !(v as string).trim();
  };
  const canSubmit = !disabled && !submitted && !questions.some(missing);

  const setSingle = (id: string, val: string) => setAnswers((a) => ({ ...a, [id]: val }));
  const setText = (id: string, val: string) => setAnswers((a) => ({ ...a, [id]: val }));
  const toggleMulti = (id: string, opt: string) =>
    setAnswers((a) => {
      const cur = (a[id] as string[]) ?? [];
      return { ...a, [id]: cur.includes(opt) ? cur.filter((x) => x !== opt) : [...cur, opt] };
    });

  const submit = () => {
    setAttempted(true);
    if (questions.some(missing)) return;
    setSubmitted(true);
    onSubmit(
      questions.map((q) => ({ id: q.id, label: q.label, type: q.type, value: answers[q.id] })),
    );
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex-1 overflow-auto px-5 py-5">
        <div className="mx-auto max-w-[560px]">
          <div className="mb-1.5 flex items-center gap-2 font-serif text-[16px] font-semibold text-ink-strong">
            <ClipboardCheck size={18} className="text-accent" />
            요구사항 확인
          </div>
          <p className="mb-5 text-[12.5px] leading-[1.5] text-ink-muted">
            아래 항목을 작성해 제출하면 답변을 반영해 작업을 이어갑니다.
          </p>

          {questions.map((q) => {
            const show = attempted && missing(q);
            return (
              <div key={q.id} className="mb-5">
                <div className="mb-2 text-[13.5px] font-medium text-ink-strong">
                  {q.label}
                  {q.required && <span className="ml-1 text-accent">*</span>}
                </div>

                {q.type === "text" && (
                  <TextAnswer
                    value={(answers[q.id] as string) ?? ""}
                    onChange={(v) => setText(q.id, v)}
                    disabled={disabled || submitted}
                  />
                )}

                {q.type === "folder" && (
                  <FolderAnswer
                    value={(answers[q.id] as string) ?? ""}
                    onChange={(v) => setText(q.id, v)}
                    disabled={disabled || submitted}
                  />
                )}

                {(q.type === "single" || q.type === "multi") && (
                  <div className="grid grid-cols-2 gap-2.5">
                    {q.options?.map((opt) => {
                      const sel =
                        q.type === "multi"
                          ? ((answers[q.id] as string[]) ?? []).includes(opt)
                          : answers[q.id] === opt;
                      return (
                        <OptionCard
                          key={opt}
                          label={opt}
                          selected={sel}
                          multi={q.type === "multi"}
                          disabled={disabled || submitted}
                          onClick={() =>
                            q.type === "multi" ? toggleMulti(q.id, opt) : setSingle(q.id, opt)
                          }
                        />
                      );
                    })}
                  </div>
                )}

                {show && (
                  <div className="mt-1.5 text-[11.5px] text-bad">필수 항목입니다.</div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      <div className="shrink-0 border-t border-line bg-panel px-5 py-3">
        <div className="mx-auto flex max-w-[560px] items-center justify-end gap-2">
          {submitted && <span className="text-[12px] text-ink-soft">제출됨</span>}
          <button
            type="button"
            onClick={submit}
            disabled={!canSubmit}
            className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3.5 py-2 text-[13px] font-semibold text-white transition-colors hover:bg-accent-strong disabled:opacity-50"
          >
            <Send size={14} />
            답변 제출
          </button>
        </div>
      </div>
    </div>
  );
}

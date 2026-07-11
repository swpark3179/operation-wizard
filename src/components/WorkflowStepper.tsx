// Persistent workflow progress strip (D57): a collapsed summary row (segment
// bar + current step) that expands into the full step checklist. Fed by
// ChatPanel's step cursor so the user can always see where a multi-step
// category workflow stands — previously the only signal was a transient
// system note that scrolled away.

import { useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  Circle,
  Loader2,
  MinusCircle,
} from "lucide-react";

export type StepProgressStatus = "pending" | "active" | "done" | "skipped" | "halted";

export interface StepProgress {
  id: string;
  name: string;
  status: StepProgressStatus;
}

const SEGMENT_CLS: Record<StepProgressStatus, string> = {
  pending: "bg-subtle",
  active: "bg-accent animate-pulse",
  done: "bg-ok",
  skipped: "bg-line-strong",
  halted: "bg-warn",
};

function StatusIcon({ status }: { status: StepProgressStatus }) {
  switch (status) {
    case "done":
      return <CheckCircle2 size={14} className="shrink-0 text-ok" />;
    case "active":
      return <Loader2 size={14} className="shrink-0 animate-spin text-accent" />;
    case "skipped":
      return <MinusCircle size={14} className="shrink-0 text-ink-faint" />;
    case "halted":
      return <AlertTriangle size={14} className="shrink-0 text-warn" />;
    default:
      return <Circle size={14} className="shrink-0 text-ink-faint" />;
  }
}

export function WorkflowStepper({ steps }: { steps: StepProgress[] }) {
  const [open, setOpen] = useState(false);
  const total = steps.length;
  const finished = steps.filter((s) => s.status === "done" || s.status === "skipped").length;
  const active = steps.find((s) => s.status === "active");
  const activeIndex = active ? steps.indexOf(active) : -1;
  const halted = steps.some((s) => s.status === "halted");

  const summary = halted
    ? "진행 중단됨 — 일반 대화로 계속합니다"
    : active
      ? `${activeIndex + 1}/${total}단계 · ${active.name}`
      : finished >= total
        ? "모든 단계 완료"
        : "시작 대기 중";

  return (
    <div className="shrink-0 border-b border-line bg-subtle/60 px-3.5 py-2">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        title={open ? "단계 목록 접기" : "단계 목록 펼치기"}
        className="flex w-full items-center gap-2 text-left"
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <div className="flex w-[92px] shrink-0 items-center gap-0.5">
            {steps.map((s) => (
              <span
                key={s.id}
                className={"h-1 min-w-0 flex-1 rounded-full " + SEGMENT_CLS[s.status]}
              />
            ))}
          </div>
          <span className="min-w-0 truncate text-[11.5px] font-medium text-ink-muted">
            {summary}
          </span>
        </div>
        <ChevronDown
          size={14}
          className={
            "shrink-0 text-ink-soft transition-transform duration-[120ms] " +
            (open ? "rotate-180" : "")
          }
        />
      </button>

      {open && (
        <div className="mt-2 flex flex-col gap-1 pb-0.5">
          {steps.map((s, i) => (
            <div key={s.id} className="flex items-center gap-2 text-[12px]">
              <StatusIcon status={s.status} />
              <span
                className={
                  "min-w-0 truncate " +
                  (s.status === "active"
                    ? "font-medium text-ink-strong"
                    : s.status === "done"
                      ? "text-ink-muted"
                      : "text-ink-soft")
                }
              >
                {i + 1}. {s.name}
              </span>
              {s.status === "skipped" && (
                <span className="shrink-0 rounded-full bg-subtle px-1.5 py-px text-[10.5px] text-ink-soft">
                  건너뜀
                </span>
              )}
              {s.status === "halted" && (
                <span className="shrink-0 rounded-full bg-warn-bg px-1.5 py-px text-[10.5px] text-warn">
                  중단됨
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

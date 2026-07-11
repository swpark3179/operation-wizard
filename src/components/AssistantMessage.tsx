import { useState } from "react";
import {
  Sparkles,
  Wrench,
  Brain,
  Check,
  ChevronRight,
  Copy,
  FilePlus2,
  RotateCcw,
} from "lucide-react";
import { MarkdownView } from "./Markdown";
import { errorHint, type ChatMessage, type TimelineEvent } from "./workspace";
import { copyText } from "../lib/clipboard";

function ToolRow({ ev }: { ev: Extract<TimelineEvent, { kind: "toolUse" }> }) {
  const [open, setOpen] = useState(false);
  const preview = JSON.stringify(ev.input);
  return (
    <div className="rounded-lg border border-line-soft bg-elevated">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-ink-muted"
      >
        <ChevronRight
          size={13}
          className={"shrink-0 transition-transform " + (open ? "rotate-90" : "")}
        />
        <Wrench size={13} className="shrink-0 text-accent" />
        <span className="font-mono text-ink-strong">{ev.name}</span>
        {!open && preview.length > 2 && (
          <span className="truncate font-mono text-ink-faint">{preview}</span>
        )}
      </button>
      {open && (
        <pre className="overflow-auto border-t border-line-soft px-2.5 py-2 font-mono text-[11.5px] leading-[1.5] text-ink-muted">
          {JSON.stringify(ev.input, null, 2)}
        </pre>
      )}
    </div>
  );
}

function Timeline({ events }: { events: TimelineEvent[] }) {
  if (events.length === 0) return null;
  return (
    <div className="mt-2 flex flex-col gap-1.5">
      {events.map((ev, i) => {
        if (ev.kind === "toolUse") return <ToolRow key={i} ev={ev} />;
        if (ev.kind === "toolResult")
          return (
            <div
              key={i}
              className={
                "rounded-lg border px-2.5 py-1.5 font-mono text-[11.5px] leading-[1.5] " +
                (ev.isError
                  ? "border-bad-border bg-bad-bg text-bad"
                  : "border-line-soft bg-subtle text-ink-muted")
              }
            >
              <span className="line-clamp-4 whitespace-pre-wrap">{ev.content || "(no output)"}</span>
            </div>
          );
        return (
          <div key={i} className="text-[11px] text-ink-faint">
            {ev.inputTokens ?? 0} in · {ev.outputTokens ?? 0} out tokens
          </div>
        );
      })}
    </div>
  );
}

export function AssistantMessage({
  message,
  speaker,
  onNewSession,
  onRetry,
}: {
  message: ChatMessage;
  /** The running agent's display name (e.g. "Claude Code"); the header shows it
   * so the conversation log identifies who is speaking (D57). */
  speaker?: string;
  /** Offer an inline "new session" recovery on failed turns. */
  onNewSession?: () => void;
  /** Re-send the failed turn in the SAME session (primary recovery, D57).
   * Only passed for the last message of the conversation. */
  onRetry?: () => void;
}) {
  const [showThinking, setShowThinking] = useState(false);
  const [copied, setCopied] = useState(false);
  const empty =
    !message.content && !message.thinking && message.events.length === 0 && !message.error;
  const hint = errorHint(message.error);

  const copy = async () => {
    const ok = await copyText(message.content);
    if (!ok) return;
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="group flex gap-2.5">
      <span className="grid h-7 w-7 shrink-0 place-items-center rounded-lg bg-accent text-white">
        <Sparkles size={15} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="mb-1 flex items-center gap-2">
          <span className="text-[11.5px] font-semibold text-ink-soft">
            {speaker || "Operation Wizard"}
          </span>
          <span className="flex-1" />
          {message.content && !message.streaming && (
            <button
              type="button"
              onClick={() => void copy()}
              title="응답 복사"
              className={
                "grid h-5 w-5 place-items-center rounded text-ink-faint transition-opacity hover:bg-subtle hover:text-ink " +
                (copied ? "opacity-100" : "opacity-0 group-hover:opacity-100")
              }
            >
              {copied ? <Check size={12} className="text-ok" /> : <Copy size={12} />}
            </button>
          )}
        </div>

        {message.thinking && (
          <div className="mb-2">
            <button
              type="button"
              onClick={() => setShowThinking((s) => !s)}
              className="inline-flex items-center gap-1.5 text-[11.5px] text-ink-soft hover:text-ink-muted"
            >
              <Brain size={13} />
              {showThinking ? "reasoning 숨기기" : "reasoning 보기"}
            </button>
            {showThinking && (
              <div className="mt-1 whitespace-pre-wrap rounded-lg border border-line-soft bg-subtle px-2.5 py-2 text-[12px] leading-[1.55] text-ink-muted">
                {message.thinking}
              </div>
            )}
          </div>
        )}

        {message.content &&
          // Plain text while streaming (partial markdown renders jumpily);
          // full markdown once the turn completes (D57).
          (message.streaming ? (
            <div className="whitespace-pre-wrap text-[13.5px] leading-[1.55] text-ink">
              {message.content}
            </div>
          ) : (
            <MarkdownView content={message.content} />
          ))}

        <Timeline events={message.events} />

        {message.error && (
          <div className="mt-2 rounded-lg border border-bad-border bg-bad-bg px-2.5 py-2 text-[12.5px] leading-[1.5] text-bad">
            <div className="whitespace-pre-wrap break-words">{message.error}</div>
            {hint && <div className="mt-1.5 text-ink-muted">{hint}</div>}
            {(onRetry || onNewSession) && (
              <div className="mt-2 flex flex-wrap items-center gap-2">
                {onRetry && (
                  <button
                    type="button"
                    onClick={onRetry}
                    className="inline-flex items-center gap-1.5 rounded-md bg-accent px-2 py-1 text-[11.5px] font-medium text-white transition-colors hover:bg-accent-strong"
                  >
                    <RotateCcw size={13} />
                    다시 시도
                  </button>
                )}
                {onNewSession && (
                  <button
                    type="button"
                    onClick={onNewSession}
                    className="inline-flex items-center gap-1.5 rounded-md border border-line bg-panel px-2 py-1 text-[11.5px] font-medium text-ink-muted transition-colors hover:bg-subtle"
                  >
                    <FilePlus2 size={13} />
                    새 세션으로 다시 시도
                  </button>
                )}
              </div>
            )}
          </div>
        )}

        {empty && message.streaming && (
          <div className="flex items-center gap-1.5 text-[12.5px] text-ink-soft">
            <span className="h-1.5 w-1.5 animate-ping rounded-full bg-accent" />
            생각하는 중…
          </div>
        )}
      </div>
    </div>
  );
}

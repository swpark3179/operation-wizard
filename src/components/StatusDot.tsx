type Tone = "ok" | "bad" | "idle";

const TONE: Record<Tone, string> = {
  ok: "bg-ok",
  bad: "bg-bad",
  idle: "bg-ink-faint",
};

export function StatusDot({ tone, pulse = false }: { tone: Tone; pulse?: boolean }) {
  return (
    <span className="relative inline-flex h-2.5 w-2.5">
      {pulse && (
        <span
          className={`absolute inline-flex h-full w-full rounded-full opacity-60 animate-ping ${TONE[tone]}`}
        />
      )}
      <span className={`relative inline-flex h-2.5 w-2.5 rounded-full ${TONE[tone]}`} />
    </span>
  );
}

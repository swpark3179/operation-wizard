// Agent badge. Mirrors Open Design's AgentIcon fallback: a rounded square with
// the agent's initial. (A real monochrome SVG could be dropped in later.)

export function AgentIcon({ name, size = 36 }: { name: string; size?: number }) {
  const initial = name.trim().charAt(0).toUpperCase() || "?";
  return (
    <div
      className="grid place-items-center rounded-[10px] bg-accent-tint text-accent font-serif font-semibold select-none border border-accent-soft"
      style={{ width: size, height: size, fontSize: size * 0.5 }}
      aria-hidden
    >
      {initial}
    </div>
  );
}

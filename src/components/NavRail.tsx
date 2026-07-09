import { Home, Boxes, Workflow, Library } from "lucide-react";
import type { ComponentType } from "react";
import { useIngestState } from "../lib/ingest";

export type View = "home" | "agents" | "flows" | "knowledge";

const ITEMS: { id: View; label: string; icon: ComponentType<{ size?: number }> }[] = [
  { id: "home", label: "Home", icon: Home },
  { id: "agents", label: "Agents", icon: Boxes },
  { id: "flows", label: "Flows", icon: Workflow },
  { id: "knowledge", label: "지식", icon: Library },
];

export function NavRail({ view, onChange }: { view: View; onChange: (v: View) => void }) {
  // A running background Confluence ingest shows a pulse dot on 지식 (D51) —
  // the reminder that there is live progress to come back to.
  const ingesting = useIngestState().status === "running";
  return (
    <nav className="flex w-14 shrink-0 flex-col items-center gap-1 border-r border-line bg-panel py-3">
      {ITEMS.map(({ id, label, icon: Icon }) => {
        const active = view === id;
        return (
          <button
            key={id}
            type="button"
            title={id === "knowledge" && ingesting ? `${label} — 수집 진행 중` : label}
            aria-label={label}
            aria-current={active}
            onClick={() => onChange(id)}
            className={
              "relative grid h-10 w-10 place-items-center rounded-[8px] transition-colors duration-[120ms] " +
              (active
                ? "bg-accent-tint text-accent"
                : "text-ink-soft hover:bg-subtle hover:text-ink")
            }
          >
            <Icon size={19} />
            {id === "knowledge" && ingesting && (
              <span className="absolute right-1.5 top-1.5 h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
            )}
          </button>
        );
      })}
    </nav>
  );
}

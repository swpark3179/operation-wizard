import { Home, Boxes, Workflow } from "lucide-react";
import type { ComponentType } from "react";

export type View = "home" | "agents" | "flows";

const ITEMS: { id: View; label: string; icon: ComponentType<{ size?: number }> }[] = [
  { id: "home", label: "Home", icon: Home },
  { id: "agents", label: "Agents", icon: Boxes },
  { id: "flows", label: "Flows", icon: Workflow },
];

export function NavRail({ view, onChange }: { view: View; onChange: (v: View) => void }) {
  return (
    <nav className="flex w-14 shrink-0 flex-col items-center gap-1 border-r border-line bg-panel py-3">
      {ITEMS.map(({ id, label, icon: Icon }) => {
        const active = view === id;
        return (
          <button
            key={id}
            type="button"
            title={label}
            aria-label={label}
            aria-current={active}
            onClick={() => onChange(id)}
            className={
              "grid h-10 w-10 place-items-center rounded-[8px] transition-colors duration-[120ms] " +
              (active
                ? "bg-accent-tint text-accent"
                : "text-ink-soft hover:bg-subtle hover:text-ink")
            }
          >
            <Icon size={19} />
          </button>
        );
      })}
    </nav>
  );
}

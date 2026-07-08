import type { ReactNode } from "react";
import { TopBar } from "./TopBar";
import { NavRail, type View } from "./NavRail";

export function AppShell({
  view,
  onViewChange,
  children,
}: {
  view: View;
  onViewChange: (v: View) => void;
  children: ReactNode;
}) {
  return (
    <div className="flex h-full flex-col bg-app text-ink">
      <TopBar />
      <div className="flex min-h-0 flex-1">
        <NavRail view={view} onChange={onViewChange} />
        {/* overflow-hidden so a full-height two-pane workspace can own its own
            scrolling; simpler views wrap themselves in an overflow-auto box. */}
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">{children}</main>
      </div>
    </div>
  );
}

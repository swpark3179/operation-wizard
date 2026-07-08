export function TopBar() {
  return (
    <header className="flex h-11 shrink-0 items-center gap-2.5 border-b border-line bg-panel px-3.5">
      <div className="grid h-6 w-6 place-items-center rounded-[7px] bg-accent text-white font-serif text-[13px] font-bold">
        O
      </div>
      <span className="font-serif text-[15px] font-semibold tracking-[-0.01em] text-ink-strong">
        Operation Wizard
      </span>
      <span className="ml-1 rounded-full bg-subtle px-2 py-0.5 text-[11px] font-medium text-ink-soft">
        시스템 운영 워크스페이스
      </span>
    </header>
  );
}

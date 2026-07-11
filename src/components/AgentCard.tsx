import { RefreshCw, ChevronDown, FolderOpen, SlidersHorizontal } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { AgentIcon } from "./AgentIcon";
import { StatusDot } from "./StatusDot";
import { DIAGNOSTIC_HINT, type AgentInfo, type DetectedAgent, type Settings } from "../lib/types";

function SourceBadge({ source }: { source: DetectedAgent["source"] }) {
  if (source === "custom-path")
    return <Pill className="bg-warn-bg text-warn">via custom path</Pill>;
  if (source === "path") return <Pill className="bg-subtle text-ink-muted">on PATH</Pill>;
  return null;
}

function Pill({ children, className }: { children: ReactNode; className: string }) {
  return (
    <span className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${className}`}>
      {children}
    </span>
  );
}

/** Collapsible custom-binary-path editor (formerly the standalone Settings view).
 * Overrides an agent's executable path when auto-detection cannot find it; saving
 * re-detects that agent. Equivalent to the agent's `*_BIN` env override. */
function PathConfig({
  info,
  initialValue,
  onSave,
}: {
  info: AgentInfo;
  initialValue: string;
  onSave: (agentId: string, path: string | null) => Promise<void>;
}) {
  const [openPanel, setOpenPanel] = useState(false);
  const [value, setValue] = useState(initialValue);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  // Sync the field when settings load/refresh.
  useEffect(() => {
    setValue(initialValue);
  }, [initialValue]);

  async function browse() {
    const picked = await open({
      multiple: false,
      directory: false,
      title: `Select the ${info.name} executable`,
      filters: [{ name: "Executable", extensions: ["exe", "cmd", "bat"] }],
    });
    if (typeof picked === "string") setValue(picked);
  }

  async function save(next: string | null) {
    setSaving(true);
    setSaved(false);
    try {
      await onSave(info.id, next);
      setSaved(true);
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="mt-3 border-t border-line pt-3">
      <button
        type="button"
        onClick={() => setOpenPanel((s) => !s)}
        className="flex items-center gap-1.5 text-[13px] font-medium text-ink-muted hover:text-ink"
      >
        <ChevronDown
          size={15}
          className={`transition-transform duration-[120ms] ${openPanel ? "rotate-0" : "-rotate-90"}`}
        />
        <SlidersHorizontal size={14} />
        Custom path
        {initialValue && <Pill className="bg-warn-bg text-warn">set</Pill>}
      </button>

      {openPanel && (
        <div className="mt-2.5">
          <p className="mb-2 text-[12.5px] text-ink-muted">
            Override this agent's binary path when auto-detection cannot find it
            {info.envVar ? (
              <>
                {" "}
                — equivalent to the <code className="font-mono">{info.envVar}</code> override
              </>
            ) : null}
            . Leave empty to auto-detect via PATH and toolchain directories.
          </p>

          <div className="flex gap-2">
            <input
              type="text"
              value={value}
              onChange={(e) => {
                setValue(e.currentTarget.value);
                setSaved(false);
              }}
              placeholder={`Path to the ${info.name} executable`}
              spellCheck={false}
              className="min-w-0 flex-1 rounded-[6px] border border-line bg-elevated px-2.5 py-1.5 font-mono text-[12.5px] text-ink outline-none focus:border-accent"
            />
            <button
              type="button"
              onClick={browse}
              className="flex shrink-0 items-center gap-1.5 rounded-[6px] border border-line px-2.5 py-1.5 text-[13px] font-medium text-ink transition-colors duration-[120ms] hover:bg-subtle"
            >
              <FolderOpen size={14} />
              Browse…
            </button>
          </div>

          <div className="mt-2.5 flex items-center gap-2">
            <button
              type="button"
              onClick={() => save(value.trim() === "" ? null : value)}
              disabled={saving}
              className="rounded-[6px] bg-accent px-3.5 py-1.5 text-[13px] font-medium text-white transition-colors duration-[120ms] hover:bg-accent-strong disabled:opacity-50"
            >
              {saving ? "Saving…" : "Save & detect"}
            </button>
            <button
              type="button"
              onClick={async () => {
                // Confirm only when a saved override is about to be removed (D57).
                if (initialValue) {
                  const ok = await ask(
                    `${info.name}의 저장된 사용자 지정 경로를 삭제하고 자동 탐지로 되돌릴까요?`,
                    { title: info.name, kind: "warning" },
                  );
                  if (!ok) return;
                }
                setValue("");
                void save(null);
              }}
              disabled={saving}
              className="rounded-[6px] border border-line px-3 py-1.5 text-[13px] font-medium text-ink-muted transition-colors duration-[120ms] hover:bg-subtle disabled:opacity-50"
            >
              Clear
            </button>
            {saved && <span className="text-[12.5px] text-ok">Saved.</span>}
          </div>
        </div>
      )}
    </div>
  );
}

export function AgentCard({
  info,
  agent,
  loading,
  error,
  onRefresh,
  settings,
  onSave,
}: {
  info: AgentInfo;
  agent: DetectedAgent | null;
  loading: boolean;
  error: string | null;
  onRefresh: () => void;
  /** App settings (for this agent's saved custom binary path). */
  settings: Settings | null;
  /** Save/clear the custom binary path, then re-detect this agent. */
  onSave: (agentId: string, path: string | null) => Promise<void>;
}) {
  const [showModels, setShowModels] = useState(false);
  const available = agent?.available ?? false;
  const tone = loading ? "idle" : available ? "ok" : "bad";
  const statusLabel = loading
    ? "Detecting…"
    : available
      ? "Detected"
      : "Not detected";

  return (
    <div className="rounded-[12px] border border-line bg-panel p-4 shadow-[var(--shadow-sm)]">
      <div className="flex items-start gap-3.5">
        <AgentIcon name={info.name} />

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="font-serif text-[16px] font-semibold text-ink-strong">{info.name}</h3>
            {agent && <SourceBadge source={agent.source} />}
          </div>

          <div className="mt-1 flex items-center gap-2 text-[13px]">
            <StatusDot tone={tone} pulse={loading} />
            <span
              className={
                available ? "font-medium text-ok" : loading ? "text-ink-muted" : "font-medium text-bad"
              }
            >
              {statusLabel}
            </span>
            {agent?.version && (
              <span className="rounded-[6px] bg-subtle px-1.5 py-0.5 font-mono text-[12px] text-ink-muted">
                {agent.version}
              </span>
            )}
          </div>
        </div>

        <button
          type="button"
          onClick={onRefresh}
          disabled={loading}
          className="flex items-center gap-1.5 rounded-[6px] border border-accent px-2.5 py-1.5 text-[13px] font-medium text-accent transition-colors duration-[120ms] hover:bg-accent-tint disabled:opacity-50"
        >
          <RefreshCw size={14} className={loading ? "animate-spin" : ""} />
          Refresh
        </button>
      </div>

      {/* Resolved path */}
      {agent?.path && (
        <div className="mt-3 break-all rounded-[8px] bg-subtle px-3 py-2 font-mono text-[12px] text-ink-muted">
          {agent.path}
        </div>
      )}

      {/* Diagnostic hint when unavailable */}
      {agent && !available && agent.diagnostic && (
        <p className="mt-3 rounded-[8px] bg-bad-bg px-3 py-2 text-[12.5px] text-bad">
          {DIAGNOSTIC_HINT[agent.diagnostic] ?? agent.diagnostic}
        </p>
      )}

      {error && (
        <p className="mt-3 rounded-[8px] bg-bad-bg px-3 py-2 text-[12.5px] text-bad">{error}</p>
      )}

      {/* Models */}
      {available && agent && (
        <div className="mt-3">
          <button
            type="button"
            onClick={() => setShowModels((s) => !s)}
            className="flex items-center gap-1.5 text-[13px] font-medium text-ink-muted hover:text-ink"
          >
            <ChevronDown
              size={15}
              className={`transition-transform duration-[120ms] ${showModels ? "rotate-0" : "-rotate-90"}`}
            />
            {agent.models.length} model{agent.models.length === 1 ? "" : "s"}
            <span className="rounded-full bg-subtle px-1.5 py-0.5 text-[11px] text-ink-soft">
              {agent.modelsSource === "live" ? "live" : "fallback"}
            </span>
          </button>
          {showModels && (
            <ul className="mt-2 flex flex-col gap-0.5">
              {agent.models.map((m) => (
                <li
                  key={m.id}
                  className="rounded-[6px] px-2 py-1 font-mono text-[12px] text-ink-muted hover:bg-subtle"
                >
                  {m.label}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {/* Custom binary path (formerly the standalone Settings view) */}
      <PathConfig
        info={info}
        initialValue={settings?.agents?.[info.id]?.customBin ?? ""}
        onSave={onSave}
      />
    </div>
  );
}

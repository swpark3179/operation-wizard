import { AgentCard } from "./AgentCard";
import type { AgentInfo, DetectedAgent, Settings } from "../lib/types";

export function AgentsView({
  agents,
  detected,
  loading,
  errors,
  onRefresh,
  settings,
  onSave,
}: {
  agents: AgentInfo[];
  detected: Record<string, DetectedAgent>;
  loading: Record<string, boolean>;
  errors: Record<string, string>;
  onRefresh: (id: string) => void;
  /** App settings (per-agent custom binary paths). */
  settings: Settings | null;
  /** Save/clear an agent's custom binary path, then re-detect it. */
  onSave: (agentId: string, path: string | null) => Promise<void>;
}) {
  return (
    <div className="mx-auto max-w-[680px] px-6 py-7">
      <div className="mb-4">
        <h2 className="font-serif text-[22px] font-semibold tracking-[-0.02em] text-ink-strong">
          Agents
        </h2>
        <p className="mt-0.5 text-[13px] text-ink-muted">
          Local CLI coding agents detected on this machine. Expand a card's custom
          path to override its binary location.
        </p>
      </div>

      <div className="flex flex-col gap-3">
        {agents.map((info) => (
          <AgentCard
            key={info.id}
            info={info}
            agent={detected[info.id] ?? null}
            loading={loading[info.id] ?? false}
            error={errors[info.id] ?? null}
            onRefresh={() => onRefresh(info.id)}
            settings={settings}
            onSave={onSave}
          />
        ))}
      </div>
    </div>
  );
}

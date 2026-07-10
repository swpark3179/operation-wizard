import { useCallback, useEffect, useState } from "react";
import { AlertTriangle, RefreshCw, X } from "lucide-react";
import { AppShell } from "./components/AppShell";
import type { View } from "./components/NavRail";
import { HomeArea } from "./components/HomeArea";
import { AgentsView } from "./components/AgentsView";
import { FlowSettingsView } from "./components/FlowSettingsView";
import { KnowledgeView } from "./components/KnowledgeView";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { detectAgent, getSettings, listAgents, setAgentBin } from "./lib/api";
import type { AgentInfo, DetectedAgent, Settings } from "./lib/types";

function App() {
  const [view, setView] = useState<View>("home");
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [detected, setDetected] = useState<Record<string, DetectedAgent>>({});
  const [loading, setLoading] = useState<Record<string, boolean>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [settings, setSettings] = useState<Settings | null>(null);
  // Bumped whenever the Home nav item is clicked, so HomeArea resets to its
  // launcher (mirrors the design's `goHome`).
  const [homeNonce, setHomeNonce] = useState(0);
  // Initial-load failure surface (D56): message for the banner + a nonce that
  // re-runs the whole boot effect on "다시 시도".
  const [bootError, setBootError] = useState<string | null>(null);
  const [bootNonce, setBootNonce] = useState(0);

  // Detect a single agent and merge its result/loading/error into the maps.
  const detectOne = useCallback(async (id: string) => {
    setLoading((m) => ({ ...m, [id]: true }));
    setErrors((m) => {
      if (!(id in m)) return m;
      const rest = { ...m };
      delete rest[id];
      return rest;
    });
    try {
      const agent = await detectAgent(id);
      setDetected((m) => ({ ...m, [id]: agent }));
    } catch (e) {
      setErrors((m) => ({ ...m, [id]: String(e) }));
    } finally {
      setLoading((m) => ({ ...m, [id]: false }));
    }
  }, []);

  // Initial load: settings + registry, then detect every agent in parallel.
  // Non-blocking (the shell renders regardless), but failures surface in the
  // boot banner instead of being silently swallowed.
  useEffect(() => {
    let stale = false;
    Promise.allSettled([getSettings(), listAgents()]).then(([s, a]) => {
      if (stale) return;
      const failures: string[] = [];
      if (s.status === "fulfilled") setSettings(s.value);
      else failures.push(`설정 불러오기 실패: ${s.reason}`);
      if (a.status === "fulfilled") {
        setAgents(a.value);
        a.value.forEach((x) => detectOne(x.id));
      } else {
        failures.push(`에이전트 목록 불러오기 실패: ${a.reason}`);
      }
      setBootError(failures.length ? failures.join(" · ") : null);
    });
    return () => {
      stale = true;
    };
  }, [detectOne, bootNonce]);

  const handleSave = useCallback(
    async (agentId: string, path: string | null) => {
      const next = await setAgentBin(agentId, path);
      setSettings(next);
      await detectOne(agentId);
    },
    [detectOne],
  );

  const handleViewChange = useCallback((v: View) => {
    if (v === "home") setHomeNonce((n) => n + 1);
    setView(v);
  }, []);

  return (
    <AppShell view={view} onViewChange={handleViewChange}>
      {/* keyed per view: a crash in one view keeps the shell alive, and
          navigating elsewhere remounts the boundary (= automatic recovery). */}
      <ErrorBoundary key={view}>
        {view === "home" ? (
          <HomeArea
            resetNonce={homeNonce}
            agents={agents}
            detected={detected}
            settings={settings}
          />
        ) : view === "agents" ? (
          <div className="h-full overflow-auto">
            <AgentsView
              agents={agents}
              detected={detected}
              loading={loading}
              errors={errors}
              onRefresh={detectOne}
              settings={settings}
              onSave={handleSave}
            />
          </div>
        ) : view === "flows" ? (
          <div className="h-full overflow-auto">
            <FlowSettingsView settings={settings} onSettingsChange={setSettings} />
          </div>
        ) : (
          <div className="h-full overflow-auto">
            <KnowledgeView settings={settings} onSettingsChange={setSettings} />
          </div>
        )}
      </ErrorBoundary>
      {bootError && (
        <div className="fixed left-1/2 top-12 z-50 flex max-w-[min(560px,calc(100vw-2rem))] -translate-x-1/2 items-center gap-2 rounded-lg border border-bad-border bg-bad-bg px-3 py-2 text-[12.5px] text-bad shadow-lg">
          <AlertTriangle size={14} className="shrink-0" />
          <span className="min-w-0 truncate" title={bootError}>
            {bootError}
          </span>
          <button
            type="button"
            onClick={() => setBootNonce((n) => n + 1)}
            className="flex shrink-0 items-center gap-1 rounded-[6px] border border-bad-border bg-panel px-2 py-0.5 font-medium transition-colors duration-[120ms] hover:bg-subtle"
          >
            <RefreshCw size={12} />
            다시 시도
          </button>
          <button
            type="button"
            onClick={() => setBootError(null)}
            aria-label="닫기"
            className="grid shrink-0 place-items-center rounded-[6px] p-0.5 transition-opacity hover:opacity-70"
          >
            <X size={14} />
          </button>
        </div>
      )}
    </AppShell>
  );
}

export default App;

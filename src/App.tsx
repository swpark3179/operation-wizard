import { useCallback, useEffect, useState } from "react";
import { AppShell } from "./components/AppShell";
import type { View } from "./components/NavRail";
import { HomeArea } from "./components/HomeArea";
import { AgentsView } from "./components/AgentsView";
import { FlowSettingsView } from "./components/FlowSettingsView";
import { KnowledgeView } from "./components/KnowledgeView";
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
  useEffect(() => {
    getSettings().then(setSettings).catch(() => {});
    listAgents()
      .then((list) => {
        setAgents(list);
        list.forEach((a) => detectOne(a.id));
      })
      .catch(() => {});
  }, [detectOne]);

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
    </AppShell>
  );
}

export default App;

import { useEffect, useState } from "react";
import { HomeView } from "./HomeView";
import { WorkspaceView } from "./WorkspaceView";
import type { Category } from "./workspace";
import type { AgentInfo, DetectedAgent, Settings, StoredSession } from "../lib/types";

/**
 * Owns the Home → Workspace transition (the design's `screen` state) and, for a
 * chat, the **projectId** (minted for a fresh chat, adopted when opening a
 * recent project). The Home nav item resets this via `resetNonce`.
 */
export function HomeArea({
  resetNonce,
  agents,
  detected,
  settings,
  onOpenAgents,
  onBusyChange,
}: {
  resetNonce: number;
  agents: AgentInfo[];
  detected: Record<string, DetectedAgent>;
  /** App settings (user workflows/skills), threaded down to the workspace. */
  settings: Settings | null;
  /** Navigate to the Agents view (undetected-agent onboarding, D57). */
  onOpenAgents?: () => void;
  /** Workspace busy (streaming) mirror for app-level navigation guards (D57). */
  onBusyChange?: (busy: boolean) => void;
}) {
  const [screen, setScreen] = useState<"home" | "workspace">("home");
  const [category, setCategory] = useState<Category | null>(null);
  const [seedPrompt, setSeedPrompt] = useState("");
  // A saved session opened from the Home "recent" list (null = fresh chat).
  const [loadedSession, setLoadedSession] = useState<StoredSession | null>(null);
  // The active project: a fresh chat mints a new id; opening a recent project
  // adopts its id + resolved workdir.
  const [projectId, setProjectId] = useState<string | null>(null);
  const [initialWorkdir, setInitialWorkdir] = useState<string | null>(null);
  // The reopened project's stored codebase path (fresh chats pick one in the
  // requirements form instead — D45).
  const [initialCodebasePath, setInitialCodebasePath] = useState<string | null>(null);
  // The agent/model picked in the Home composer (fresh chats only — a loaded
  // session carries its own; D60).
  const [initialAgentId, setInitialAgentId] = useState<string | null>(null);
  const [initialModel, setInitialModel] = useState<string | null>(null);

  // Reset to the launcher whenever Home is (re)selected in the nav rail.
  useEffect(() => {
    setScreen("home");
    setCategory(null);
    setSeedPrompt("");
    setLoadedSession(null);
    setProjectId(null);
    setInitialWorkdir(null);
    setInitialCodebasePath(null);
    setInitialAgentId(null);
    setInitialModel(null);
  }, [resetNonce]);

  if (screen === "workspace" && category && projectId) {
    return (
      <WorkspaceView
        projectId={projectId}
        initialWorkdir={initialWorkdir}
        initialCodebasePath={initialCodebasePath}
        initialAgentId={initialAgentId}
        initialModel={initialModel}
        category={category}
        seedPrompt={seedPrompt}
        agents={agents}
        detected={detected}
        settings={settings}
        initialSession={loadedSession}
        onOpenAgents={onOpenAgents}
        onBusyChange={onBusyChange}
        onHome={() => {
          setScreen("home");
          setCategory(null);
          setSeedPrompt("");
          setLoadedSession(null);
          setProjectId(null);
          setInitialWorkdir(null);
          setInitialCodebasePath(null);
        }}
      />
    );
  }

  return (
    <HomeView
      agents={agents}
      detected={detected}
      onOpenAgents={onOpenAgents}
      // New chat / category → a brand-new project. `workdir` is the folder the
      // user optionally picked on Home; null → auto (own workspace/ subfolder).
      // `agentId`/`model` seed the ChatPanel composer (D60).
      onStart={(cat, prompt, workdir, agentId, model) => {
        setProjectId(crypto.randomUUID());
        setInitialWorkdir(workdir ?? null);
        setInitialCodebasePath(null);
        setInitialAgentId(agentId ?? null);
        setInitialModel(model ?? null);
        setLoadedSession(null);
        setCategory(cat);
        setSeedPrompt(prompt);
        setScreen("workspace");
      }}
      // Recent project → adopt its id + resolved workdir/codebase, open the
      // given session (which carries its own agent/model).
      onOpenSession={(session, id, projectWorkdir, codebasePath) => {
        setProjectId(id);
        setInitialWorkdir(projectWorkdir);
        setInitialCodebasePath(codebasePath ?? null);
        setInitialAgentId(null);
        setInitialModel(null);
        setLoadedSession(session);
        setCategory((session.category as Category) ?? "plan");
        setSeedPrompt("");
        setScreen("workspace");
      }}
    />
  );
}

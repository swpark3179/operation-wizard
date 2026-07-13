import { RefreshCw, ChevronDown, Plug } from "lucide-react";
import { useEffect, useState } from "react";
import { AgentIcon } from "./AgentIcon";
import { StatusDot } from "./StatusDot";
import { probeFabrix } from "../lib/api";
import {
  DIAGNOSTIC_HINT,
  type AgentInfo,
  type DetectedAgent,
  type FabrixConfig,
  type Settings,
} from "../lib/types";

const inputCls =
  "w-full rounded-[6px] border border-line bg-elevated px-2.5 py-1.5 text-[13px] text-ink outline-none focus:border-accent disabled:opacity-60";
const labelCls = "mb-1 block text-[11.5px] font-medium text-ink-soft";
const primaryBtn =
  "rounded-[6px] bg-accent px-3.5 py-1.5 text-[13px] font-medium text-white transition-colors hover:bg-accent-strong disabled:opacity-50";
const ghostBtn =
  "inline-flex items-center gap-1.5 rounded-[6px] border border-line px-2.5 py-1.5 text-[12.5px] font-medium text-ink-muted transition-colors hover:bg-subtle disabled:opacity-40";

/** Card for the Fabrix remote HTTP agent (D64). Unlike {@link AgentCard} (a
 * single custom-binary path), Fabrix needs an endpoint + two request headers,
 * so this renders a credentials form + a connection test alongside the usual
 * detection status and model list. Detection ("Detected"/"Not detected") here
 * means "configured + endpoint reachable" — populated by `detect_agent`'s
 * remote branch, which fetches the live model list over HTTP. */
export function FabrixCard({
  info,
  agent,
  loading,
  error,
  onRefresh,
  settings,
  onSaveFabrix,
}: {
  info: AgentInfo;
  agent: DetectedAgent | null;
  loading: boolean;
  error: string | null;
  onRefresh: () => void;
  settings: Settings | null;
  /** Save/clear the Fabrix connection config, then re-detect. */
  onSaveFabrix: (config: FabrixConfig | null) => Promise<void>;
}) {
  const cfg = settings?.fabrix;
  const [endpoint, setEndpoint] = useState(cfg?.endpointUrl ?? "");
  const [client, setClient] = useState(cfg?.client ?? "");
  const [token, setToken] = useState(cfg?.openapiToken ?? "");
  const [allowInvalid, setAllowInvalid] = useState(cfg?.allowInvalidCerts ?? false);
  const [showModels, setShowModels] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [probe, setProbe] = useState<{ ok: boolean; msg: string } | null>(null);

  // Re-sync local fields when settings reload (e.g. after a save returns the
  // normalized config, or another view changes it) — mirrors PathConfig.
  useEffect(() => {
    setEndpoint(cfg?.endpointUrl ?? "");
    setClient(cfg?.client ?? "");
    setToken(cfg?.openapiToken ?? "");
    setAllowInvalid(cfg?.allowInvalidCerts ?? false);
    setSaved(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings]);

  const available = agent?.available ?? false;
  const tone = loading ? "idle" : available ? "ok" : "bad";
  const statusLabel = loading ? "Detecting…" : available ? "Detected" : "Not detected";

  const save = async () => {
    setSaving(true);
    setSaveError(null);
    setProbe(null);
    try {
      const next: FabrixConfig | null = endpoint.trim()
        ? {
            endpointUrl: endpoint.trim(),
            client: client.trim() || null,
            openapiToken: token.trim() || null,
            allowInvalidCerts: allowInvalid,
          }
        : null;
      await onSaveFabrix(next);
      setSaved(true);
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const test = async () => {
    setProbe(null);
    try {
      const msg = await probeFabrix();
      setProbe({ ok: true, msg });
    } catch (e) {
      setProbe({ ok: false, msg: String(e) });
    }
  };

  return (
    <div className="rounded-[12px] border border-line bg-panel p-4 shadow-[var(--shadow-sm)]">
      <div className="flex items-start gap-3.5">
        <AgentIcon name={info.name} />

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="font-serif text-[16px] font-semibold text-ink-strong">{info.name}</h3>
            <span className="rounded-full bg-subtle px-2 py-0.5 text-[11px] font-medium text-ink-muted">
              원격 HTTP API
            </span>
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

      {/* Diagnostic hint when unavailable (not-configured / unreachable) */}
      {agent && !available && agent.diagnostic && (
        <p className="mt-3 rounded-[8px] bg-bad-bg px-3 py-2 text-[12.5px] text-bad">
          {DIAGNOSTIC_HINT[agent.diagnostic] ?? agent.diagnostic}
        </p>
      )}

      {error && (
        <p className="mt-3 rounded-[8px] bg-bad-bg px-3 py-2 text-[12.5px] text-bad">{error}</p>
      )}

      {/* Models (fetched live over HTTP when configured) */}
      {available && agent && agent.models.length > 0 && (
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
                  className="rounded-[6px] px-2 py-1 text-[12px] text-ink-muted hover:bg-subtle"
                >
                  {m.label}
                  <span className="ml-2 font-mono text-[11px] text-ink-soft">{m.id}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {/* Connection settings */}
      <div className="mt-4 border-t border-line pt-3.5">
        <p className="mb-3 text-[12px] leading-[1.5] text-ink-muted">
          Fabrix 서버 연결 정보입니다. 토큰은 <span className="font-mono">x-fabrix-client</span>·
          <span className="font-mono">x-openapi-token</span> 요청 헤더로 전달되며 settings.json에
          평문으로 저장되니 읽기 전용 키를 권장합니다.
        </p>
        <div className="mb-2.5 flex flex-col gap-2.5">
          <div>
            <label className={labelCls}>ENDPOINT_URL</label>
            <input
              value={endpoint}
              onChange={(e) => {
                setEndpoint(e.target.value);
                setSaved(false);
              }}
              disabled={saving}
              placeholder="https://fabrix.example.com"
              className={inputCls + " font-mono text-[12px]"}
            />
          </div>
          <div className="grid grid-cols-2 gap-2.5">
            <div>
              <label className={labelCls}>x-fabrix-client</label>
              <input
                type="password"
                value={client}
                onChange={(e) => {
                  setClient(e.target.value);
                  setSaved(false);
                }}
                disabled={saving}
                placeholder="클라이언트 키"
                className={inputCls + " font-mono text-[12px]"}
              />
            </div>
            <div>
              <label className={labelCls}>x-openapi-token</label>
              <input
                type="password"
                value={token}
                onChange={(e) => {
                  setToken(e.target.value);
                  setSaved(false);
                }}
                disabled={saving}
                placeholder="OpenAPI 토큰"
                className={inputCls + " font-mono text-[12px]"}
              />
            </div>
          </div>
          <label className="flex items-center gap-2 text-[12px] text-ink-muted">
            <input
              type="checkbox"
              checked={allowInvalid}
              onChange={(e) => {
                setAllowInvalid(e.target.checked);
                setSaved(false);
              }}
              disabled={saving}
            />
            인증서 검증 건너뛰기 (사내 TLS 검사 프록시용 — 위험, 가능하면 사용 안 함)
          </label>
        </div>
        <div className="flex items-center gap-2">
          <button type="button" onClick={() => void test()} disabled={!settings?.fabrix} className={ghostBtn}>
            <Plug size={13} /> 연결 테스트
          </button>
          {probe && (
            <span
              className={"min-w-0 truncate text-[12px] " + (probe.ok ? "text-ok" : "text-bad")}
              title={probe.msg}
            >
              {probe.msg}
            </span>
          )}
          <div className="flex-1" />
          {saved && <span className="text-[12.5px] text-ok">Saved.</span>}
          {saveError && (
            <span className="max-w-[280px] truncate text-[12px] text-bad" title={saveError}>
              {saveError}
            </span>
          )}
          <button type="button" onClick={() => void save()} disabled={saving} className={primaryBtn}>
            {saving ? "저장 중…" : "저장"}
          </button>
        </div>
      </div>
    </div>
  );
}

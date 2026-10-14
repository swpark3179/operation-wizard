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
const badgeCls = "rounded-full bg-subtle px-1.5 py-0.5 text-[10.5px] text-ink-soft";

/** Default model when none is configured — 70 = Glm 5.2 (analysis + tool
 * calling; mirrors `fabrix::DEFAULT_MODEL_ID`). */
const DEFAULT_MODEL_ID = "70";
/** Licence quota when the card leaves the field blank (D91 — mirrors
 * `fabrix_rate::DEFAULT_CALLS_PER_MINUTE`). */
const DEFAULT_CALLS_PER_MINUTE = 10;

/** The saved quota as input text (absent → blank = default). */
function quotaText(v: number | null | undefined): string {
  return v === null || v === undefined ? "" : String(v);
}

/** Card for the FabriX remote HTTP agent — the FabriX LLM Serving API
 * (OpenAI-compatible, D64/D84). Unlike {@link AgentCard} (a single custom
 * binary path), FabriX needs an endpoint + two auth headers and a default model
 * (`x-llm-model-id`), so this renders a credentials form + a connection test
 * alongside the detection status and model list. The model list comes from
 * `DetectedAgent.models` (always fresh after a detect), with the FabriX
 * `context`/`tools` metadata as badges; the backend lists the default first. */
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
  /** Save/clear the FabriX connection config, then re-detect. */
  onSaveFabrix: (config: FabrixConfig | null) => Promise<void>;
}) {
  const cfg = settings?.fabrix;
  const [endpoint, setEndpoint] = useState(cfg?.endpointUrl ?? "");
  const [client, setClient] = useState(cfg?.client ?? "");
  const [token, setToken] = useState(cfg?.openapiToken ?? "");
  const [defaultModel, setDefaultModel] = useState(cfg?.defaultModelId ?? DEFAULT_MODEL_ID);
  const [callsPerMinute, setCallsPerMinute] = useState(quotaText(cfg?.callsPerMinute));
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
    setDefaultModel(cfg?.defaultModelId ?? DEFAULT_MODEL_ID);
    setCallsPerMinute(quotaText(cfg?.callsPerMinute));
    setSaved(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings]);

  const available = agent?.available ?? false;
  const tone = loading ? "idle" : available ? "ok" : "bad";
  const statusLabel = loading ? "Detecting…" : available ? "Detected" : "Not detected";
  const models = agent?.models ?? [];
  const savedDefault = cfg?.defaultModelId ?? DEFAULT_MODEL_ID;

  const save = async () => {
    setSaving(true);
    setSaveError(null);
    setProbe(null);
    const quota = callsPerMinute.trim();
    if (quota && !/^\d+$/.test(quota)) {
      setSaveError("분당 호출 한도는 0 이상의 정수로 입력하세요 (0 = 제한 없음).");
      setSaving(false);
      return;
    }
    try {
      const next: FabrixConfig | null = endpoint.trim()
        ? {
            endpointUrl: endpoint.trim(),
            client: client.trim() || null,
            openapiToken: token.trim() || null,
            // TLS is always verified (D84) — the legacy opt-out stays off.
            allowInvalidCerts: false,
            defaultModelId: defaultModel.trim() || null,
            // Blank → the default licence quota (D91); the backend caps it.
            callsPerMinute: quota ? Number(quota) : null,
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
              LLM Serving API
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

      {/* Models (fetched over HTTP and cached; default listed first) */}
      {models.length > 0 && (
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
            {models.length} model{models.length === 1 ? "" : "s"}
            <span className={badgeCls}>{agent?.modelsSource === "live" ? "live" : "cached"}</span>
          </button>
          {showModels && (
            <ul className="mt-2 flex flex-col gap-0.5">
              {models.map((m) => (
                <li
                  key={m.id}
                  className="flex items-center gap-1.5 rounded-[6px] px-2 py-1 text-[12px] text-ink-muted hover:bg-subtle"
                >
                  <span className="text-ink">{m.label}</span>
                  <span className="font-mono text-[11px] text-ink-soft">id {m.id}</span>
                  {m.context && <span className={badgeCls}>{m.context}</span>}
                  {m.tools && <span className={badgeCls}>도구 지원</span>}
                  {m.id === savedDefault && (
                    <span className="rounded-full bg-accent-tint px-1.5 py-0.5 text-[10.5px] font-medium text-accent">
                      기본
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {/* Connection settings */}
      <div className="mt-4 border-t border-line pt-3.5">
        <p className="mb-3 text-[12px] leading-[1.5] text-ink-muted">
          FabriX 포털에서 'LLM Serving APIs'로 발급받은 연결 정보입니다. 엔드포인트에는 발급받은 URL만
          넣으면 앱이 <span className="font-mono">/openapi/llm</span>을 붙이고, 토큰의{" "}
          <span className="font-mono">Bearer</span> 접두사도 자동으로 붙입니다. 값은{" "}
          <span className="font-mono">x-fabrix-client</span>·<span className="font-mono">x-openapi-token</span>{" "}
          헤더로 전달되며 settings.json에 평문으로 저장됩니다. 대화 중에는 작업 폴더·코드베이스를 읽기
          전용 도구로 조회합니다(파일 수정 없음).
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
              placeholder="https://nsds-api.fabrix-s.samsungsds.com/sds/trial/api-llm"
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
                placeholder="클라이언트 키 (JWT)"
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
                placeholder="OpenAPI 토큰 (Bearer 생략 가능)"
                className={inputCls + " font-mono text-[12px]"}
              />
            </div>
          </div>
          <div>
            <label className={labelCls}>기본 모델 (x-llm-model-id)</label>
            {models.length > 0 ? (
              <select
                value={defaultModel}
                onChange={(e) => {
                  setDefaultModel(e.target.value);
                  setSaved(false);
                }}
                disabled={saving}
                className={inputCls}
              >
                {!models.some((m) => m.id === defaultModel) && (
                  <option value={defaultModel}>id {defaultModel} (목록에 없음)</option>
                )}
                {models.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.label} · id {m.id}
                    {m.context ? ` · ${m.context}` : ""}
                    {m.tools === false ? " · 도구 미지원" : ""}
                  </option>
                ))}
              </select>
            ) : (
              <input
                value={defaultModel}
                onChange={(e) => {
                  setDefaultModel(e.target.value);
                  setSaved(false);
                }}
                disabled={saving}
                placeholder={DEFAULT_MODEL_ID}
                className={inputCls + " font-mono text-[12px]"}
              />
            )}
            <p className="mt-1 text-[11.5px] text-ink-soft">
              채팅의 기본 선택 모델입니다. 70(Glm 5.2)은 분석·도구 활용, 16(gpt-oss-120b)은 빠르고 저렴한
              간단 질의에 권장됩니다. 도구 미지원 모델은 파일을 읽지 못합니다.
            </p>
          </div>
          <div>
            <label className={labelCls}>분당 호출 한도 (라이선스)</label>
            <div className="w-[120px]">
              <input
                value={callsPerMinute}
                onChange={(e) => {
                  setCallsPerMinute(e.target.value);
                  setSaved(false);
                }}
                disabled={saving}
                inputMode="numeric"
                placeholder={String(DEFAULT_CALLS_PER_MINUTE)}
                className={inputCls + " font-mono text-[12px]"}
              />
            </div>
            <p className="mt-1 text-[11.5px] text-ink-soft">
              도구를 쓰는 대화는 조회 라운드마다 API를 1회 호출합니다. 앱이 이 한도에 맞춰 호출 간격을
              조절하고, 한도에 걸리면 실패 대신 잠시 기다렸다가 이어서 진행합니다. 비우면{" "}
              {DEFAULT_CALLS_PER_MINUTE}회, 0이면 제한 없음.
            </p>
          </div>
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

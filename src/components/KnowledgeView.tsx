// 지식 view (D48): everything about external/in-house knowledge in one place —
// ① the in-house RAG service endpoint (in-house documents such as Confluence are
// searched through it — the app's own Confluence collection was removed, D98),
// ② the knowledge base entries injected into the foundation phase's knowledge
// step. Separate from Flows (flow *definition* vs knowledge *content*
// management).

import { useEffect, useRef, useState } from "react";
import {
  BookOpen,
  ChevronDown,
  Plug,
  Plus,
  Trash2,
} from "lucide-react";
import { ask } from "@tauri-apps/plugin-dialog";
import {
  deleteKnowledge,
  getSettings,
  listKnowledge,
  probeRag,
  saveKnowledge,
  setRagConfig,
} from "../lib/api";
import { ragUserError } from "../lib/foundation";
import { categoryLabel, sessionTime, type Category } from "./workspace";
import { useAutoGrow } from "../lib/useAutoGrow";
import type { KnowledgeEntry, Settings } from "../lib/types";

const inputCls =
  "w-full rounded-[6px] border border-line bg-elevated px-2.5 py-1.5 text-[13px] text-ink outline-none focus:border-accent disabled:opacity-60";
const labelCls = "mb-1 block text-[11.5px] font-medium text-ink-soft";
const cardCls = "rounded-[12px] border border-line bg-panel p-4 shadow-[var(--shadow-sm)]";
const primaryBtn =
  "rounded-[6px] bg-accent px-3.5 py-1.5 text-[13px] font-medium text-white transition-colors hover:bg-accent-strong disabled:opacity-50";
const ghostBtn =
  "inline-flex items-center gap-1.5 rounded-[6px] border border-line px-2.5 py-1.5 text-[12.5px] font-medium text-ink-muted transition-colors hover:bg-subtle disabled:opacity-40";

/** ① RAG 검색 설정 — endpoint the rag workflow step queries (rag.rs adapter). */
function RagSection({
  settings,
  onSettingsChange,
}: {
  settings: Settings | null;
  onSettingsChange: (s: Settings) => void;
}) {
  const cfg = settings?.rag;
  const [endpoint, setEndpoint] = useState(cfg?.endpoint ?? "");
  const [secretKey, setSecretKey] = useState(cfg?.secretKey ?? "");
  const [passKey, setPassKey] = useState(cfg?.passKey ?? "");
  const [topK, setTopK] = useState(cfg?.topK != null ? String(cfg.topK) : "");
  const [assetId, setAssetId] = useState(cfg?.knowledgeAssetId ?? "");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [probe, setProbe] = useState<{ ok: boolean; msg: string } | null>(null);
  const [showModels, setShowModels] = useState(false);
  // Backend-owned model cache (filled by save / 연결 테스트, shown without a
  // network call — cache-first, D66). The RAG search itself uses a fixed model.
  const models = cfg?.models ?? [];

  // Re-seed inputs when settings reload (e.g. after a save elsewhere) — matches
  // FabrixCard; without it the form keeps stale values after onSettingsChange.
  useEffect(() => {
    setEndpoint(cfg?.endpoint ?? "");
    setSecretKey(cfg?.secretKey ?? "");
    setPassKey(cfg?.passKey ?? "");
    setTopK(cfg?.topK != null ? String(cfg.topK) : "");
    setAssetId(cfg?.knowledgeAssetId ?? "");
    setSaved(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings]);

  const save = async () => {
    setSaving(true);
    setError(null);
    setProbe(null);
    try {
      const k = parseInt(topK, 10);
      const next = await setRagConfig(
        endpoint.trim()
          ? {
              endpoint: endpoint.trim(),
              secretKey: secretKey.trim() || null,
              passKey: passKey.trim() || null,
              topK: Number.isFinite(k) && k > 0 ? k : null,
              knowledgeAssetId: assetId.trim() || null,
            }
          : null,
      );
      onSettingsChange(next);
      setSaved(true);
      // On save, fetch the model list (proxy-bypassed) and cache it (D66). The
      // fetch persists into settings.rag.models; reload to show it. A fetch
      // failure is surfaced but does not fail the save.
      if (endpoint.trim()) await refreshModels();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  // Run the connection test (fetches + caches the model list) then reload
  // settings so the freshly-cached list renders.
  const refreshModels = async () => {
    try {
      const msg = await probeRag();
      setProbe({ ok: true, msg });
      onSettingsChange(await getSettings());
    } catch (e) {
      // The rag.rs error message is rephrased for end users (D57).
      setProbe({ ok: false, msg: ragUserError(String(e)) });
    }
  };

  const test = async () => {
    setProbe(null);
    await refreshModels();
  };

  return (
    <div className={cardCls}>
      <div className="mb-1 font-serif text-[15px] font-semibold text-ink-strong">RAG 검색 설정</div>
      <p className="mb-3 text-[12px] leading-[1.5] text-ink-muted">
        기반 단계의 'RAG 검색'이 조회할 사내 RAG(Fabrix rag-chat) 서비스입니다. Confluence 등
        사내 문서는 이 지식 자산에서 검색되고, 결과의 출처 링크로 원문을 열 수 있습니다. 두 키는{" "}
        <span className="font-mono">x-fabrix-client</span>·
        <span className="font-mono">x-openapi-token</span> 요청 헤더로 전달되며 settings.json에 평문으로
        저장되니 읽기 전용 키를 권장합니다. 모델은 GLM 5.2를 사용하고, Knowledge Asset ID를 비워두면 샘플
        자산으로 조회합니다.
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
            placeholder="https://nsds-api.fabrix-s.samsungsds.com/sds/trial/api-rag-chat"
            className={inputCls + " font-mono text-[12px]"}
          />
        </div>
        <div className="grid grid-cols-2 gap-2.5">
          <div>
            <label className={labelCls}>x-fabrix-client</label>
            <input
              type="password"
              value={secretKey}
              onChange={(e) => {
                setSecretKey(e.target.value);
                setSaved(false);
              }}
              disabled={saving}
              placeholder="비워두면 미사용"
              className={inputCls + " font-mono text-[12px]"}
            />
          </div>
          <div>
            <label className={labelCls}>x-openapi-token</label>
            <input
              type="password"
              value={passKey}
              onChange={(e) => {
                setPassKey(e.target.value);
                setSaved(false);
              }}
              disabled={saving}
              placeholder="Bearer 포함 · 비워두면 미사용"
              className={inputCls + " font-mono text-[12px]"}
            />
          </div>
        </div>
        <div className="grid grid-cols-[1fr_96px] gap-2.5">
          <div>
            <label className={labelCls}>Knowledge Asset ID</label>
            <input
              value={assetId}
              onChange={(e) => {
                setAssetId(e.target.value);
                setSaved(false);
              }}
              disabled={saving}
              placeholder="비워두면 샘플 자산 사용 (019f5a11-…)"
              className={inputCls + " font-mono text-[12px]"}
            />
          </div>
          <div>
            <label className={labelCls}>Top K</label>
            <input
              value={topK}
              onChange={(e) => {
                setTopK(e.target.value);
                setSaved(false);
              }}
              disabled={saving}
              placeholder="5"
              className={inputCls + " font-mono text-[12px]"}
            />
          </div>
        </div>
      </div>
      {/* Cached model list (filled by save / 연결 테스트 — D66). Informational:
          the RAG search always uses the fixed GLM 5.2 model. */}
      {models.length > 0 && (
        <div className="mb-2.5">
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
            <span className="rounded-full bg-subtle px-1.5 py-0.5 text-[11px] text-ink-soft">cached</span>
          </button>
          {showModels && (
            <ul className="mt-2 flex flex-col gap-0.5">
              {models.map((m) => (
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
      <div className="flex items-center gap-2">
        <button type="button" onClick={() => void test()} disabled={!settings?.rag} className={ghostBtn}>
          <Plug size={13} /> 연결 테스트
        </button>
        {probe && (
          <span
            className={
              "min-w-0 truncate text-[12px] " + (probe.ok ? "text-ok" : "text-bad")
            }
            title={probe.msg}
          >
            {probe.msg}
          </span>
        )}
        <div className="flex-1" />
        {saved && <span className="text-[12.5px] text-ok">Saved.</span>}
        {error && (
          <span className="max-w-[280px] truncate text-[12px] text-bad" title={error}>
            {error}
          </span>
        )}
        <button type="button" onClick={() => void save()} disabled={saving} className={primaryBtn}>
          {saving ? "저장 중…" : "저장"}
        </button>
      </div>
    </div>
  );
}

/** One collapsible knowledge entry editor. */
function KnowledgeCard({
  entry,
  initiallyOpen,
  onSaved,
  onDeleted,
}: {
  entry: KnowledgeEntry;
  initiallyOpen: boolean;
  onSaved: () => void;
  onDeleted: () => void;
}) {
  const [open, setOpen] = useState(initiallyOpen);
  const [title, setTitle] = useState(entry.title);
  const [body, setBody] = useState(entry.body);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const bodyRef = useAutoGrow(body, 260);
  const rootRef = useRef<HTMLDivElement>(null);
  const titleRef = useRef<HTMLInputElement>(null);

  // A freshly added entry is prepended to the TOP of the list, which can be far
  // above the "지식 추가" button at the bottom — so clicking add looked like it
  // did nothing. Scroll the new card into view and focus its title so the add
  // has an obvious, visible effect. Runs once on mount (fresh cards only).
  useEffect(() => {
    if (!initiallyOpen) return;
    rootRef.current?.scrollIntoView({ block: "center", behavior: "smooth" });
    titleRef.current?.focus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await saveKnowledge({ ...entry, title: title.trim(), body });
      setSaved(true);
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const isArtifact = entry.kind === "artifact";
  const files = entry.files ?? [];

  const remove = async () => {
    // Deleting a saved entry is immediate and has no undo — confirm first
    // (D57). Unsaved drafts (never persisted) are dropped without asking.
    // An artifact entry's copied documents go with it (D59).
    if (entry.updatedAt > 0) {
      const fileNote = isArtifact && files.length ? `\n첨부 문서 ${files.length}개도 함께 삭제됩니다.` : "";
      const ok = await ask(
        `'${title.trim() || "(제목 없음)"}' 지식 항목을 삭제할까요?${fileNote}\n삭제하면 되돌릴 수 없습니다.`,
        { title: "지식 삭제", kind: "warning" },
      );
      if (!ok) return;
    }
    setBusy(true);
    try {
      await deleteKnowledge(entry.id);
      onDeleted();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <div ref={rootRef} className={cardCls}>
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          className="flex min-w-0 flex-1 items-center gap-1.5 text-left text-[13px] font-medium text-ink-muted hover:text-ink"
        >
          <ChevronDown
            size={15}
            className={
              "shrink-0 transition-transform duration-[120ms] " + (open ? "rotate-0" : "-rotate-90")
            }
          />
          <span className="truncate text-ink-strong">{title.trim() || "(제목 없음)"}</span>
        </button>
        {isArtifact && (
          <span className="shrink-0 rounded-full bg-accent-tint px-1.5 py-px text-[10.5px] font-medium text-accent">
            산출물
          </span>
        )}
        {entry.updatedAt > 0 && (
          <span className="shrink-0 text-[11px] text-ink-faint">{sessionTime(entry.updatedAt)}</span>
        )}
        <button
          type="button"
          onClick={() => void remove()}
          disabled={busy}
          title="지식 삭제"
          className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-ink-soft transition-colors hover:bg-bad-bg hover:text-bad disabled:opacity-30"
        >
          <Trash2 size={14} />
        </button>
      </div>

      {open && (
        <div className="mt-3">
          {/* Artifact provenance (D59): where this entry came from. */}
          {isArtifact && (entry.sourceCategory || entry.sourceTitle) && (
            <div className="mb-2 text-[11.5px] text-ink-soft">
              출처:{" "}
              {[
                entry.sourceCategory ? categoryLabel(entry.sourceCategory as Category) : null,
                entry.sourceTitle,
              ]
                .filter(Boolean)
                .join(" · ")}
            </div>
          )}
          <input
            ref={titleRef}
            value={title}
            onChange={(e) => {
              setTitle(e.target.value);
              setSaved(false);
            }}
            disabled={busy}
            placeholder="지식 제목 (예: 주문 테이블 정합성 점검 방법)"
            className={inputCls + " mb-2"}
          />
          <textarea
            ref={bodyRef}
            value={body}
            onChange={(e) => {
              setBody(e.target.value);
              setSaved(false);
            }}
            disabled={busy}
            rows={4}
            placeholder={
              isArtifact
                ? "이 작업의 요약 — 이후 작업의 지식 주입 턴에 전달됩니다."
                : "어떤 상황에서 · 어떤 테이블/모듈을 보고 · 어떤 방식으로 접근했는지 기록하세요.\n작업 계획 수립 시 에이전트에게 제약·관례로 주입됩니다."
            }
            className="max-h-[260px] w-full resize-none overflow-y-auto rounded-[6px] border border-line bg-elevated px-2.5 py-2 font-mono text-[12px] leading-[1.55] text-ink outline-none focus:border-accent"
          />
          {/* Artifact documents (read-only — copied into the knowledge store,
              injected as an absolute-path index in later tasks). */}
          {isArtifact && files.length > 0 && (
            <div className="mt-2 rounded-[6px] border border-line bg-subtle px-2.5 py-2">
              <div className="mb-1 text-[11px] font-medium text-ink-soft">
                첨부 문서 {files.length}개 — 이후 작업에서 에이전트가 원문을 직접 읽습니다.
              </div>
              <ul className="space-y-0.5">
                {files.map((f) => (
                  <li key={f} className="truncate font-mono text-[11.5px] text-ink-muted">
                    {f}
                  </li>
                ))}
              </ul>
            </div>
          )}
          <div className="mt-2 flex items-center gap-2">
            <div className="flex-1" />
            {saved && <span className="text-[12.5px] text-ok">Saved.</span>}
            {error && (
              <span className="max-w-[280px] truncate text-[12px] text-bad" title={error}>
                {error}
              </span>
            )}
            <button
              type="button"
              onClick={() => void save()}
              disabled={busy || !title.trim()}
              className={primaryBtn}
            >
              {busy ? "저장 중…" : "저장"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

/** ③ 지식 베이스 — CRUD over the on-disk knowledge entries. */
function KnowledgeSection() {
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [freshIds, setFreshIds] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);

  const reload = () => {
    listKnowledge()
      .then(setEntries)
      .catch((e) => setError(String(e)));
  };

  useEffect(reload, []);

  const add = () => {
    const entry: KnowledgeEntry = {
      id: crypto.randomUUID(),
      title: "",
      body: "",
      createdAt: 0,
      updatedAt: 0,
    };
    setEntries((d) => [entry, ...d]);
    setFreshIds((s) => new Set(s).add(entry.id));
  };

  return (
    <div>
      <div className="mb-2.5 flex items-center gap-2">
        <span className="text-[12.5px] text-ink-muted">
          과거 작업 방식(상황·참조 테이블·접근 방법)을 기록해 두면, 기반 단계의 '지식 주입' 턴에
          에이전트에게 전달됩니다(총 16KB 상한, 최신순 우선). 워크스페이스에서 작업이 완료되면
          산출물 문서를 '산출물' 지식으로 저장할 수 있고, 이 경우 요약이 주입되며 원문은
          에이전트가 직접 읽습니다.
        </span>
      </div>
      {error && <div className="mb-2 text-[12px] text-bad">{error}</div>}
      <div className="flex flex-col gap-2.5">
        {entries.length === 0 && (
          <div className="rounded-[10px] border border-dashed border-line px-3 py-5 text-center text-[12.5px] text-ink-faint">
            등록된 지식이 없습니다. '지식 추가'로 첫 항목을 만들어 보세요.
          </div>
        )}
        {entries.map((e) => (
          <KnowledgeCard
            key={e.id}
            entry={e}
            initiallyOpen={freshIds.has(e.id)}
            onSaved={reload}
            onDeleted={() => setEntries((d) => d.filter((x) => x.id !== e.id))}
          />
        ))}
      </div>
      <div className="mt-3">
        <button type="button" onClick={add} className={ghostBtn}>
          <Plus size={14} /> 지식 추가
        </button>
      </div>
    </div>
  );
}

export function KnowledgeView({
  settings,
  onSettingsChange,
}: {
  settings: Settings | null;
  /** Mutations return the full new Settings; App replaces its state with it. */
  onSettingsChange: (s: Settings) => void;
}) {
  return (
    <div className="mx-auto max-w-[760px] px-6 py-7">
      <div className="mb-4">
        <h2 className="flex items-center gap-2 font-serif text-[22px] font-semibold tracking-[-0.02em] text-ink-strong">
          <BookOpen size={20} className="text-accent" />
          지식
        </h2>
        <p className="mt-0.5 text-[13px] text-ink-muted">
          기반 3단계가 사용하는 사내 지식을 관리합니다 — 사내 문서 RAG 검색 연결과 작업 방식
          지식 베이스.
        </p>
      </div>

      <RagSection settings={settings} onSettingsChange={onSettingsChange} />

      <div className="my-6 border-t border-line" />

      <h3 className="mb-2 font-serif text-[17px] font-semibold text-ink-strong">지식 베이스</h3>
      <KnowledgeSection />
    </div>
  );
}

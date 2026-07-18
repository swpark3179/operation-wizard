// Foundation-phase helpers (D44~D46): pure functions the ChatPanel preflight
// and the canvas RAG tab use — RAG query building, prompt-context formatting
// with size caps, and the self-contained HTML document for the "검색 결과"
// canvas tab (rendered via sandboxed iframe srcdoc, never written to disk).

import type { KnowledgeEntry, RagHit } from "./types";
import type { RagVerdict } from "./ragRelevance";

/** Per-context byte budget for prompt injection (rag excerpts / knowledge). */
const CONTEXT_CAP = 16_384;
const RAG_QUERY_CAP = 500;

function clip(text: string, max: number): string {
  return text.length > max ? text.slice(0, max) + " …(이하 생략)" : text;
}

/** Translate a raw RAG-adapter error into user-facing wording. The rag.rs
 * stub's "미구현 — src-tauri/…를 채워 주세요" message is developer guidance; a
 * non-developer user just needs to know the integration isn't wired up yet. */
export function ragUserError(msg: string): string {
  return msg.includes("미구현")
    ? "이 빌드에는 RAG 연동 모듈이 아직 구성되지 않았습니다. 관리자(개발팀)에게 문의해 주세요."
    : msg;
}

/** Build the RAG search query from the launcher prompt + option answers. */
export function buildRagQuery(seed: string, answersWire: string): string {
  const combined = [seed.trim(), answersWire.trim()].filter(Boolean).join("\n");
  return combined.length > RAG_QUERY_CAP ? combined.slice(0, RAG_QUERY_CAP) : combined;
}

/** Format search hits as numbered, source-attributed prompt context. */
export function formatRagContext(hits: RagHit[], capBytes = CONTEXT_CAP): string {
  const parts: string[] = ["다음은 사내 문서 지식베이스에서 검색된 발췌입니다:"];
  let used = parts[0].length;
  let dropped = 0;
  hits.forEach((hit, i) => {
    const title = hit.title?.trim() || `문서 ${i + 1}`;
    const source = hit.url?.trim() ? `\n출처: ${hit.url.trim()}` : "";
    const block = `\n\n[${i + 1}] ${title}${source}\n${clip(hit.snippet.trim(), 4000)}`;
    if (used + block.length > capBytes) {
      dropped += 1;
      return;
    }
    parts.push(block);
    used += block.length;
  });
  if (dropped > 0) parts.push(`\n\n(발췌 ${dropped}건은 길이 제한으로 생략)`);
  return parts.join("");
}

/** One entry's injected block. Artifact entries (D59) get a provenance line
 * and an absolute-path index of their copied documents — the summary (`body`)
 * is inlined but full documents never are; the agent reads them on demand via
 * the extraDirs grant. `artifactsRoot` = `<knowledge root>\artifacts` (null →
 * summaries only, no index). */
function knowledgeBlock(entry: KnowledgeEntry, artifactsRoot: string | null): string {
  const head = `\n\n## ${entry.title.trim()}`;
  const body = clip(entry.body.trim(), 4000);
  if (entry.kind !== "artifact" || !entry.files?.length) {
    return `${head}\n${body}`;
  }
  const origin = [entry.sourceCategory?.trim(), entry.sourceTitle?.trim()]
    .filter(Boolean)
    .join(" · ");
  const provenance = `[과거 작업 산출물${origin ? ` · ${origin}` : ""}]`;
  const index = artifactsRoot
    ? `\n첨부 문서(전문, 절대경로):\n${entry.files
        .map((name) => `- ${artifactsRoot}\\${entry.id}\\${name}`)
        .join("\n")}`
    : "";
  return `${head}\n${provenance}\n${body}${index}`;
}

/** Format knowledge entries (newest-first as given) as prompt context.
 * `artifactsRoot` is the absolute `knowledge\artifacts` folder used for the
 * per-entry document index (D59). */
export function formatKnowledgeContext(
  entries: KnowledgeEntry[],
  artifactsRoot: string | null,
  capBytes = CONTEXT_CAP,
): string {
  const parts: string[] = ["다음은 등록된 사내 지식입니다:"];
  let used = parts[0].length;
  let dropped = 0;
  let hasArtifactIndex = false;
  for (const entry of entries) {
    const block = knowledgeBlock(entry, artifactsRoot);
    if (used + block.length > capBytes) {
      dropped += 1;
      continue;
    }
    parts.push(block);
    used += block.length;
    if (artifactsRoot && entry.kind === "artifact" && entry.files?.length) {
      hasArtifactIndex = true;
    }
  }
  if (hasArtifactIndex) {
    parts.push(
      "\n\n위 첨부 문서는 파일 읽기 도구로 직접 열람할 수 있습니다. 요약만으로 부족하면 절대경로로 원문을 읽으세요.",
    );
  }
  if (dropped > 0) parts.push(`\n\n(지식 ${dropped}건은 길이 제한으로 생략)`);
  return parts.join("");
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** Shared style for the canvas "검색 결과" panel (raw + curated views). */
const RAG_PANEL_STYLE = `
  :root { color-scheme: light; }
  body { font-family: "Segoe UI", "Malgun Gothic", sans-serif; margin: 0; padding: 24px;
         background: #faf9f5; color: #3d3929; line-height: 1.6; }
  header { margin-bottom: 20px; }
  header h1 { font-size: 15px; margin: 0 0 4px; color: #6c6a60; font-weight: 600; }
  header .q { font-size: 14px; white-space: pre-wrap; background: #f0eee6; border: 1px solid #e0ddd1;
              border-radius: 8px; padding: 10px 12px; }
  header .reason { font-size: 13px; margin-top: 8px; color: #6c6a60; }
  article { background: #ffffff; border: 1px solid #e0ddd1; border-radius: 10px;
            padding: 14px 16px; margin-bottom: 12px; }
  article h2 { font-size: 14px; margin: 0 0 6px; }
  .n { display: inline-block; min-width: 20px; height: 20px; line-height: 20px; text-align: center;
       background: #c96442; color: #fff; border-radius: 999px; font-size: 11px; margin-right: 4px; }
  .score { font-size: 11px; color: #8f8b7e; font-weight: 400; margin-left: 6px; }
  .source { font-size: 12px; margin-bottom: 6px; word-break: break-all; }
  .source a { color: #c96442; }
  article ul { margin: 6px 0 8px; padding-left: 18px; }
  article li { font-size: 13px; margin: 0 0 4px; }
  p { font-size: 13px; margin: 0; white-space: pre-wrap; }
  .empty { color: #8f8b7e; font-size: 13px; }
`;

/** Wrap a header + body into the self-contained sandboxed-iframe document. */
function ragPanelDoc(headerHtml: string, bodyHtml: string): string {
  return `<!doctype html><html lang="ko"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>${RAG_PANEL_STYLE}</style></head><body>
${headerHtml}
${bodyHtml}
</body></html>`;
}

/** Build the self-contained HTML document for the canvas "검색 결과" tab (raw
 * hits — used as the fail-open fallback when no curated verdict is available).
 * Everything is escaped; the iframe is sandboxed (`allow-scripts`, no
 * same-origin) like the file viewer's HTML preview. */
export function ragResultHtml(query: string, hits: RagHit[]): string {
  const items = hits
    .map((hit, i) => {
      const title = escapeHtml(hit.title?.trim() || `문서 ${i + 1}`);
      const url = hit.url?.trim() ? escapeHtml(hit.url.trim()) : null;
      const score =
        typeof hit.score === "number" ? `<span class="score">${hit.score.toFixed(2)}</span>` : "";
      const source = url
        ? `<div class="source"><a href="${url}" target="_blank" rel="noreferrer">${url}</a></div>`
        : "";
      return `<article>
  <h2><span class="n">${i + 1}</span> ${title} ${score}</h2>
  ${source}
  <p>${escapeHtml(hit.snippet.trim())}</p>
</article>`;
    })
    .join("\n");
  const header = `<header><h1>사내 문서 검색 결과 · ${hits.length}건</h1><div class="q">${escapeHtml(query)}</div></header>`;
  return ragPanelDoc(header, items || '<p class="empty">검색 결과가 없습니다.</p>');
}

/** Build the CURATED "검색 결과" panel from the relevance judge's verdict (D70):
 * topic sections with bullet points + source links. Falls back to the raw hit
 * list when the verdict has no usable sections. */
export function ragCuratedHtml(query: string, verdict: RagVerdict, hits: RagHit[]): string {
  const sections = verdict.sections ?? [];
  if (sections.length === 0) return ragResultHtml(query, hits);

  // A source token is either a full URL (passes through) or a hit number that
  // maps back to that hit's url.
  const resolveSource = (token: string): string | null => {
    const t = token.trim();
    if (/^https?:\/\//i.test(t)) return t;
    const m = t.match(/\d+/);
    if (!m) return null;
    return hits[parseInt(m[0], 10) - 1]?.url?.trim() || null;
  };

  const body = sections
    .map((sec, i) => {
      const heading = escapeHtml(sec.heading?.trim() || `관련 정보 ${i + 1}`);
      const points = sec.points
        .map((p) => `<li>${escapeHtml(p.trim())}</li>`)
        .join("\n");
      const links = (sec.sources ?? [])
        .map((s) => {
          const label = escapeHtml(s.trim());
          const url = resolveSource(s);
          return url
            ? `<a href="${escapeHtml(url)}" target="_blank" rel="noreferrer">${label}</a>`
            : `<span>${label}</span>`;
        })
        .join(" · ");
      const source = links ? `<div class="source">출처: ${links}</div>` : "";
      return `<article class="section">
  <h2><span class="n">${i + 1}</span> ${heading}</h2>
  ${points ? `<ul>${points}</ul>` : ""}
  ${source}
</article>`;
    })
    .join("\n");

  const reason = verdict.reason?.trim()
    ? `<div class="reason">${escapeHtml(verdict.reason.trim())}</div>`
    : "";
  const header = `<header><h1>사내 문서 검색 결과 · 관련 정보 정리</h1><div class="q">${escapeHtml(query)}</div>${reason}</header>`;
  return ragPanelDoc(header, body);
}

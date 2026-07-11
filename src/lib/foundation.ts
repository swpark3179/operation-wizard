// Foundation-phase helpers (D44~D46): pure functions the ChatPanel preflight
// and the canvas RAG tab use — RAG query building, prompt-context formatting
// with size caps, and the self-contained HTML document for the "검색 결과"
// canvas tab (rendered via sandboxed iframe srcdoc, never written to disk).

import type { KnowledgeEntry, RagHit } from "./types";

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

/** Format knowledge entries (newest-first as given) as prompt context. */
export function formatKnowledgeContext(entries: KnowledgeEntry[], capBytes = CONTEXT_CAP): string {
  const parts: string[] = ["다음은 등록된 사내 지식입니다:"];
  let used = parts[0].length;
  let dropped = 0;
  for (const entry of entries) {
    const block = `\n\n## ${entry.title.trim()}\n${clip(entry.body.trim(), 4000)}`;
    if (used + block.length > capBytes) {
      dropped += 1;
      continue;
    }
    parts.push(block);
    used += block.length;
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

/** Build the self-contained HTML document for the canvas "검색 결과" tab.
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
  return `<!doctype html><html lang="ko"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
  :root { color-scheme: light; }
  body { font-family: "Segoe UI", "Malgun Gothic", sans-serif; margin: 0; padding: 24px;
         background: #faf9f5; color: #3d3929; line-height: 1.6; }
  header { margin-bottom: 20px; }
  header h1 { font-size: 15px; margin: 0 0 4px; color: #6c6a60; font-weight: 600; }
  header .q { font-size: 14px; white-space: pre-wrap; background: #f0eee6; border: 1px solid #e0ddd1;
              border-radius: 8px; padding: 10px 12px; }
  article { background: #ffffff; border: 1px solid #e0ddd1; border-radius: 10px;
            padding: 14px 16px; margin-bottom: 12px; }
  article h2 { font-size: 14px; margin: 0 0 6px; }
  .n { display: inline-block; min-width: 20px; height: 20px; line-height: 20px; text-align: center;
       background: #c96442; color: #fff; border-radius: 999px; font-size: 11px; margin-right: 4px; }
  .score { font-size: 11px; color: #8f8b7e; font-weight: 400; margin-left: 6px; }
  .source { font-size: 12px; margin-bottom: 6px; word-break: break-all; }
  .source a { color: #c96442; }
  p { font-size: 13px; margin: 0; white-space: pre-wrap; }
  .empty { color: #8f8b7e; font-size: 13px; }
</style></head><body>
<header><h1>사내 문서 검색 결과 · ${hits.length}건</h1><div class="q">${escapeHtml(query)}</div></header>
${items || '<p class="empty">검색 결과가 없습니다.</p>'}
</body></html>`;
}

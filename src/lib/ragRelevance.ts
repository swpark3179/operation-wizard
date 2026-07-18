// RAG relevance judge (D70): before the rag foundation step surfaces its search
// results, an ISOLATED agent turn decides whether they're actually relevant to
// the current task — and, when they are, organizes the key content into sections
// for a curated HTML panel. Irrelevant results are then skipped (no panel, no
// prompt injection) via the existing preflight skip-chain.
//
// The turn runs isolated exactly like knowledgeSave.ts::generateKnowledgeSummary
// (D59 family): a one-shot runAgent with no session id / resume, its own Channel,
// text accumulation, fenced-block parsing, and a cancel(). Any parse/agent
// failure resolves to null → the caller fails OPEN (shows the raw panel, injects
// as before) so relevant info is never wrongly hidden.

import { Channel, cancelRun, runAgent } from "./api";
import { fencedBlocks } from "./clarify";
import type { RagHit, RunEvent } from "./types";

/** One organized topic in the curated panel. */
export interface RagSection {
  heading: string;
  points: string[];
  /** Provenance tokens: hit numbers ("1") or full URLs. */
  sources?: string[];
}

/** The judge turn's verdict. `sections` is empty when `relevant` is false. */
export interface RagVerdict {
  relevant: boolean;
  reason?: string;
  sections?: RagSection[];
}

/** Cap each source chunk fed to the judge (keep the prompt bounded). */
const JUDGE_SNIPPET_CLIP = 2000;

function clip(text: string, max: number): string {
  return text.length > max ? text.slice(0, max) + " …(이하 생략)" : text;
}

/** Serialize the hits (summary answer + numbered source chunks) for the judge. */
function serializeHits(hits: RagHit[]): string {
  return hits
    .map((h, i) => {
      const title = h.title?.trim() || `문서 ${i + 1}`;
      const source = h.url?.trim() ? `\n출처: ${h.url.trim()}` : "";
      return `[${i + 1}] ${title}${source}\n${clip(h.snippet.trim(), JUDGE_SNIPPET_CLIP)}`;
    })
    .join("\n\n");
}

/** Build the judge instruction: is this search result relevant to the task, and
 * if so, organize its key content. Output contract = one ```ragrelevance fence. */
export function ragRelevanceInstruction(task: string, hits: RagHit[]): string {
  return `[시스템 지시: 사내 문서 검색 결과 관련성 판단 단계]
아래는 사용자의 현재 작업 내용과, 사내 문서 지식베이스에서 검색된 결과입니다.
이 검색 결과가 현재 작업에 실제로 도움이 되는지(관련이 있는지) 판단하고, 관련이 있으면 핵심 내용을 주제별로 정리하세요.

반드시 아래 규칙을 지키세요:
1. 응답은 다른 텍스트 없이 아래 형식의 \`\`\`ragrelevance 코드 펜스 하나로만 출력하세요.
2. 블록 안은 유효한 JSON이며 스키마는 다음과 같습니다:
   {
     "relevant": true | false,
     "reason": "관련 있다/없다고 판단한 짧은 근거(한국어)",
     "sections": [
       { "heading": "소제목", "points": ["핵심 요점", "..."], "sources": ["1", "https://..."] }
     ]
   }
3. 검색 결과에 현재 작업과 관련된 유용한 정보가 사실상 없으면 "relevant"를 false로 두고 "sections"는 빈 배열([])로 두세요.
4. "relevant"가 true인 경우에만 "sections"를 채우되, 검색 결과에 **실제로 있는 내용만** 사용하세요(지어내기 금지). "sources"에는 근거가 된 문서 번호(예: "1")나 출처 URL을 넣으세요.

현재 작업:
---
${task.trim()}
---

검색 결과:
---
${serializeHits(hits)}
---
`;
}

function coerceSections(raw: unknown): RagSection[] {
  if (!Array.isArray(raw)) return [];
  const out: RagSection[] = [];
  for (const s of raw) {
    const o = s as Partial<RagSection>;
    const heading = typeof o?.heading === "string" ? o.heading.trim() : "";
    const points = Array.isArray(o?.points)
      ? o.points.filter((p): p is string => typeof p === "string" && p.trim().length > 0)
      : [];
    if (!heading && points.length === 0) continue; // empty section — drop
    const sources = Array.isArray(o?.sources)
      ? o.sources.filter((x): x is string => typeof x === "string" && x.trim().length > 0)
      : undefined;
    out.push({ heading: heading || "관련 정보", points, sources });
  }
  return out;
}

/** Parse the ```ragrelevance verdict (else any fenced JSON with a boolean
 * `relevant`). Returns null on any failure → the caller fails open. */
export function parseRagRelevance(content: string): RagVerdict | null {
  const blocks = fencedBlocks(content);
  if (blocks.length === 0) return null;
  const ordered = [...blocks.filter((b) => b.tag === "ragrelevance"), ...blocks];
  for (const b of ordered) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(b.body.trim());
    } catch {
      continue;
    }
    const o = parsed as { relevant?: unknown; reason?: unknown; sections?: unknown };
    if (typeof o?.relevant !== "boolean") continue;
    return {
      relevant: o.relevant,
      reason: typeof o.reason === "string" && o.reason.trim() ? o.reason.trim() : undefined,
      sections: coerceSections(o.sections),
    };
  }
  return null;
}

/** Run the isolated relevance-judge turn. Resolves with the parsed verdict, or
 * null on failure/cancel/empty (caller fails open). Mirrors
 * generateKnowledgeSummary (knowledgeSave.ts). */
export function judgeRagRelevance(args: {
  agentId: string;
  model: string | null;
  /** Project workdir (the isolated turn's cwd; it reads nothing under it). */
  cwd: string;
  /** The task descriptor (the same query used for the RAG search). */
  task: string;
  hits: RagHit[];
  onText?: (accumulated: string) => void;
}): { promise: Promise<RagVerdict | null>; cancel: () => void } {
  let runId: string | null = null;
  let canceled = false;
  let acc = "";

  const promise = new Promise<RagVerdict | null>((resolve) => {
    const channel = new Channel<RunEvent>();
    channel.onmessage = (ev) => {
      switch (ev.type) {
        case "textDelta":
          acc += ev.delta;
          args.onText?.(acc);
          break;
        case "stdout":
          acc += ev.chunk;
          args.onText?.(acc);
          break;
        case "end":
          resolve(ev.status === "succeeded" && !canceled ? parseRagRelevance(acc) : null);
          break;
        default:
          break;
      }
    };
    runAgent(
      {
        agentId: args.agentId,
        prompt: ragRelevanceInstruction(args.task, args.hits),
        cwd: args.cwd,
        model: args.model,
        sessionId: null,
        resume: false,
        extraDirs: [], // judgment is text-only; no folder access needed
      },
      channel,
    )
      .then((id) => {
        runId = id;
        if (canceled) void cancelRun(id).catch(() => {});
      })
      .catch(() => resolve(null)); // spawn failure → fail open
  });

  return {
    promise,
    cancel: () => {
      canceled = true;
      if (runId) void cancelRun(runId).catch(() => {});
    },
  };
}

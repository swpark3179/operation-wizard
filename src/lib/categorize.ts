// Category classification (D81): when a user starts from the HOME composer by
// typing a prompt (rather than clicking a category card), an ISOLATED agent turn
// decides which of the four work categories best fits the request, so the
// workflow routes to the right guided flow instead of always defaulting to plan.
//
// The turn runs isolated exactly like ragRelevance.ts::judgeRagRelevance and
// knowledgeSave.ts::generateKnowledgeSummary: a one-shot runAgent with no session
// id / resume, its own Channel, text accumulation, fenced-block parsing, and a
// cancel(). Any parse/agent failure resolves to null → the caller falls back to
// "plan" (the historical default), so a misclassification never blocks the user.

import { Channel, cancelRun, runAgent } from "./api";
import { fencedBlocks } from "./clarify";
import { CATEGORIES, type Category } from "../components/workspace";
import type { RunEvent } from "./types";

/** Valid category ids, for validating the parsed classification. */
const VALID: ReadonlySet<string> = new Set(CATEGORIES.map((c) => c.id));

/** Build the classification instruction: pick the single best-fitting category
 * for the user's request. Output contract = one ```category fence. */
export function categorizeInstruction(seed: string): string {
  const list = CATEGORIES.map((c) => `- ${c.id} — ${c.label}: ${c.desc}`).join("\n");
  return `[시스템 지시: 작업 유형(카테고리) 분류 단계]
아래 사용자의 요청을 읽고, 다음 4개 작업 카테고리 중 이 요청에 **가장 잘 맞는 하나**를 고르세요.

카테고리 목록(id — 이름: 설명):
${list}

판단 기준(요약):
- plan: 코드/기능 변경을 어떻게 구현·수정할지 계획하고 계획서·영향도·테스트 계획을 만드는 작업.
- guide: 반복되는 운영 절차를 단계별 가이드/런북 문서로 정리하는 작업.
- query: 데이터를 조회·확인·집계하는 작업(읽기 전용, 수정 없음).
- change: 데이터 수정·테이블/스키마 변경·권한 부여 등 변경 작업.

반드시 아래 규칙을 지키세요:
1. 응답은 다른 텍스트 없이 아래 형식의 \`\`\`category 코드 펜스 하나로만 출력하세요.
2. 블록 안은 유효한 JSON이며 스키마는 다음과 같습니다:
   {
     "category": "plan" | "guide" | "query" | "change",
     "reason": "그 카테고리를 고른 짧은 근거(한국어)"
   }
3. 확신이 서지 않으면 개발 계획 수립(plan)을 선택하세요.

사용자 요청:
---
${seed.trim()}
---
`;
}

/** Parse the ```category verdict (else any fenced JSON with a valid `category`).
 * Returns null on any failure → the caller falls back to "plan". */
export function parseCategory(content: string): Category | null {
  const blocks = fencedBlocks(content);
  if (blocks.length === 0) return null;
  const ordered = [...blocks.filter((b) => b.tag === "category"), ...blocks];
  for (const b of ordered) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(b.body.trim());
    } catch {
      continue;
    }
    const o = parsed as { category?: unknown };
    if (typeof o?.category === "string" && VALID.has(o.category)) {
      return o.category as Category;
    }
  }
  return null;
}

/** Run the isolated classification turn. Resolves with the chosen category, or
 * null on failure/cancel/empty (caller falls back to "plan"). Mirrors
 * judgeRagRelevance (ragRelevance.ts). */
export function classifyCategory(args: {
  agentId: string;
  model: string | null;
  /** Project workdir (the isolated turn's cwd; it reads nothing under it). */
  cwd: string;
  /** The launcher prompt to classify. */
  seed: string;
  onText?: (accumulated: string) => void;
}): { promise: Promise<Category | null>; cancel: () => void } {
  let runId: string | null = null;
  let canceled = false;
  let acc = "";

  const promise = new Promise<Category | null>((resolve) => {
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
          resolve(ev.status === "succeeded" && !canceled ? parseCategory(acc) : null);
          break;
        default:
          break;
      }
    };
    runAgent(
      {
        agentId: args.agentId,
        prompt: categorizeInstruction(args.seed),
        cwd: args.cwd,
        model: args.model,
        sessionId: null,
        resume: false,
        extraDirs: [], // classification is text-only; no folder access needed
      },
      channel,
    )
      .then((id) => {
        runId = id;
        if (canceled) void cancelRun(id).catch(() => {});
      })
      .catch(() => resolve(null)); // spawn failure → fall back to plan
  });

  return {
    promise,
    cancel: () => {
      canceled = true;
      if (runId) void cancelRun(runId).catch(() => {});
    },
  };
}

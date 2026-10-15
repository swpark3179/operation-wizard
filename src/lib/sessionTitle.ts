// Session title from the requirement (D97): the requirements form's mandatory
// "무엇을 하고 싶으신가요?" answer is summarized into a short title for the
// session (and, for a new project, the project) — so the recent list and the
// history popover no longer show the same category name for every entry.
//
// Two layers:
// - `heuristicTitle` — instant and deterministic (first line, clipped). Shown
//   right away and used when the agent turn fails.
// - `summarizeTitle` — an ISOLATED agent turn exactly like
//   ragRelevance.ts::judgeRagRelevance / knowledgeSave.ts: a one-shot runAgent
//   with no session id / resume, its own Channel, text accumulation, a
//   ```title fence, and a cancel(). It runs alongside the first work turn and
//   never blocks it; any failure resolves to null (the heuristic stays).

import { Channel, cancelRun, runAgent } from "./api";
import { fencedBlocks } from "./clarify";
import type { RunEvent } from "./types";

/** Title length cap (characters) — fits the recent list / chat header. */
export const TITLE_MAX = 40;

function clip(s: string): string {
  return s.length > TITLE_MAX ? `${s.slice(0, TITLE_MAX - 1).trimEnd()}…` : s;
}

/** Normalize a candidate title: one line, collapsed whitespace, no wrapping
 * quotes / markdown heading marks / trailing period, clipped. */
function cleanTitle(raw: string): string {
  const line = raw
    .split(/\r?\n/)
    .map((l) => l.trim())
    .find(Boolean);
  if (!line) return "";
  const t = line
    .replace(/^#+\s*/, "")
    .replace(/^(제목|title)\s*[:：]\s*/i, "")
    .replace(/^["'“‘「『`]+|["'”’」』`]+$/g, "")
    .replace(/\s+/g, " ")
    .replace(/[.。]+$/, "")
    .trim();
  return clip(t);
}

/** Instant fallback title: the requirement's first line, clipped. */
export function heuristicTitle(requirement: string): string {
  return cleanTitle(requirement);
}

/** The summarization instruction. Output contract = one ```title fence. */
export function titleInstruction(requirement: string, categoryLabel: string): string {
  return `[시스템 지시: 작업 제목 요약]
아래는 사용자가 "${categoryLabel}" 작업으로 입력한 요구사항입니다. 이 작업을 목록에서 한눈에 구분할 수 있는
**짧은 제목**으로 요약하세요.

반드시 아래 규칙을 지키세요:
1. 한국어 명사구로 ${TITLE_MAX}자 이내, 한 줄로 쓰세요. 대상(화면·테이블·배치 등)과 작업 내용이 드러나야 합니다.
2. "요청", "작업", "${categoryLabel}" 같은 일반적인 말이나 따옴표·마침표·이모지를 넣지 마세요.
3. 응답은 다른 텍스트 없이 아래 형식의 \`\`\`title 코드 펜스 하나로만 출력하세요.

예시:
\`\`\`title
주문 목록 배송상태 필터 추가
\`\`\`

요구사항:
---
${requirement.trim()}
---
`;
}

/** Parse the ```title fence. A fence-less reply is accepted only when it is a
 * single short line (plain agents echo raw stdout); anything chattier is
 * rejected → null (the caller keeps the heuristic title). */
export function parseTitle(content: string): string | null {
  const block = fencedBlocks(content).find((b) => b.tag === "title");
  if (block) return cleanTitle(block.body) || null;
  const trimmed = content.trim();
  if (!trimmed || trimmed.includes("\n") || trimmed.includes("```") || trimmed.length > TITLE_MAX) {
    return null;
  }
  return cleanTitle(trimmed) || null;
}

/** Run the isolated title turn. Resolves with the summarized title, or null on
 * failure/cancel/unparseable output. */
export function summarizeTitle(args: {
  agentId: string;
  model: string | null;
  /** Project workdir (the isolated turn's cwd; it reads nothing under it). */
  cwd: string;
  requirement: string;
  categoryLabel: string;
}): { promise: Promise<string | null>; cancel: () => void } {
  let runId: string | null = null;
  let canceled = false;
  let acc = "";

  const promise = new Promise<string | null>((resolve) => {
    const channel = new Channel<RunEvent>();
    channel.onmessage = (ev) => {
      switch (ev.type) {
        case "textDelta":
          acc += ev.delta;
          break;
        case "stdout":
          acc += ev.chunk;
          break;
        case "end":
          resolve(ev.status === "succeeded" && !canceled ? parseTitle(acc) : null);
          break;
        default:
          break;
      }
    };
    runAgent(
      {
        agentId: args.agentId,
        prompt: titleInstruction(args.requirement, args.categoryLabel),
        cwd: args.cwd,
        model: args.model,
        sessionId: null,
        resume: false,
        extraDirs: [], // text-only; no folder access needed
        tools: false, // no FabriX tool rounds for a one-line title (D84)
      },
      channel,
    )
      .then((id) => {
        runId = id;
        if (canceled) void cancelRun(id).catch(() => {});
      })
      .catch(() => resolve(null)); // spawn failure → keep the heuristic title
  });

  return {
    promise,
    cancel: () => {
      canceled = true;
      if (runId) void cancelRun(runId).catch(() => {});
    },
  };
}

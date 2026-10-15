// Learnings hand-off (frontend-only, D86). A code-analysis step's document ends
// with a `## Learnings for Next Steps` section (the built-in contract in
// codeMethod.ts); the app extracts it and injects it — with the absolute paths
// of the documents produced so far — into the later steps' wire, so they reuse
// the analysis instead of re-exploring the codebase (and sessionless agents,
// whose transcript skips the auto-advanced step wires, keep the context).

import type { CodebaseLearnings, StepDef } from "./types";

/** Per-step cap on the injected Learnings excerpt. */
export const LEARNINGS_MAX = 3000;

function clip(text: string, max: number): string {
  const t = text.trim();
  return t.length > max ? `${t.slice(0, max)}\n…(잘림)` : t;
}

/** Extract the Learnings section from a markdown document, or null. Scans lines
 * outside code fences (same CommonMark fence rules as `extractMermaidBlocks`)
 * for a heading mentioning "Learnings"/"학습" and takes everything up to the
 * next heading of the same or a higher level. No such heading → falls back to
 * the first `[Architecture Pattern]` label through the end of the document. */
export function extractLearnings(md: string): string | null {
  const lines = md.split(/\r?\n/);
  let fence: { char: string; len: number } | null = null;
  let start = -1;
  let level = 0;
  let end = lines.length;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const f = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
    if (fence) {
      if (f && f[1][0] === fence.char && f[1].length >= fence.len && f[2].trim() === "") fence = null;
      continue;
    }
    if (f && !(f[1][0] === "`" && f[2].includes("`"))) {
      fence = { char: f[1][0], len: f[1].length };
      continue;
    }
    const h = /^ {0,3}(#{1,6})\s+(.*)$/.exec(line);
    if (!h) continue;
    if (start < 0) {
      if (/learnings?|학습/i.test(h[2])) {
        start = i + 1;
        level = h[1].length;
      }
    } else if (h[1].length <= level) {
      end = i;
      break;
    }
  }
  let body = "";
  if (start >= 0) {
    body = lines.slice(start, end).join("\n");
  } else {
    const idx = md.indexOf("[Architecture Pattern]");
    if (idx >= 0) {
      const lineStart = md.lastIndexOf("\n", idx) + 1;
      body = md.slice(lineStart);
    }
  }
  const out = clip(body, LEARNINGS_MAX);
  return out ? out : null;
}

/** One document produced by an earlier step of this run. */
export interface ProducedDoc {
  step: StepDef;
  /** Absolute path of the produced file. */
  path: string;
  /** Extracted Learnings (code-analysis steps only; null = none found). */
  learnings: string | null;
  /** Index label instead of the step name (the app-made repository map — D94). */
  label?: string;
}

function docLabel(d: ProducedDoc): string {
  return d.label ?? (d.step.name?.trim() || d.step.id);
}

/** The hand-off context for a later generative step: the absolute-path index of
 * the documents produced so far + the code-analysis Learnings excerpts. Empty
 * string when nothing was produced yet. */
export function formatHandoffContext(docs: ProducedDoc[]): string {
  if (!docs.length) return "";
  const parts: string[] = [];
  parts.push(
    [
      "[앞 단계 산출물 — 절대경로]",
      ...docs.map((d) => `- ${docLabel(d)}: ${d.path}`),
      "이번 단계에 필요한 근거는 먼저 위 문서에서 찾으세요. 코드베이스를 처음부터 다시 탐색하지 말고,",
      "문서에 없는 사실이 필요할 때만 해당 파일·심볼을 선택적으로 읽으세요.",
    ].join("\n"),
  );
  for (const d of docs) {
    if (!d.learnings) continue;
    parts.push(
      `[앞 단계 코드 분석 Learnings — ${docLabel(d)} (${d.path})]\n${d.learnings}`,
    );
  }
  return parts.join("\n\n");
}

function changedNote(c: CodebaseLearnings): string {
  if (c.changedCount === 0) return "이후 변경된 파일 없음";
  const sample = c.changedSample.length ? `: ${c.changedSample.join(", ")}` : "";
  return `이후 변경 파일 ${c.changedCount}개${c.changedCapped ? " 이상" : ""}${sample}`;
}

function savedDate(c: CodebaseLearnings): string {
  return c.updatedAt ? new Date(c.updatedAt).toISOString().slice(0, 10) : "?";
}

/** Wire context for Learnings an earlier project cached for this codebase
 * (D90): start from them, re-check only what changed / what this request
 * touches, and write the refreshed full Learnings again. */
export function formatCachedLearnings(c: CodebaseLearnings): string {
  return [
    `[이전 분석 Learnings — 같은 코드베이스를 ${savedDate(c)}에 분석한 결과, ${changedNote(c)}]`,
    c.text.trim(),
    "이 내용을 출발점으로 삼아 저장소를 처음부터 다시 탐색하지 말고, 변경된 파일과 이번 요청 관련 영역만 확인·갱신하세요. " +
      "이번 분석서의 Learnings 섹션에는 갱신된 전체 내용을 다시 쓰세요.",
  ].join("\n");
}

/** The chat note shown when cached Learnings are reused. */
export function cachedLearningsNote(c: CodebaseLearnings): string {
  return `이전 코드베이스 분석(${savedDate(c)})의 Learnings를 출발점으로 재활용합니다 — ${changedNote(c)}.`;
}

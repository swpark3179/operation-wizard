// Prompt-optimizer built-in skill (D65). Unlike registry skills (skills.ts,
// user-replaceable wholesale — D39), this one is hardcoded and injected at
// runtime into the FIRST actual work turn of every conversation, regardless of
// any saved skill/workflow overrides. The agent is asked to open its reply
// with exactly one ```prompt fence containing an optimized restatement of the
// user's request (built from the requirements-form answers), then continue the
// step's real work in the same turn. The client parses the fence on `end`,
// strips it from the chat, and shows it in the canvas "프롬프트" tab so users
// gradually learn effective prompting by example.
//
// Contract mirrors knowledgeSave.ts's ```summary fence, with one deliberate
// difference: NO whole-reply fallback — the reply is the actual work output,
// so a missing fence must mean "no prompt to show", never "show everything".

import { fencedBlocks } from "./clarify";

/** Prepended above the first work turn's step skills/instruction (wire top). */
export const PROMPT_OPTIMIZER_SKILL = `[시스템 스킬: 프롬프트 최적화]
이번 턴에는 두 가지 임무가 있습니다. 반드시 이 순서로 수행하세요.

1) 최적화된 프롬프트 작성 — 응답의 가장 처음에, 아래 요구사항과 선택 항목 답변을 바탕으로
   "이 작업을 처음부터 다시 요청한다면 이렇게 써야 한다"는 관점의 최적 프롬프트 하나를
   \`\`\`prompt 코드 펜스 하나로만 출력하세요. 프롬프트는 한국어로 쓰고 다음 요소를 갖추세요:
   - 역할 부여(예: "당신은 ~ 전문가입니다"), 상황·배경 요약, 구체적인 작업 지시,
     제약 조건, 기대 산출물과 그 형식, 필요하면 단계별 진행 지시.
   - 사용자가 제공하지 않은 사실을 지어내지 마세요. 모르는 값은 <미정>으로 표기하세요.
   - 펜스 앞뒤에 다른 설명을 붙이지 마세요.
2) 실제 작업 수행 — 펜스를 닫은 직후, 이어지는 지시문에 따라 이번 단계의 실제 작업을
   즉시 계속 진행하세요. 1)은 사용자 학습용 표시일 뿐 작업 범위를 바꾸지 않습니다.`;

/** Replaces the fence in the visible/persisted chat text once parsed out. */
export const PROMPT_NOTE = "(최적화된 프롬프트를 캔버스 '프롬프트' 탭에 표시했습니다.)";

/** The optimized prompt from the turn's reply — the ```prompt fence body only
 * (no fallback; see header). Null when absent/blank → graceful no-op. */
export function parsePromptBlock(content: string): string | null {
  const body = fencedBlocks(content)
    .find((b) => b.tag === "prompt")
    ?.body.trim();
  return body || null;
}

/** Swap the ```prompt fence(s) for PROMPT_NOTE, keeping every other code
 * block. Runs on the stored message content, so the note is what persists and
 * what sessionless transcripts re-send (the agent never re-sees the fence). */
export function stripPromptBlock(content: string): string {
  return content
    .replace(/```([^\n`]*)\n?[\s\S]*?```/g, (full, tag: string) =>
      (tag ?? "").trim().toLowerCase() === "prompt" ? PROMPT_NOTE : full,
    )
    .trim();
}

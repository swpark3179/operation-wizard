// Built-in "prompt optimizer" skill (frontend-only). Unlike the registry skills
// in skills.ts, this one is hardcoded and injected at RUNTIME on the first real
// work turn of every category — outside the user-editable registry so it always
// applies, even to users who saved a full-replace skill override (D39/D78). The
// agent emits an optimized prompt as a ```prompt fence FIRST (educational: the
// user learns better prompting), then continues the real task in the same turn.
//
// It mirrors the fenced-block contract used by clarify.ts / knowledgeSave.ts, but
// deliberately has NO whole-response fallback: if the agent never fenced a prompt,
// the reply is the real work output and must not be mistaken for a prompt.
import { fencedBlocks } from "./clarify";

/** Prepended (invisibly) to the first real work turn's wire (above the step
 * skills → wire top). */
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

/** Shown in the chat where the ```prompt block was stripped out to the canvas. */
export const PROMPT_NOTE = "(최적화된 프롬프트를 캔버스 '프롬프트' 탭에 표시했습니다.)";

/** Extract the ```prompt block's body, or null. Tag match only — NO fallback to
 * the whole response (that is the real work output, not a prompt). */
export function parsePromptBlock(content: string): string | null {
  const block = fencedBlocks(content).find((b) => b.tag === "prompt");
  const body = block?.body.trim();
  return body ? body : null;
}

/** Replace the ```prompt fence(s) with PROMPT_NOTE in the visible chat text,
 * keeping every other code block intact. Mirrors stripClarifyBlock. */
export function stripPromptBlock(content: string): string {
  return content.replace(/```([^\n`]*)\n?[\s\S]*?```/g, (full, tag: string) => {
    const t = (tag ?? "").trim().toLowerCase();
    return t === "prompt" ? PROMPT_NOTE : full; // keep unrelated code blocks
  });
}

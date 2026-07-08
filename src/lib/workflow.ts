// Category workflow orchestration (frontend-only).
//
// The local CLI agents expose no custom tool-call channel (design D30/D34), so a
// multi-step flow is driven entirely on the client: each step prepends its
// skills (see `skills.ts` + `StepDef.skillIds`) and its instruction to the wire
// prompt, the streamed reply is interpreted by the step's `kind`, and the
// orchestrator either AUTO-ADVANCES to the next step (generative steps like
// search/document) or stops at a terminal `chat` step. The interactive first
// phase (deciding the fixed option catalog) is NOT a step here — it runs before
// the workflow (see `options.ts` + ChatPanel); the first workflow step runs on
// the turn that submits the options. The whole workflow is transient (never
// persisted); generated files persist as real files, and a parse/step failure
// always falls back to plain chat so the conversation never breaks.
//
// The catalogs below are the BUILT-IN DEFAULTS + editable sample content: the
// Flows settings view shows them as the initial steps, and a user-saved
// workflow in settings.json (`settings.workflows[category]`) replaces the
// category's default wholesale (design D39/D40).

import type { Category } from "../components/workspace";
import type { Settings, StepDef } from "./types";

export type StepKind = "search" | "document" | "chat";

const STEP_KINDS: readonly string[] = ["search", "document", "chat"];

// ── plan(프로그램 변경 계획) 기본 플로우: 소스코드 분석 → 컨플루언스 탐색 →
//    계획 생성 → 변경영향분석서 → 테스트 계획서 → 마무리 대화 ─────────────────────
// The requirements options are a pre-workflow phase (options.ts); the steps
// below start on the options-submit turn and auto-advance through the
// generative (search/document) steps.

const SOURCE_ANALYSIS_STEP = `[시스템 지시: 소스코드 분석 단계]
확정된 요구사항을 바탕으로 작업 폴더의 소스코드를 분석하고, 결과를 **분석서 파일로 작성**하세요.
- 파일 검색·읽기 도구로 관련 소스를 실제로 확인한 뒤, \`docs/source-analysis.md\` 파일을
  만들어(없으면 docs 폴더도 생성) 파일 쓰기 도구로 저장하세요.
- 분석서에는 다음을 포함하세요: 프로젝트 구조 개요 / 변경 대상 모듈과 의존 관계(mermaid 다이어그램) /
  주요 호출 흐름 / 미확인 사항·추가로 필요한 정보.
- 아직 코드를 수정하거나 계획을 세우지 마세요. 이번 턴은 "분석과 기록"만 합니다.
- 파일 저장 후, 핵심 발견 사항을 한두 문장으로만 보고하세요.`;

const CONFLUENCE_STEP = `[시스템 지시: 컨플루언스(사내 문서) 탐색 단계]
이번 변경과 관련된 사내 문서·설계 자료를 조사하세요.
- 사내 위키(Confluence)에 접근할 도구가 있으면 그것으로 검색하고, 없으면 작업 폴더의
  \`docs/\` 등 로컬 문서를 탐색한 뒤 사용자에게 필요한 문서 목록을 요청하세요.
- 아직 계획서를 작성하지 마세요. 이번 턴은 "문서 조사와 요약"만 합니다.
- 마지막에 찾은 자료(출처 포함)와 찾지 못해 사용자 확인이 필요한 항목을 정리해 보고하세요.`;

const PLAN_STEP = `[시스템 지시: 계획서 작성 단계]
지금까지의 요구사항·소스 분석·문서 조사 결과를 종합해 **개발 계획서를 파일로 작성**하세요.
- 작업 폴더에 \`docs/plan.md\` 파일을 만들어(없으면 docs 폴더도 생성) 파일 쓰기 도구로 저장하세요.
- 계획서에는 다음을 포함하세요: 개요 / 요구사항 요약 / 영향받는 소스 / 구현 단계(순서) /
  일정·선행 조건 / 리스크·확인 필요 사항.
- 파일을 저장한 뒤에는, 무엇을 어디에 작성했는지 한두 문장으로만 보고하세요.`;

const IMPACT_STEP = `[시스템 지시: 변경영향분석서 작성 단계]
계획서를 근거로 **변경영향분석서를 파일로 작성**하세요.
- 작업 폴더에 \`docs/impact-analysis.md\` 파일을 만들어 파일 쓰기 도구로 저장하세요.
- 분석서에는 다음을 포함하세요: 영향 파일 표(경로/변경 유형/영향 내용/위험도) / 호출 경로·데이터
  흐름의 전파 범위 / 코드 밖 영향(DB·API·설정) / 회귀 위험 영역 / mermaid 의존 다이어그램.
- 파일을 저장한 뒤에는 핵심 위험 요약만 한두 문장으로 보고하세요.`;

const TEST_PLAN_STEP = `[시스템 지시: 테스트 계획서 작성 단계]
변경영향분석서의 영향 범위를 근거로 **테스트 계획서를 파일로 작성**하세요.
- 작업 폴더에 \`docs/test-plan.md\` 파일을 만들어 파일 쓰기 도구로 저장하세요.
- 계획서에는 다음을 포함하세요: 테스트 케이스 표(ID/분류/시나리오/사전조건·데이터/기대 결과) /
  경계·실패·롤백 시나리오 / 테스트 데이터·환경 준비 / 완료 기준.
- 파일을 저장한 뒤에는 케이스 구성 요약만 한두 문장으로 보고하세요.`;

/** Built-in default workflows (also the sample content in the Flows view). */
export const DEFAULT_WORKFLOWS: Record<Category, StepDef[]> = {
  plan: [
    {
      id: "source-analysis",
      name: "소스코드 분석",
      kind: "document",
      instruction: SOURCE_ANALYSIS_STEP,
      file: "docs/source-analysis.md",
      skillIds: ["source-analysis"],
    },
    {
      id: "confluence",
      name: "컨플루언스 탐색",
      kind: "search",
      instruction: CONFLUENCE_STEP,
      skillIds: ["confluence-search"],
    },
    {
      id: "plan",
      name: "계획 생성",
      kind: "document",
      instruction: PLAN_STEP,
      file: "docs/plan.md",
      skillIds: ["plan-method"],
    },
    {
      id: "impact",
      name: "변경영향분석서 생성",
      kind: "document",
      instruction: IMPACT_STEP,
      file: "docs/impact-analysis.md",
      skillIds: ["impact-analysis"],
    },
    {
      id: "test-plan",
      name: "테스트 계획서 생성",
      kind: "document",
      instruction: TEST_PLAN_STEP,
      file: "docs/test-plan.md",
      skillIds: ["test-plan"],
    },
    { id: "chat", name: "마무리 대화", kind: "chat", instruction: "", skillIds: [] },
  ],
  guide: [
    { id: "guide", name: "대화", kind: "chat", instruction: "", skillIds: ["guide-author"] },
  ],
  query: [
    { id: "query", name: "대화", kind: "chat", instruction: "", skillIds: ["query-safe"] },
  ],
  change: [
    { id: "change", name: "대화", kind: "chat", instruction: "", skillIds: ["change-safe"] },
  ],
};

/** Coerce stored steps into a shape the orchestrator can trust: drop entries
 * with a missing id or unknown kind, and guarantee a terminal `chat` step (the
 * runtime never trusts persisted data, even though saves are validated). */
export function coerceSteps(steps: StepDef[]): StepDef[] {
  const out = (steps ?? []).filter(
    (s) =>
      !!s &&
      typeof s.id === "string" &&
      s.id.trim() !== "" &&
      STEP_KINDS.includes(s.kind) &&
      Array.isArray(s.skillIds),
  );
  if (out.length === 0 || out[out.length - 1].kind !== "chat") {
    out.push({ id: "chat-terminal", name: "대화", kind: "chat", instruction: "", skillIds: [] });
  }
  return out;
}

/** The effective steps for a category (user override else default; ≥1 step,
 * always ending in a terminal chat step). */
export function workflowFor(category: Category, settings: Settings | null): StepDef[] {
  const raw = settings?.workflows?.[category] ?? DEFAULT_WORKFLOWS[category] ?? DEFAULT_WORKFLOWS.plan;
  return coerceSteps(raw);
}

/** Derived progress note for an (auto-)starting step, e.g. "3/6단계 · 계획 생성 중…". */
export function progressLabel(index: number, steps: StepDef[]): string {
  const name = steps[index]?.name?.trim() || steps[index]?.id || "";
  return `${index + 1}/${steps.length}단계 · ${name} 중…`;
}

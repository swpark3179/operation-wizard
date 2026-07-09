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

export type StepKind = "search" | "document" | "chat" | "codebase" | "rag" | "knowledge";

/** Keep in sync with `STEP_KINDS` in src-tauri/src/settings.rs. */
const STEP_KINDS: readonly string[] = [
  "search",
  "document",
  "chat",
  "codebase",
  "rag",
  "knowledge",
];

/** The mandatory foundation pre-phase, in canonical order (D44). Presence of
 * any of these kinds in a stored workflow means "foundation enabled" for that
 * category (plan is always enabled). */
export const FOUNDATION_KINDS: readonly string[] = ["codebase", "rag", "knowledge"];

/** Step output modes — keep in sync with `STEP_OUTPUTS` in settings.rs (D47).
 * Absent → derived from kind (document→"file", else "chat"). */
export const STEP_OUTPUTS: readonly string[] = ["chat", "file", "html"];

/** Generative steps auto-advance on success; only terminal `chat` waits. */
export function isGenerative(kind: string): boolean {
  return kind !== "chat";
}

// ── plan(프로그램 변경 계획) 기본 플로우: 소스코드 분석 → 컨플루언스 탐색 →
//    계획 생성 → 변경영향분석서 → 테스트 계획서 → 마무리 대화 ─────────────────────
// The requirements options are a pre-workflow phase (options.ts); the steps
// below start on the options-submit turn and auto-advance through the
// generative (search/document) steps.

// ── 기반 3단계 (foundation) 기본 지시문 ─────────────────────────────────────────
// 모든 카테고리 워크플로우 앞에 고정(pinned)으로 프리펜드되는 필수 단계들.
// rag/knowledge 단계의 실제 컨텍스트(검색 발췌·지식 본문)는 ChatPanel의
// preflight가 지시문 아래에 첨부한다(lib/foundation.ts).

const CODEBASE_STEP = `[시스템 지시: 코드베이스 분석 단계]
프롬프트에 명시된 "분석 대상 코드베이스 폴더"(절대경로)를 **읽기 전용으로 탐색**하고, 결과를 분석서 파일로 작성하세요.
- **모든 파일 탐색·검색·읽기를 그 코드베이스 폴더의 절대경로에서 시작하세요.** 현재 작업 디렉터리(작업 폴더)에서
  소스를 찾지 마세요 — 작업 폴더는 산출물 저장 전용이며 분석 대상이 아닙니다.
- 코드베이스 폴더의 파일은 절대 수정·생성·삭제하지 마세요. 읽기와 검색만 합니다.
- 분석서는 코드베이스가 아니라 **작업 폴더**에 \`docs/codebase-analysis.md\`로 저장하세요
  (프롬프트에 명시된 작업 폴더 절대경로 기준으로 쓰고, 없으면 docs 폴더도 생성).
- 분석서에는 다음을 포함하세요: 프로젝트 구조 개요 / 진입점과 핵심 모듈 / 요청과 관련된
  영역의 위치와 의존 관계(mermaid 다이어그램) / 미확인 사항.
- 아직 계획을 세우지 마세요. 이번 턴은 "코드베이스 파악과 기록"만 합니다.
- 파일 저장 후, 핵심 구조를 한두 문장으로만 보고하세요.`;

const RAG_STEP = `[시스템 지시: 사내 문서 RAG 검색 단계]
아래에 첨부된 사내 문서(Confluence) 발췌는 사전 임베딩된 지식베이스에서 검색된 결과입니다.
- 발췌를 근거로 이번 요청과 관련된 내용을 정리하고, 각 항목에 출처(제목/URL)를 인용하세요.
- 발췌에 없는 내용을 지어내지 마세요. 부족한 부분은 "추가 확인 필요"로 명시하세요.
- 아직 계획서를 작성하지 마세요. 이번 턴은 "문서 근거 정리"만 합니다.`;

const KNOWLEDGE_STEP = `[시스템 지시: 지식 베이스 반영 단계]
아래에 첨부된 사내 지식 항목들은 과거 작업 방식(상황·참조 테이블·접근 방법)의 기록입니다.
- 이번 요청에 적용되는 항목을 골라, 무엇을 어떻게 반영할지 정리하세요.
- 이후 계획 수립 단계에서 이 지식을 제약·관례로 준수하세요.
- 적용할 항목이 없으면 없다고 보고하세요.`;

const SOURCE_ANALYSIS_STEP = `[시스템 지시: 소스코드 분석 단계]
확정된 요구사항을 바탕으로 소스코드를 분석하고, 결과를 **분석서 파일로 작성**하세요.
- 분석 대상은 프롬프트에 "분석 대상 코드베이스 폴더"가 명시되어 있으면 **그 폴더**(읽기 전용),
  없으면 작업 폴더의 소스입니다.
- 파일 검색·읽기 도구로 관련 소스를 실제로 확인한 뒤, **작업 폴더**에 \`docs/source-analysis.md\`
  파일을 만들어(없으면 docs 폴더도 생성) 파일 쓰기 도구로 저장하세요.
- 분석서에는 다음을 포함하세요: 프로젝트 구조 개요 / 변경 대상 모듈과 의존 관계(mermaid 다이어그램) /
  주요 호출 흐름 / 미확인 사항·추가로 필요한 정보.
- 아직 코드를 수정하거나 계획을 세우지 마세요. 이번 턴은 "분석과 기록"만 합니다.
- 파일 저장 후, 핵심 발견 사항을 한두 문장으로만 보고하세요.`;

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

/** The mandatory foundation trio, in canonical order (D44). These are the
 * defaults merged/pinned by `coerceSteps`; the Flows editor shows them as
 * non-deletable cards whose instruction/skills/file stay editable. */
export const DEFAULT_FOUNDATION_STEPS: StepDef[] = [
  {
    id: "codebase-analysis",
    name: "코드베이스 분석",
    kind: "codebase",
    instruction: CODEBASE_STEP,
    file: "docs/codebase-analysis.md",
    skillIds: ["codebase-explore"],
  },
  {
    id: "rag-search",
    name: "사내 문서 RAG 검색",
    kind: "rag",
    instruction: RAG_STEP,
    skillIds: [],
  },
  {
    id: "knowledge",
    name: "지식 베이스 반영",
    kind: "knowledge",
    instruction: KNOWLEDGE_STEP,
    skillIds: [],
  },
];

/** Built-in default workflows (also the sample content in the Flows view).
 * plan's foundation trio is listed explicitly so the editor shows it; other
 * categories opt in via the Flows toggle (their stored workflow then carries
 * the trio — presence IS the flag). The old `confluence` search step was
 * superseded by the `rag` foundation step (user-saved workflows keep theirs). */
export const DEFAULT_WORKFLOWS: Record<Category, StepDef[]> = {
  plan: [
    ...DEFAULT_FOUNDATION_STEPS,
    {
      id: "source-analysis",
      name: "소스코드 분석",
      kind: "document",
      instruction: SOURCE_ANALYSIS_STEP,
      file: "docs/source-analysis.md",
      skillIds: ["source-analysis"],
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
 * with a missing id or unknown kind, pin the foundation trio at the front (in
 * canonical order, merging user edits over defaults and filling missing ones)
 * when the foundation phase applies, and guarantee a terminal `chat` step (the
 * runtime never trusts persisted data, even though saves are validated). */
export function coerceSteps(steps: StepDef[], opts?: { foundation?: boolean }): StepDef[] {
  const valid = (steps ?? []).filter(
    (s) =>
      !!s &&
      typeof s.id === "string" &&
      s.id.trim() !== "" &&
      STEP_KINDS.includes(s.kind) &&
      Array.isArray(s.skillIds),
  );
  // The foundation phase is mandatory when asked for (plan) or when the stored
  // workflow already carries any foundation step (opt-in categories) —
  // all-or-nothing: missing members are filled from the defaults.
  const stored = valid.filter((s) => FOUNDATION_KINDS.includes(s.kind));
  const rest = valid.filter((s) => !FOUNDATION_KINDS.includes(s.kind));
  const out =
    opts?.foundation || stored.length > 0
      ? [
          ...FOUNDATION_KINDS.map(
            (kind) =>
              stored.find((s) => s.kind === kind) ??
              DEFAULT_FOUNDATION_STEPS.find((d) => d.kind === kind)!,
          ),
          ...rest,
        ]
      : rest;
  if (out.length === 0 || out[out.length - 1].kind !== "chat") {
    out.push({ id: "chat-terminal", name: "대화", kind: "chat", instruction: "", skillIds: [] });
  }
  return out;
}

/** Whether the foundation pre-phase applies to a category: always for `plan`;
 * for the rest, when the stored workflow carries a foundation step (the Flows
 * toggle inserts/removes the trio — presence IS the flag, D44). */
export function foundationEnabled(category: Category, settings: Settings | null): boolean {
  if (category === "plan") return true;
  const stored = settings?.workflows?.[category];
  return !!stored?.some((s) => FOUNDATION_KINDS.includes(s?.kind));
}

/** Derive a step's effective output mode (D47). */
export function stepOutput(step: StepDef): string {
  if (step.output && STEP_OUTPUTS.includes(step.output)) return step.output;
  return step.kind === "document" ? "file" : "chat";
}

function htmlFileFor(step: StepDef): string {
  const file = step.file?.trim();
  if (!file) return `docs/${step.id}.html`;
  return file.replace(/\.(md|markdown|txt)$/i, "") + ".html";
}

/** Runtime expansion of output modes (D47): a generative step with
 * `output:"html"` gets a synthetic render sub-step right after it — a
 * `document` turn carrying the built-in `html-render` skill that regenerates
 * the artifact as a standalone pretty .html (opened by the canvas like any
 * document file). `output:"chat"` drops the file at runtime. Stored settings
 * keep only `output`; the editor never sees the synthetic steps. */
export function expandOutputSteps(steps: StepDef[]): StepDef[] {
  const out: StepDef[] = [];
  for (const step of steps) {
    const output = stepOutput(step);
    if (output === "chat" && step.kind !== "chat") {
      out.push({ ...step, file: null });
      continue;
    }
    out.push(step);
    if (output === "html" && isGenerative(step.kind)) {
      const htmlFile = htmlFileFor(step);
      out.push({
        id: `${step.id}-html`,
        name: `${step.name} HTML 변환`,
        kind: "document",
        instruction: `[시스템 지시: HTML 변환 단계]
직전 단계의 산출물(${step.file?.trim() || "직전 턴의 결과"})을 근거로, 내용을 보기 좋게 재구성한
**자립형 HTML 문서**를 \`${htmlFile}\`로 저장하세요(없으면 폴더도 생성).
- 내용을 새로 만들지 말고 산출물의 내용을 시각적으로 재표현만 하세요.
- 파일 저장 후 한 문장으로만 보고하세요.`,
        file: htmlFile,
        skillIds: ["html-render"],
      });
    }
  }
  return out;
}

/** The effective steps for a category (user override else default; foundation
 * trio pinned when applicable; ≥1 step, always ending in a terminal chat).
 * NOTE: this is the *editable* shape (Flows draft seed). The orchestrator
 * additionally applies {@link expandOutputSteps} for the runtime sequence. */
export function workflowFor(category: Category, settings: Settings | null): StepDef[] {
  const raw = settings?.workflows?.[category] ?? DEFAULT_WORKFLOWS[category] ?? DEFAULT_WORKFLOWS.plan;
  return coerceSteps(raw, { foundation: foundationEnabled(category, settings) });
}

/** The runtime step sequence the ChatPanel orchestrator executes. */
export function runtimeWorkflowFor(category: Category, settings: Settings | null): StepDef[] {
  return expandOutputSteps(workflowFor(category, settings));
}

/** Derived progress note for an (auto-)starting step, e.g. "3/6단계 · 계획 생성 중…". */
export function progressLabel(index: number, steps: StepDef[]): string {
  const name = steps[index]?.name?.trim() || steps[index]?.id || "";
  return `${index + 1}/${steps.length}단계 · ${name} 중…`;
}

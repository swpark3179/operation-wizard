// Built-in code-analysis method (frontend-only, D86). Like the prompt optimizer
// (promptCraft.ts, D78) this is hardcoded and injected at RUNTIME — outside the
// user-editable skill registry — so it applies even to users who saved a
// full-replace skill/workflow override (D39). It ports the Trail-Code/"graft"
// reading discipline (github.com/trailhq/Trail-Code) into agent instructions:
// skeleton (signatures) first, hubs & entry points first, callers/callees +
// blast radius, and a structured Learnings record that later steps reuse
// instead of re-exploring the codebase.
//
// The one part the app parses is the Learnings contract (`## Learnings for Next
// Steps` + three labels) — see learnings.ts. The document's other sections stay
// in the (editable) step instructions.

import type { StepDef } from "./types";
import { isGenerative } from "./workflow";

export type CodeAnalysisMode = "orient" | "focus";

/** The heading the Learnings contract requires (parsed by `extractLearnings`). */
export const LEARNINGS_HEADING = "Learnings for Next Steps";

/** Which code-analysis role a step plays: a foundation `codebase` step orients
 * (broad map of the repository); the plan `source-analysis` step (by id or by
 * carrying the `source-analysis` skill) focuses on the change targets. Other
 * steps get no method (they receive the hand-off context instead). */
export function codeAnalysisMode(step: StepDef | null | undefined): CodeAnalysisMode | null {
  if (!step) return null;
  if (step.kind === "codebase") return "orient";
  if (
    isGenerative(step.kind) &&
    (step.id === "source-analysis" || step.skillIds.includes("source-analysis"))
  ) {
    return "focus";
  }
  return null;
}

/** Synthetic skill id for session-agent dedupe (`injectedSkillIdsRef`). The
 * `__builtin:` prefix is rejected for user skills by settings.rs. */
export function codeMethodSkillId(mode: CodeAnalysisMode): string {
  return `__builtin:code-method:${mode}`;
}

/** Agent-specific reading tactics. The search tools differ: claude/gemini have
 * regex search + ranged reads, codex runs shell commands, FabriX's
 * `search_text` is a LITERAL substring search (D84). */
function tacticsFor(agentId: string): string {
  switch (agentId) {
    case "claude":
      return `- Glob으로 폴더 구조와 파일 분포를 먼저 보고, Grep(정규식, 파일명·줄번호 출력)으로 **선언부만** 모으세요.
  예: \`@(Rest)?Controller|@RequestMapping|@(Get|Post|Put|Delete)Mapping\`, \`^\\s*(public|protected)\\s[^=;]*\\(\`,
  \`<mapper namespace=|<(select|insert|update|delete) id=\`, \`CREATE (OR REPLACE )?(TABLE|VIEW|PROCEDURE|FUNCTION|PACKAGE)\`.
- Read는 offset/limit로 필요한 줄 범위만 읽으세요. 파일 전체 읽기는 짧은 설정 파일에만 씁니다.`;
    case "fabrix":
      return `- 읽기 전용 도구를 쓰세요. 소스 파일은 본문 대신 **\`outline_file\`(선언부·어노테이션·줄 범위, 매퍼 XML은 문장 id와
  참조 테이블 CRUD)** 을 먼저 보고, 필요한 범위만 \`read_file\`의 start_line/max_lines로 읽습니다.
- \`search_text\`는 정규식이 아닌 **리터럴(부분 문자열) 검색**입니다 — \`@RestController\`, \`@RequestMapping\`,
  \`<mapper namespace=\`, \`CREATE OR REPLACE\` 같은 리터럴 앵커로 위치를 찾고, \`find_files\`로 *Controller.java·
  *Mapper.xml 등의 분포를 봅니다.`;
    case "codex":
      return `- 검색 명령(rg -n / grep -n / Select-String)으로 선언부 패턴(어노테이션·public 메서드·mapper id·CREATE 문)만 먼저 모으고,
  파일은 필요한 줄 범위만 출력해 읽으세요(전체 cat 금지).`;
    default:
      return `- 검색 기능으로 선언부 패턴(어노테이션·public 메서드·mapper id·CREATE 문)만 먼저 모으고,
  파일은 필요한 줄 범위만 읽으세요.`;
  }
}

const LEARNINGS_CONTRACT = `[필수 — Learnings 기록] 산출 문서의 **맨 끝**에 반드시 아래 형식의 섹션을 넣으세요. 다음 단계는
코드를 다시 탐색하지 않고 이 섹션을 재활용합니다(앱이 자동으로 다음 단계에 전달합니다).
## ${LEARNINGS_HEADING}
- [Architecture Pattern]: (예: 레이어드 — Controller → Service → DAO/Mapper → DB)
- [Core Flow]: (데이터가 유입되어 처리되는 핵심 경로 3~5단계. 각 단계에 \`코드베이스 상대경로\`를 백틱으로 인용)
- [Key Invariants/Decisions]: (깨면 안 되는 규칙·전제·설계 의도·제약 — 확인된 것만)`;

/** The built-in method text injected on an armed code-analysis turn. */
export function codeMethodBody(mode: CodeAnalysisMode, agentId: string): string {
  if (mode === "orient") {
    return `[시스템 스킬: 코드 분석 방법론 (내장)]
당신은 대규모 저장소의 구조를 빠르고 정확하게 파악하는 시니어 소프트웨어 아키텍트입니다. 파일 본문을 통째로 읽어
토큰을 낭비하지 말고, 구조와 의존성을 먼저 파악한 뒤 필요한 세부 구현만 선택적으로 확인하세요.
1. 스켈레톤 우선: 클래스·인터페이스·메서드의 선언부(시그니처)·입출력 타입·어노테이션·Javadoc을 먼저 모아 API 뼈대를
   세웁니다. 메서드 본문은 로직 확인이 꼭 필요한 핵심 심볼에 한해서만 읽습니다.
2. 허브·진입점 우선: 빌드/설정 파일(pom.xml·build.gradle·web.xml 등)로 기술 스택을 확인하고, 진입점(컨트롤러·라우터·
   서블릿·배치·main)과 가장 많이 참조되는 중심 모듈을 먼저 찾습니다. 저장소를 평면적으로 훑지 말고 진입점에서 데이터
   흐름(Controller → Service → DAO/Mapper → SQL → 테이블)을 따라가며 범위를 좁힙니다.
3. 호출 관계·영향 범위: 핵심 심볼마다 누가 호출하는가(Callers)와 무엇을 호출하는가(Callees)를 정리하고, 이번 요청과
   관련된 변경이 퍼질 범위(Blast Radius)를 예측해 핵심 파일 목록을 먼저 확정합니다.
4. 사실만 기록: 모든 모듈·함수에 실제 파일 경로(코드베이스 기준 상대경로)를 붙이고, 확인하지 못한 내용은
   "미확인"/"추정"으로 구분합니다.
[탐색 요령]
${tacticsFor(agentId)}
${LEARNINGS_CONTRACT}`;
  }
  return `[시스템 스킬: 코드 분석 방법론 — 변경 대상 심층 분석 (내장)]
앞 단계의 코드베이스 분석서와 Learnings(아래 [앞 단계 …] 컨텍스트에 경로·발췌가 첨부됨)를 출발점으로 삼으세요. 이미
파악된 구조를 다시 탐색하지 말고, 이번 요구사항의 변경 대상에만 집중합니다.
1. 변경 대상 확정: 요구사항이 건드리는 심볼(클래스·메서드·Mapper 문장·테이블)을 정하고, 각 심볼의 시그니처와
   입출력 규격(Key Interfaces)을 정리합니다.
2. 호출 관계: 대상마다 1단계 Callers(검색으로 실제 호출 지점 확인)와 Callees를 표로 정리합니다.
3. 영향 범위(Blast Radius): 변경이 전파되는 파일 목록(핵심 파일 목록)을 확정하고 위험도를 표시합니다.
4. 본문은 대상 심볼과 직접 호출 지점만 읽습니다. 근거가 부족할 때만 해당 심볼을 선택적으로 더 읽습니다.
[탐색 요령]
${tacticsFor(agentId)}
${LEARNINGS_CONTRACT}
(이번 단계의 Learnings는 앞 단계 내용을 반복하지 말고, 변경 대상 관점에서 새로 확인한 흐름·제약을 기록하세요.)`;
}

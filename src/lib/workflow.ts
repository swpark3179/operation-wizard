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

/** Which foundation kinds a category pins when its foundation phase is on, in
 * canonical order (D63). Most categories use the full trio; `guide` is
 * Confluence/knowledge-centric and deliberately omits `codebase` (no mandatory
 * codebase-folder pick). `coerceSteps` pins exactly these when foundation is
 * on; `optionsFor` prepends the folder question only when `codebase` is among
 * the resolved steps. */
export const CATEGORY_FOUNDATION: Record<Category, readonly string[]> = {
  plan: FOUNDATION_KINDS,
  query: FOUNDATION_KINDS,
  change: FOUNDATION_KINDS,
  guide: ["rag", "knowledge"],
};

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

// ── query(데이터 조회) 단계별 기본 지시문 ────────────────────────────────────────
// 데이터 조회 카테고리의 기반 3단계는 "참조 SQL·테이블 정보 탐색"에 맞춘 맞춤 지시문을
// 쓴다(coerceSteps가 kind별로 stored/기본 항목을 병합할 때 이 지시문이 채택됨, D61).

const QUERY_CODEBASE_STEP = `[시스템 지시: 참조 SQL·테이블 사용처 탐색 단계]
프롬프트에 명시된 "분석 대상 코드베이스 폴더"(절대경로)를 **읽기 전용으로 탐색**해, 이번 조회 요청과
관련된 **참조할 만한 기존 SQL과 테이블 사용처**를 찾고 결과를 파일로 작성하세요.
- **모든 탐색·검색·읽기를 그 코드베이스 폴더의 절대경로에서 시작하세요.** 작업 폴더는 산출물 저장 전용이며
  분석 대상이 아닙니다. 코드베이스 폴더의 파일은 절대 수정·생성·삭제하지 마세요(읽기·검색만).
- 요청/요구사항에 언급된 테이블명·컬럼명·업무 용어로 검색하고, SQL 파일·DAO/매퍼(XML)·리포지토리·
  리포트·배치에서 유사한 조회/집계 패턴을 찾습니다.
- 결과는 **작업 폴더**에 \`docs/query-references.md\`로 저장하세요(없으면 docs 폴더도 생성). 각 후보는
  실제 파일 경로 / 무엇을 조회하는지 / 이번 요청과 유사한 이유를 함께 표기하고, 핵심 SQL 스니펫을 인용하세요.
- 아직 새 SQL을 작성하지 마세요. 이번 턴은 "참조 후보 탐색과 기록"만 합니다.
- 파일 저장 후, 가장 유사한 참조 한두 건만 한 문장으로 보고하세요.`;

const QUERY_RAG_STEP = `[시스템 지시: 사내 문서 RAG 검색 단계]
아래에 첨부된 사내 문서(Confluence) 발췌는 사전 임베딩된 지식베이스에서 검색된 결과입니다.
- 이번 조회의 **산출/집계 기준**(포함·제외 규칙, 기준일 정의, 표준코드 값의 의미 등)과 관련된 내용을
  발췌에서 정리하고, 각 항목에 출처(제목/URL)를 인용하세요.
- 발췌에 없는 내용을 지어내지 마세요. 부족한 부분은 "추가 확인 필요"로 명시하세요.
- 아직 SQL을 작성하지 마세요. 이번 턴은 "조회 기준 근거 정리"만 합니다.`;

const QUERY_KNOWLEDGE_STEP = `[시스템 지시: 지식 베이스 반영 단계]
아래에 첨부된 사내 지식 항목들은 과거 조회 방식(용어→테이블 매핑·표준코드·참조 테이블·조회 패턴)의 기록입니다.
- 이번 요청에 적용되는 항목(예: 업무 용어가 가리키는 실제 테이블, 표준코드 정의, 과거 유사 조회)을 골라
  무엇을 어떻게 반영할지 정리하세요.
- 이후 ERD·SQL 단계에서 이 매핑과 관례를 제약으로 준수하세요.
- 적용할 항목이 없으면 없다고 보고하세요.`;

const QUERY_TABLE_INFO_STEP = `[시스템 지시: 테이블 정보·ERD 정리 단계]
지금까지의 요구사항·참조 SQL·문서·지식을 종합해, 조회 대상 테이블들의 정보를 **파일로 작성**하세요.
- 작업 폴더에 \`docs/table-info.md\` 파일을 만들어(없으면 docs 폴더도 생성) 파일 쓰기 도구로 저장하세요.
- 문서에는 다음을 포함하세요:
  - **테이블 상관관계 ERD**: \`\`\`mermaid\`\`\` \`erDiagram\` (엔티티의 PK/주요 컬럼 + 관계선에 조인 키·카디널리티).
  - **테이블 마스터 정보 표**(테이블 / 설명 / 저장소·DBMS / 건수·갱신 / 담당 — 확인 안 되면 "미확인").
  - **관련 프로그램 표**(프로그램 ID·명 / 참조 테이블 / 참조 유형 R·U·D).
  - 동일·유사 집계를 이미 제공하는 화면/리포트가 있으면 맨 앞에 안내.
- 근거 없는 컬럼·관계·값을 지어내지 마세요(불확실은 "미확인"). 파일 저장 후 핵심만 한두 문장으로 보고하세요.`;

const QUERY_SQL_STEP = `[시스템 지시: 참고 SQL 작성 단계]
확정된 요구사항과 앞 단계의 ERD·테이블 정보를 근거로 **참고용 조회 SQL을 파일로 작성**하세요.
- 작업 폴더에 \`docs/query-sql.md\` 파일을 만들어 파일 쓰기 도구로 저장하세요.
- 문서 구성: ① 이 SQL이 **참고용 초안이며 실행 결과를 보장하지 않는다**는 경고, ② \`\`\`sql\`\`\` 펜스로 감싼
  SELECT 문(맨 위에 [참고용]/참조 출처/기준을 머리 주석으로), ③ **검토 포인트**(포함·제외 가정, NULL·경계값,
  중복 가능성, 대량 조회 범위 한정), ④ 필요 시 변형 쿼리(예: 휴직자 포함 버전).
- **읽기 전용 SELECT만** 작성하고, ERD에서 확인된 테이블·컬럼만 사용하세요(미확인은 주석으로 표시).
- 파일 저장 후, 무엇을 조회하는 SQL인지 한두 문장으로만 보고하세요.`;

// ── change(데이터 변경·권한) 단계별 기본 지시문 ──────────────────────────────────
// 데이터 변경·권한 카테고리도 query처럼 기반 3단계를 변경 관점 지시문으로 쓴다.
// 코드베이스 분석·ERD 정리는 조회와 거의 동일하고, 마지막 산출물이 운영 반영용
// DC Manager 신청양식(HTML)이라는 점에서 차이가 난다(D62).

const CHANGE_CODEBASE_STEP = `[시스템 지시: 변경 대상·영향 탐색 단계]
프롬프트에 명시된 "분석 대상 코드베이스 폴더"(절대경로)를 **읽기 전용으로 탐색**해, 이번 변경 대상 객체와
그 변경이 미치는 영향(참조·수정 지점)을 찾고 결과를 파일로 작성하세요.
- **모든 탐색·검색·읽기를 그 코드베이스 폴더의 절대경로에서 시작하세요.** 작업 폴더는 산출물 저장 전용이며
  분석 대상이 아닙니다. 코드베이스 폴더의 파일은 절대 수정·생성·삭제하지 마세요(읽기·검색만).
- 대상 테이블/객체의 정의(DDL·모델·매퍼 매핑)와, 그 테이블을 삽입(C)·조회(R)·수정(U)·삭제(D)하는
  SQL·DAO/매퍼·리포지토리·배치를 찾습니다. 유사한 기존 변경/마이그레이션 스크립트도 참고로 수집합니다.
- 결과는 **작업 폴더**에 \`docs/change-references.md\`로 저장하세요(없으면 docs 폴더도 생성). 각 항목은
  실제 파일 경로 / 무엇을 하는지 / 참조 유형(C·R·U·D) / 이번 변경과의 관계를 함께 표기하세요.
- 아직 신청양식이나 변경 SQL을 작성하지 마세요. 이번 턴은 "대상·영향 탐색과 기록"만 합니다.
- 파일 저장 후, 영향이 큰 지점 한두 건만 한 문장으로 보고하세요.`;

const CHANGE_RAG_STEP = `[시스템 지시: 사내 문서 RAG 검색 단계]
아래에 첨부된 사내 문서(Confluence) 발췌는 사전 임베딩된 지식베이스에서 검색된 결과입니다.
- 이번 변경과 관련된 **표준 절차·승인 규칙·명명 규칙·과거 변경 이력**을 발췌에서 정리하고, 각 항목에
  출처(제목/URL)를 인용하세요.
- 발췌에 없는 내용을 지어내지 마세요. 부족한 부분은 "추가 확인 필요"로 명시하세요.
- 아직 신청양식을 작성하지 마세요. 이번 턴은 "변경 기준·절차 근거 정리"만 합니다.`;

const CHANGE_KNOWLEDGE_STEP = `[시스템 지시: 지식 베이스 반영 단계]
아래에 첨부된 사내 지식 항목들은 과거 변경 작업 방식(대상 테이블·표준 절차·승인 흐름·주의사항)의 기록입니다.
- 이번 요청에 적용되는 항목(예: 대상 테이블의 특성·주의점, 과거 유사 변경, DC Manager 신청 관례)을 골라
  무엇을 어떻게 반영할지 정리하세요.
- 이후 ERD·신청양식 단계에서 이 관례를 제약으로 준수하세요.
- 적용할 항목이 없으면 없다고 보고하세요.`;

const CHANGE_TABLE_INFO_STEP = `[시스템 지시: 테이블 정보·ERD 정리 단계]
지금까지의 요구사항·영향 탐색·문서·지식을 종합해, 변경 대상 및 관련 테이블 정보를 **파일로 작성**하세요.
- 작업 폴더에 \`docs/change-table-info.md\` 파일을 만들어(없으면 docs 폴더도 생성) 파일 쓰기 도구로 저장하세요.
- 문서에는 다음을 포함하세요:
  - **테이블 상관관계 ERD**: \`\`\`mermaid\`\`\` \`erDiagram\` (엔티티의 PK/주요 컬럼 + 관계선에 조인 키·카디널리티).
  - **대상 테이블 정보 표**(테이블 / 설명 / 저장소·DBMS / 주요 컬럼·제약 / 담당 — 확인 안 되면 "미확인").
  - **영향 프로그램 표**(프로그램 ID·명 / 참조 테이블 / 참조 유형 C·R·U·D / 변경 시 후속 조치).
- 근거 없는 컬럼·관계·값을 지어내지 마세요(불확실은 "미확인"). 파일 저장 후 핵심만 한두 문장으로 보고하세요.`;

const CHANGE_DC_MANAGER_STEP = `[시스템 지시: DC Manager 신청양식 생성 단계]
요구사항에서 고른 **변경 종류**에 맞춰, 운영서버 반영용 **DC Manager 신청양식을 HTML 파일로 작성**하세요.
- 작업 폴더에 \`docs/dc-manager-form.html\` 파일을 만들어(없으면 docs 폴더도 생성) 파일 쓰기 도구로 저장하세요.
- 앞 단계의 ERD·영향 정보에서 **확인된 테이블·컬럼·프로그램만** 사용하고, 확인되지 않은 값은 "[확인 필요]"로
  표시하세요. 스키마를 지어내지 마세요.
- 스킬 지시대로 **인라인 style 속성 + 시맨틱 표** 중심의 자립형 HTML을 만드세요(붙여넣기 시 서식 유지).
  본문은 앱의 "본문 복사" 버튼으로 그대로 복사되어 DC Manager 편집기에 붙습니다.
- 파일 저장 후, 어떤 종류의 신청서를 만들었는지 한 문장으로만 보고하세요.`;

// ── guide(운영 가이드 생성) 단계별 기본 지시문 ──────────────────────────────────
// 운영 가이드 카테고리의 강점은 사내 문서(Confluence)/지식을 시각적으로 정리해 보여주고,
// 사용자가 업무를 어떤 절차로 수행하면 되는지 재현 가능한 가이드를 제공하는 것이다(D63).
// 코드베이스 분석 단계는 두지 않고 rag + knowledge 두 기반 단계만 사용한다.

const GUIDE_RAG_STEP = `[시스템 지시: 사내 문서 RAG 검색 단계]
아래에 첨부된 사내 문서(Confluence) 발췌는 사전 임베딩된 지식베이스에서 검색된 결과입니다.
- 이번 운영 작업과 관련된 **표준 절차·런북·설정·주의사항·과거 사례**를 발췌에서 정리하고, 각 항목에
  출처(제목/URL)를 인용하세요.
- 발췌에 없는 내용을 지어내지 마세요. 부족한 부분은 "추가 확인 필요"로 명시하세요.
- 아직 가이드 문서를 작성하지 마세요. 이번 턴은 "문서 근거 정리"만 합니다.`;

const GUIDE_KNOWLEDGE_STEP = `[시스템 지시: 지식 베이스 반영 단계]
아래에 첨부된 사내 지식 항목들은 과거 운영 작업 방식(절차·주의점·관례)의 기록입니다.
- 이번 가이드에 적용되는 항목(예: 표준 절차, 자주 겪는 실패와 대응, 사내 관례)을 골라 무엇을 어떻게
  반영할지 정리하세요.
- 이후 가이드 작성 단계에서 이 지식을 절차·주의사항으로 반영하세요.
- 적용할 항목이 없으면 없다고 보고하세요.`;

const GUIDE_DOC_STEP = `[시스템 지시: 운영 가이드 작성 단계]
지금까지의 요구사항·사내 문서(RAG)·지식을 종합해, 사용자가 그대로 따라 할 수 있는 **운영 가이드를
파일로 작성**하세요.
- 작업 폴더에 \`docs/operation-guide.md\` 파일을 만들어(없으면 docs 폴더도 생성) 파일 쓰기 도구로 저장하세요.
- 스킬 지시대로 **대상 독자 수준에 맞춘 재현 가능한 절차**로 구성하세요: 개요·목적 → 전제 조건 →
  단계별 행동(명령/화면/입력) → 각 단계의 검증 방법 → 실패 시 롤백/원복.
- **전체 흐름을 \`\`\`mermaid\`\`\` 다이어그램(flowchart)으로** 맨 앞에 넣어 절차를 한눈에 보이게 하세요.
- 맨 뒤에 **참고 문서 섹션**을 두고, 근거로 삼은 사내 문서(Confluence)를 제목/URL과 함께 인용하세요.
- 발췌·지식에 없는 내용을 지어내지 마세요(불확실은 "추가 확인 필요"로 표기).
- 파일 저장 후, 어떤 작업의 가이드를 만들었는지 한두 문장으로만 보고하세요.`;

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
    {
      id: "rag-search",
      name: "사내 문서 RAG 검색",
      kind: "rag",
      instruction: GUIDE_RAG_STEP,
      skillIds: [],
    },
    {
      id: "knowledge",
      name: "지식 베이스 반영",
      kind: "knowledge",
      instruction: GUIDE_KNOWLEDGE_STEP,
      skillIds: [],
    },
    {
      // document + output:"html" → expandOutputSteps가 뒤에 html-render 서브스텝을
      // 붙여 docs/operation-guide.html을 자동 생성한다(D47/D63).
      id: "guide-doc",
      name: "운영 가이드 작성",
      kind: "document",
      instruction: GUIDE_DOC_STEP,
      file: "docs/operation-guide.md",
      output: "html",
      skillIds: ["guide-author"],
    },
    { id: "chat", name: "마무리 대화", kind: "chat", instruction: "", skillIds: ["guide-author"] },
  ],
  query: [
    {
      id: "query-codebase",
      name: "참조 SQL·테이블 사용처 탐색",
      kind: "codebase",
      instruction: QUERY_CODEBASE_STEP,
      file: "docs/query-references.md",
      // codebase 기본 output은 "chat"(파일 스트립)이므로, 참조 목록을 산출물로
      // 남기려면 "file"을 명시한다(D61).
      output: "file",
      skillIds: ["reference-sql-explore"],
    },
    {
      id: "rag-search",
      name: "사내 문서 RAG 검색",
      kind: "rag",
      instruction: QUERY_RAG_STEP,
      skillIds: [],
    },
    {
      id: "knowledge",
      name: "지식 베이스 반영",
      kind: "knowledge",
      instruction: QUERY_KNOWLEDGE_STEP,
      skillIds: [],
    },
    {
      id: "table-info",
      name: "테이블 정보·ERD 정리",
      kind: "document",
      instruction: QUERY_TABLE_INFO_STEP,
      file: "docs/table-info.md",
      skillIds: ["table-erd"],
    },
    {
      id: "sql-draft",
      name: "참고 SQL 작성",
      kind: "document",
      instruction: QUERY_SQL_STEP,
      file: "docs/query-sql.md",
      skillIds: ["sql-draft", "query-safe"],
    },
    { id: "chat", name: "마무리 대화", kind: "chat", instruction: "", skillIds: ["query-safe"] },
  ],
  change: [
    {
      id: "change-codebase",
      name: "변경 대상·영향 탐색",
      kind: "codebase",
      instruction: CHANGE_CODEBASE_STEP,
      file: "docs/change-references.md",
      // codebase 기본 output은 "chat"(파일 스트립)이므로, 참조·영향 목록을
      // 산출물로 남기려면 "file"을 명시한다(D61/D62 패턴).
      output: "file",
      skillIds: ["change-impact-explore"],
    },
    {
      id: "rag-search",
      name: "사내 문서 RAG 검색",
      kind: "rag",
      instruction: CHANGE_RAG_STEP,
      skillIds: [],
    },
    {
      id: "knowledge",
      name: "지식 베이스 반영",
      kind: "knowledge",
      instruction: CHANGE_KNOWLEDGE_STEP,
      skillIds: [],
    },
    {
      id: "change-table-info",
      name: "테이블 정보·ERD 정리",
      kind: "document",
      instruction: CHANGE_TABLE_INFO_STEP,
      file: "docs/change-table-info.md",
      skillIds: ["table-erd"],
    },
    {
      // document + .html이라 output이 "file"로 파생 → 합성 html-render substep 없이
      // 에이전트가 HTML을 직접 저술한다(붙여넣기용 폼이라 범용 렌더러 chrome를 피함, D62).
      id: "dc-manager",
      name: "DC Manager 신청양식 생성",
      kind: "document",
      instruction: CHANGE_DC_MANAGER_STEP,
      file: "docs/dc-manager-form.html",
      skillIds: ["dc-manager-form"],
    },
    { id: "chat", name: "마무리 대화", kind: "chat", instruction: "", skillIds: ["change-safe"] },
  ],
};

/** Coerce stored steps into a shape the orchestrator can trust: drop entries
 * with a missing id or unknown kind, pin the applicable foundation steps at the
 * front (in canonical order, merging user edits over defaults), and guarantee a
 * terminal `chat` step (the runtime never trusts persisted data, even though
 * saves are validated).
 *
 * Foundation selection (D44/D63):
 * - `foundationKinds` given → pin exactly those kinds (fill missing from the
 *   defaults). This is how a category forces its trio/subset (e.g. plan/query/
 *   change → full trio; guide → rag+knowledge).
 * - otherwise → pin whichever foundation kinds are *present*, as a canonical
 *   subsequence (no forced fill). Used for hand-edited/legacy drafts.
 * The legacy `foundation: true` boolean is still honored (≡ full trio). */
export function coerceSteps(
  steps: StepDef[],
  opts?: { foundation?: boolean; foundationKinds?: readonly string[] },
): StepDef[] {
  const valid = (steps ?? []).filter(
    (s) =>
      !!s &&
      typeof s.id === "string" &&
      s.id.trim() !== "" &&
      STEP_KINDS.includes(s.kind) &&
      Array.isArray(s.skillIds),
  );
  const stored = valid.filter((s) => FOUNDATION_KINDS.includes(s.kind));
  const rest = valid.filter((s) => !FOUNDATION_KINDS.includes(s.kind));
  // Which foundation kinds to pin, always in canonical (FOUNDATION_KINDS) order.
  const forced = opts?.foundationKinds ?? (opts?.foundation ? FOUNDATION_KINDS : undefined);
  const kinds = forced
    ? FOUNDATION_KINDS.filter((k) => forced.includes(k))
    : FOUNDATION_KINDS.filter((k) => stored.some((s) => s.kind === k));
  const pinned = kinds.map(
    (kind) =>
      stored.find((s) => s.kind === kind) ?? DEFAULT_FOUNDATION_STEPS.find((d) => d.kind === kind)!,
  );
  const out = [...pinned, ...rest];
  if (out.length === 0 || out[out.length - 1].kind !== "chat") {
    out.push({ id: "chat-terminal", name: "대화", kind: "chat", instruction: "", skillIds: [] });
  }
  return out;
}

/** Whether the foundation pre-phase applies to a category: always for `plan`;
 * for the rest, when the effective workflow carries a foundation step (the Flows
 * toggle inserts/removes the foundation steps — presence IS the flag, D44). */
export function foundationEnabled(category: Category, settings: Settings | null): boolean {
  if (category === "plan") return true;
  // Fall back to the built-in default so categories whose DEFAULT_WORKFLOWS
  // ship foundation steps (e.g. query D61, guide D63) enable it out of the box;
  // a user-saved override (incl. the Flows toggle turning it off) still wins.
  const steps = settings?.workflows?.[category] ?? DEFAULT_WORKFLOWS[category];
  return !!steps?.some((s) => FOUNDATION_KINDS.includes(s?.kind));
}

/** The foundation kinds a category pins when its foundation phase is on (D63):
 * `CATEGORY_FOUNDATION[category]` when enabled, else `undefined` (no forced
 * foundation → `coerceSteps` keeps only whatever is present). */
export function mandatoryFoundation(
  category: Category,
  settings: Settings | null,
): readonly string[] | undefined {
  return foundationEnabled(category, settings) ? CATEGORY_FOUNDATION[category] : undefined;
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
  return coerceSteps(raw, { foundationKinds: mandatoryFoundation(category, settings) });
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

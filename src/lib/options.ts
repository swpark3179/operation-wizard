// Static per-category option catalogs (frontend-only). When a category is
// started, its catalog is shown IMMEDIATELY as a canvas form (no agent turn) so
// the user decides the key setup options first; if the launcher carried a
// prompt, an agent "prefill" pass (see clarify.ts) pre-fills the options it can
// confidently infer and leaves the rest for the user (design D36).
//
// We reuse the ClarifyQuestion shape (single/multi/text) so the existing
// RequirementsForm renders these unchanged. An empty catalog for a category
// means "no options phase" → the flow falls back to the seed-first behavior.

import type { ClarifyQuestion } from "./clarify";
import type { Category } from "../components/workspace";
import type { Settings } from "./types";
import { workflowFor } from "./workflow";

/** The requirement question — "what do you want to do" — prepended to EVERY
 * category's form so the intent always comes first (D65). Required; when the
 * chat starts from the launcher prompt the client seeds it with that text
 * verbatim (`noPrefill` keeps the agent prefill pass from rewriting it). Its
 * answer stays in the answers wire (it feeds the prompt-optimizer skill) and
 * replaces the old "원래 요청" seed suffix on the first work turn. */
export const REQUIREMENT_QUESTION: ClarifyQuestion = {
  id: "userRequest",
  label: "무엇을 하고 싶으신가요? 원하는 작업을 자유롭게 설명해 주세요.",
  type: "text",
  required: true,
  noPrefill: true,
};

/** The mandatory codebase-folder question, prepended when the category's
 * foundation phase applies (D45). Excluded from prefill (folder type); its
 * answer is delivered structurally (WorkspaceView state → ChatPanel prop), not
 * as wire prose. */
export const CODEBASE_QUESTION: ClarifyQuestion = {
  id: "codebasePath",
  label: "분석할 코드베이스 폴더를 선택하세요.",
  type: "folder",
  required: true,
};

export const CATEGORY_OPTIONS: Record<Category, ClarifyQuestion[]> = {
  plan: [
    {
      id: "scope",
      label: "이번 작업의 유형은 무엇인가요?",
      type: "single",
      options: ["버그 수정", "신규 기능", "리팩터링", "성능 개선"],
      required: true,
    },
    {
      id: "targets",
      label: "영향을 받는 영역을 모두 선택하세요.",
      type: "multi",
      options: ["프론트엔드", "백엔드", "데이터베이스", "인프라·배포"],
    },
    {
      id: "priority",
      label: "우선순위는 어느 정도인가요?",
      type: "single",
      options: ["긴급", "보통", "낮음"],
    },
    {
      id: "constraints",
      label: "그 밖에 알아야 할 제약이나 배경이 있나요?",
      type: "text",
    },
  ],
  guide: [
    {
      id: "audience",
      label: "이 가이드의 대상 독자는 누구인가요?",
      type: "single",
      options: ["운영자", "개발자", "신규 입사자"],
      required: true,
    },
    {
      id: "guideType",
      label: "어떤 유형의 가이드인가요?",
      type: "single",
      options: ["절차·런북", "트러블슈팅", "설치·설정"],
      required: true,
    },
    {
      id: "rollback",
      label: "롤백/원복 절차를 포함할까요?",
      type: "single",
      options: ["포함", "미포함"],
    },
    {
      id: "referenceDocs",
      label: "참고할 사내 문서·Confluence 공간/키워드가 있다면 알려주세요. (선택)",
      type: "text",
    },
    {
      id: "topic",
      label: "다룰 운영 작업이나 주제를 알려주세요.",
      type: "text",
    },
  ],
  query: [
    {
      id: "dataSource",
      label: "조회할 데이터 소스는 무엇인가요?",
      type: "single",
      options: ["관계형 DB", "로그", "API·기타"],
      required: true,
    },
    {
      id: "outputForm",
      label: "원하는 결과 형태는 무엇인가요?",
      type: "single",
      options: ["표·목록", "집계·통계", "단건 확인"],
    },
    {
      id: "knownTables",
      label: "이미 알고 있는 테이블·컬럼·프로그램이 있다면 알려주세요.",
      type: "text",
    },
    {
      id: "target",
      label: "무엇을, 어떤 조건으로 조회하고 싶은가요? (대상·기간·필터)",
      type: "text",
    },
  ],
  change: [
    {
      id: "changeType",
      label: "어떤 변경 작업인가요?",
      type: "single",
      options: ["데이터 수정", "테이블 생성", "테이블 권한 부여", "스키마 변경"],
      required: true,
    },
    {
      id: "riskLevel",
      label: "변경의 영향 범위는 어느 정도인가요?",
      type: "single",
      options: ["단건", "다건·일괄", "전체"],
      required: true,
    },
    {
      id: "approval",
      label: "실행 전 승인이 필요한가요?",
      type: "single",
      options: ["필요", "불필요"],
    },
    {
      id: "knownObjects",
      label: "이미 알고 있는 대상 테이블·컬럼·객체가 있다면 알려주세요.",
      type: "text",
    },
    {
      id: "target",
      label: "변경 대상과 조건을 알려주세요.",
      type: "text",
    },
  ],
};

/** The static option catalog for a category (may be empty → no options phase). */
export function optionsFor(category: Category, settings: Settings | null): ClarifyQuestion[] {
  const base = CATEGORY_OPTIONS[category] ?? [];
  // The codebase-analysis step needs its target folder up front, so the folder
  // question gates the form only when the resolved workflow actually has a
  // `codebase` step (D63) — a Confluence/knowledge-only flow like guide has
  // none, so it shows no folder picker.
  const needsCodebase = workflowFor(category, settings).some((s) => s.kind === "codebase");
  // The requirement question always leads — what the user wants comes before
  // any setup choice (D65).
  return needsCodebase
    ? [REQUIREMENT_QUESTION, CODEBASE_QUESTION, ...base]
    : [REQUIREMENT_QUESTION, ...base];
}

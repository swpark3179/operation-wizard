# 08. 가이드 플로우 & 스킬 (다단계 대화·결과형식 강제 가이드)

이 문서는 **"초반 몇 단계의 대화를 강제하고, 이 시스템이 제공하는 스킬을 적용하며, 결과 형식을 강제"**
하려 할 때 **어디를 고쳐야 하는지**를 정리한 실무 가이드다. 배경 개념은
[07-workspace-and-runs.md](07-workspace-and-runs.md)의 "카테고리 워크플로우" 절과
[05-decisions.md](05-decisions.md) D30/D34/D36/D39/D40에 있다.

## 대전제 (실행은 클라이언트, 정의는 설정)

로컬 CLI 에이전트 스트림에는 **우리가 제어하는 표준 도구(tool-call) 채널이 없다**(D30 재확인). 따라서
모든 **실행 강제**는 **클라이언트 오케스트레이션**으로 한다: 프롬프트에 지시문을 (사용자에게 안 보이게)
주입하고, 에이전트가 낸 텍스트를 **약속된 fenced 코드블록**으로 해석한다. 파싱/프리필이 실패하면 **항상
일반 chat으로 폴백**해 대화가 깨지지 않는다.

단계·스킬의 **정의**는 D39부터 사용자가 **Flows 설정 화면**에서 등록하고 `settings.json`에 영속화된다
(`set_skills`/`set_workflow`). 코드 내 `DEFAULT_WORKFLOWS`/`DEFAULT_SKILLS`는 **폴백이자 편집 가능한
샘플 콘텐츠**다(설정에 값이 없으면 항상 기본값 적용).

## 아키텍처 3층

가이드 플로우는 세 개의 독립된 데이터/로직 층으로 구성된다.

| 층 | 정의 위치 | 역할 | 강제 시점 |
|----|------|------|-----------|
| ① 고정 선택지(옵션) | `src/lib/options.ts` (`CATEGORY_OPTIONS`, 코드 정적) + `src/lib/clarify.ts`(프리필) | 카테고리 진입 시 **즉시** 보여줄 선택지 카탈로그. 프롬프트로 시작하면 아는 값 자동 채움. **폼 대기 중 채팅 차단**(D41) | 워크플로우 **이전**(프리플로우). 폼 제출이 첫 작업 턴을 발사 |
| ② 스킬 | **Flows 설정 화면**(settings.json `skills`) ?? `src/lib/skills.ts` (`DEFAULT_SKILLS`); 조회는 `resolveSkills(settings)` | **단계에 귀속되는 지시문 묶음**(페르소나·방법·제약·산출 형식). CLI 자체 스킬과 무관 | 그 스킬을 가진 **단계가 armed된 턴**에 주입. 세션형은 대화당 1회 dedupe(D40) |
| ③ 워크플로우 단계 | **Flows 설정 화면**(settings.json `workflows[category]`) ?? `src/lib/workflow.ts` (`DEFAULT_WORKFLOWS`); 조회는 `workflowFor(category, settings)` | 옵션 제출 **이후**의 에이전트 턴들(조사/문서작성/대화). `StepDef{id,name,kind,instruction,file?,skillIds}`. 자동전진·정지 제어 | 매 턴 단계 스킬+지시문 주입 + `end`에서 `kind`로 분기 |

주입은 모두 `ChatPanel.send()`의 **wire 조립**에서 일어난다:
```
wire = [ (단계 armed면) step.skillIds의 스킬 body들(세션형은 대화당 1회),
         (단계 armed면) step.instruction,
         prompt ].join("\n\n")
```
프리필 턴만 예외로 wire = `prefillInstruction(...)`이고, **스텝 커서/스킬 주입 이력/세션/영속화를 건드리지
않는다**(격리 실행).

## 흐름 한눈에 (plan 기본값 기준)

```
카테고리 진입
  └─ optionsFor(category) 비었나?
       ├─ 아니오 → onClarify(옵션) [즉시 폼 · 채팅 차단 · '요구사항' 탭 표시]
       │            └─ seedPrompt 있으면 → 프리필 턴(숨김) → parsePrefill → onPrefill(폼 자동채움)
       │            └─ 사용자 폼 제출 → answerSubmission [채팅 차단 해제 · 탭 소멸]
       │                  └─ 첫 작업 턴: WF[0].skills + WF[0].instruction + 답변 + 원요청
       │                       └─ (plan 기본) 소스코드 분석(document) → (auto) 컨플루언스 탐색(search)
       │                          → (auto) 계획 생성(document) → (auto) 변경영향분석서(document)
       │                          → (auto) 테스트 계획서(document) → chat(정지)
       └─ 예 → seedPrompt를 첫 작업 턴으로 자동 전송(기존 동작)
```

각 생성형 단계는 자기 스킬(예: 소스코드 분석 = mermaid 다이어그램 스킬)을 그 턴에 주입하고, document
단계는 산출물 파일(`docs/*.md`)을 캔버스에 연다(마크다운+mermaid 미리보기 — D42). 세션 이어가기(로드
세션)는 이 전부를 건너뛰고 일반 chat으로 시작한다(스텝 커서 끝, 주입 없음).

## 변경 지점 쿡북 ("무엇을 하려면 어디를 고치나")

### A. 카테고리의 선택지(옵션)를 추가·수정
- `src/lib/options.ts`의 `CATEGORY_OPTIONS[category]` 배열만 편집.
- 항목은 `ClarifyQuestion`(`single`/`multi`/`text`) 스키마 그대로. `single/multi`는 `options` 필수,
  `required: true`로 필수화. 렌더는 `RequirementsForm`(accent 카드 그리드)이 자동 처리(D35).
- 빈 배열이면 그 카테고리는 옵션 단계 없이 기존 seed-first 동작.

### B. 스킬(지시문)을 작성·수정·교체
- **1차 경로(재빌드 없음)**: **Flows 설정 화면 → 스킬** 섹션에서 추가/수정/삭제 후 저장(`set_skills` —
  레지스트리 전체 교체, settings.json `skills`). "기본값으로 되돌리기"가 override를 지운다.
- **기본값(샘플) 변경**: `src/lib/skills.ts`의 `DEFAULT_SKILLS`를 수정(빌드 필요). 단, 사용자가 이미
  레지스트리를 저장했다면 그 사용자에게는 반영되지 않는다(전체 교체형 — D39).
- 스킬은 단계의 `skillIds`로 연결되고, **그 단계가 armed된 턴**에 주입된다(D40). 세션형(claude/codex)은
  같은 스킬을 대화당 1회만(이미 세션에 있음), 세션리스(gemini/aipro)는 transcript로 재노출.

### C. 초반 대화 단계를 강제하거나 새 단계를 추가
- **1차 경로(재빌드 없음)**: **Flows 설정 화면 → 카테고리 탭**에서 단계 추가/삭제/순서 변경/스킬 연결 후
  저장(`set_workflow`, settings.json `workflows[category]`). 어느 카테고리든 다단계로 확장 가능.
- **기본값(샘플) 변경**: `src/lib/workflow.ts`의 `DEFAULT_WORKFLOWS[category]`를 수정(빌드 필요).
- 단계 규칙:
  - `kind: "search"|"document"` = **생성형**: 성공 시 자동으로 다음 생성형 단계로 전진(`document`는 `file`
    지정 시 캔버스에 그 파일을 연다). 진행 노트는 `name`에서 파생("N/M단계 · <name> 중…").
  - `kind: "chat"` = **종단**: 전진 없이 사용자 입력 대기. **마지막 단계는 반드시 `chat`**(저장 검증 +
    `coerceSteps` 자동 보강).
- 커서 로직은 `ChatPanel`의 `stepIndexRef`/`stepArmedRef`/`injectedSkillIdsRef`/`inflightStepRef` + `end`
  핸들러 분기 + `autoTurn`(생성형→생성형 자동 발사). **새 kind를 추가하면** `StepKind`/`STEP_KINDS`
  (workflow.ts + settings.rs)와 `ChatPanel`의 `end` 분기, Flows 편집기의 kind 셀렉트를 함께 확장한다.
- **상호작용(사용자 확인) 단계**가 필요하면: 옵션 프리플로우(A)처럼 폼→제출로 확인점을 만들거나, 새 fenced
  파서(D)로 에이전트가 낸 질문을 폼으로 올려 멈춘 뒤 답변으로 다음 단계를 발사한다.

### D. 새로운 결과 형식을 강제(에이전트가 특정 구조로 응답하게)
- `src/lib/clarify.ts`의 **fenced-block 패턴을 복제**한다(참조 구현: `parseClarify`/`stripClarifyBlock`
  = 에이전트가 질문을 생성, `parsePrefill` = 에이전트가 고정 항목을 채움).
  1. 지시문 상수/함수 작성(에이전트에게 ` ```<tag> ` JSON 하나만 내라고 지시).
  2. `fencedBlocks`로 파싱하는 `fn(&str)->결과|null` 파서 작성(검증 실패 시 `null`/`{}`).
  3. `ChatPanel.send()`에서 해당 턴에 지시문 주입 + `inflight` 마킹, `end`에서 파서 호출 후 캔버스/상태로 반영.
- 캔버스 렌더가 필요하면 `WorkspaceView`에 상태를 리프트하고 `CanvasPanel`에 탭/뷰를 추가한다(요구사항 폼이
  대표 예).

### E. 프롬프트 자동채움(프리필) 동작 조정
- `src/lib/clarify.ts`의 `prefillInstruction`(지시문)과 `parsePrefill`(검증 규칙)만 수정.
- 프리필 턴은 `ChatPanel.send(_, {prefill:true})`로 **격리 실행**된다: 세션 id/resume 미사용(실제 세션 오염
  방지), 영속화 안 함, 스텝 커서 불변, 끝나면 프리필 메시지쌍 제거 후 `onPrefill`로 폼만 채움.

### F. 새 카테고리 추가(전체 절차)
1. `src/components/workspace.ts` — `Category` 유니온에 id 추가 + `CATEGORIES`에 카드 메타(라벨/아이콘/타일).
2. `src-tauri/src/settings.rs` — `CATEGORIES` 상수에 id 추가(**프론트와 동기화 필수** — `set_workflow`가
   이 목록으로 검증).
3. `src/lib/options.ts` — `CATEGORY_OPTIONS[새id]` 선택지(없으면 `[]`).
4. `src/lib/workflow.ts` — `DEFAULT_WORKFLOWS[새id]` 기본 단계 배열(최소 `[{kind:"chat", skillIds:[...]}]`;
   스킬이 필요하면 `DEFAULT_SKILLS`에도 추가).
5. 프론트는 자동 반영(HomeView 카테고리 카드가 `CATEGORIES`를, Flows 편집기가 카테고리 탭을 렌더).

## 주입 지점 지도 (`ChatPanel`)

- **wire 조립**: `send()` — `[step.skillIds의 스킬 body들(armed, 세션형 dedupe) → step.instruction(단계
  armed) → prompt]`. 프리필은 `prefillInstruction`. 단계/스킬은 `workflowFor(category, settings)`/
  `resolveSkills(settings)`로 해석해 **마운트 시 고정**.
- **부트**: 마운트 시 옵션 있으면 즉시 `onClarify`(채팅 차단 시작) + (seed 있으면) 프리필 턴; 없으면 seed
  자동전송.
- **첫 작업 턴**: `answerSubmission` 효과(폼 제출; 차단 해제와 같은 커밋) 또는 seed 자동전송이 발사. 여기서
  step[0]의 스킬+지시문 주입.
- **자동전진**: `autoTurn` nonce 효과가 생성형→생성형 턴을 사용자 입력 없이 발사(system 라인 =
  `progressLabel`).
- **분기/정지**: `end` 핸들러가 `prefill`/`step.kind`/취소·실패로 분기(정지 or 전진 or 폴백).
- **게이팅**: `formPending`(WorkspaceView의 `clarify` 유무)이 컴포저·`send()`를 차단(system/prefill 턴 예외,
  Stop 비차단 — D41).

## 에이전트별 한계 (degrade)

- **claude/codex(세션형)**: 스킬·단계 지시가 세션에 유지 → 가장 풍부(같은 스킬은 대화당 1회만 주입).
  프리필은 격리 세션으로 실행.
- **gemini/aipro(세션리스)**: 매 턴 transcript 재전송이라 과거 주입이 원문 그대로는 아니고 요약된 형태로만
  남는다(lossy). 단계별 주입이 반복되며 크기 증가.
- **opencode/antigravity(plain)**: 도구 스트림/파일쓰기 보장이 약해 `search`/`document` 단계가 degrade.

## 안전장치 (무한 진행/깨짐 방지)

단조 증가 커서 + 단계당 1회 arm + 세션형 스킬 dedupe(전송 실패 시 되감기) + `succeeded`에만 자동전진 +
종단 `chat`(저장 검증 `validate_steps` + 런타임 `coerceSteps` 이중 보장) + 프리필/파싱 실패 시 폴백.
취소/실패는 자동전진 중단 후 일반 대화. 단계·스킬 **정의**는 settings.json에 영속(D39), 실행 상태·대기
폼은 **transient**(저장 안 함); 생성된 파일만 실제로 남는다.

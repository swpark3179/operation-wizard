# 구현 계획: 요구사항 우선 필드 + 첫 작업 턴 '프롬프트 최적화' 내장 스킬 + '프롬프트' 캔버스 탭

> 이 문서는 **구현 전 계획서**다. 구현 담당 에이전트는 이 문서만으로 작업을 완수할 수 있도록
> 파일 경로·함수명·삽입 지점을 명시했다. 구현 완료 시 이 문서는 삭제하거나
> `docs/design/05-decisions.md`의 D65 항목으로 흡수한다.

## Context (왜 이 변경인가)

카테고리 가이드 플로우는 진입 시 고정 선택지 폼(캔버스 '요구사항' 탭)을 먼저 보여주고,
제출이 첫 작업 턴을 발사한다(D36/D41). 사용자 요구 두 가지:

1. **"뭘 하고 싶은지"가 폼에서 우선되어야 한다** — 현재 자유 텍스트 질문(constraints/topic/target)은
   각 카탈로그 맨 뒤에 있고, 카테고리 카드로 시작하면 요구사항 자체를 적을 곳이 없다.
2. **폼 제출 후, 답변들을 바탕으로 AI가 프롬프팅 기법을 활용해 "어떤 상황이고 어떤 작업을
   해야 하는지"의 최적 프롬프트를 완성해 캔버스에 보여준다** — 사용자가 쓰면서 프롬프팅
   개선법을 학습하는 교육 효과. 구현 방식 제약(사용자 명시): **프로그램 구조를 바꾸지 말고,
   기존 스킬 주입 기능에서 모든 카테고리의 첫 단계에 스킬 하나를 더 얹는 것**.

### 사용자 확정 결정

- **같은 턴 인라인**: 첫 작업 턴에 스킬 주입 → 에이전트가 작업 시작 전 ` ```prompt ` 펜스
  블록으로 최적 프롬프트를 먼저 출력 → 클라이언트가 `end`에서 파싱해 캔버스 '프롬프트' 탭에
  표시하고 같은 턴에서 작업 계속. 추가 턴/확인 게이트 없음.
- **항상 주입되는 내장 스킬**: 코드 하드코딩·런타임 전용. Flows 편집기/settings.json/백엔드
  무변경. 스킬 레지스트리를 override 저장한 사용자에게도 항상 적용 (D39 전체 교체형이라
  `DEFAULT_SKILLS`에 넣으면 override 사용자에게 미적용되는 문제 회피 — `resolveSkills`는
  내장 스킬을 merge하지 않음).
- **요구사항 필드는 맨 앞·필수·자동 채움**: 모든 카테고리 폼 최상단 필수 텍스트 필드. 홈
  프롬프트(seed)로 시작하면 그 텍스트로 클라이언트 측 자동 채움(수정 가능).

**전부 프론트엔드 전용. 백엔드(Rust)/IPC/신규 의존성 0.**

---

## 구현

### 1. 신규 모듈 `src/lib/promptCraft.ts` — 내장 스킬 본문 + ```prompt 파서/스트리퍼

`src/lib/knowledgeSave.ts`의 `summaryInstruction`/`parseSummary` 계약을 미러하되,
**전체 응답 폴백은 하지 않는다**(폴백하면 작업 응답 전체를 프롬프트로 오인). `fencedBlocks`는
`./clarify`에서 import (이미 export됨, `clarify.ts:78`).

```ts
// 내장 스킬(레지스트리 밖, 항상 주입 — D65). Flows에 노출되지 않는다.
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

export const PROMPT_NOTE = "(최적화된 프롬프트를 캔버스 '프롬프트' 탭에 표시했습니다.)";

export function parsePromptBlock(content: string): string | null;
// fencedBlocks(content).find(b => b.tag === "prompt")?.body.trim() || null — 태그 일치만, 폴백 없음.

export function stripPromptBlock(content: string): string;
// prompt 태그 펜스만 PROMPT_NOTE로 치환(정규식은 fencedBlocks와 동일 패턴), 나머지 코드블록 보존.
// stripClarifyBlock(clarify.ts:129-135) 미러.
```

### 2. `src/lib/clarify.ts` — `noPrefill` 플래그

- `ClarifyQuestion`(`clarify.ts:17-27`)에 `noPrefill?: boolean` 추가.
- `prefillInstruction`(`:174-175`) 필터를 `q.type !== "folder" && !q.noPrefill`로 확장.
- `parsePrefill`(`:219`) folder skip 옆에 `if (q.noPrefill) continue;` 방어 추가.
- 이유: 요구사항 필드는 seed 원문으로 클라이언트가 결정적으로 채우므로 에이전트 프리필
  대상에서 제외(에이전트가 재서술하면 오히려 손실). options.ts→clarify.ts 순환 import 없이
  질문 스스로가 플래그를 가진다.

### 3. `src/lib/options.ts` — 요구사항 질문 전 카테고리 프리펜드

```ts
export const REQUIREMENT_QUESTION: ClarifyQuestion = {
  id: "userRequest",            // 기존 질문 id들과 충돌 없음(확인됨)
  label: "무엇을 하고 싶으신가요? 원하는 작업을 자유롭게 설명해 주세요.",
  type: "text",
  required: true,
  noPrefill: true,
};
```

`optionsFor`(`options.ts:146-154`)에서 **무조건 맨 앞** 프리펜드("뭘 하고 싶은지 우선"):
```ts
return needsCodebase
  ? [REQUIREMENT_QUESTION, CODEBASE_QUESTION, ...base]
  : [REQUIREMENT_QUESTION, ...base];
```
- `RequirementsForm`은 text+required+useAutoGrow를 이미 지원 → **폼 컴포넌트 무변경**.
- 기존 카탈로그의 topic/target 등 카테고리별 텍스트 질문은 그대로 둔다(더 구체적인 보조
  질문 — 중복은 수용).
- 부수 효과: 옵션 카탈로그가 비는 카테고리가 없어져 seed-first 폴백 경로(`ChatPanel.tsx:995-996`)는
  사실상 미도달이나 폴백으로 유지.

### 4. `src/components/WorkspaceView.tsx` — 제출 처리 + '프롬프트' 탭 상태

- `CanvasTab` 유니온(`:16-23`)에 `"prompt"` 추가.
- 상태 추가(`ragResult` `:121` 미러): `const [promptResult, setPromptResult] = useState<string | null>(null);`
- 핸들러(`handleRagResult` `:214-217` 미러):
  ```ts
  const handlePromptResult = (text: string) => { setPromptResult(text); setCanvasTab("prompt"); };
  ```
  (D46 선례대로 도착 시 자동 전환 — 교육 목적상 보여주는 게 핵심.)
- 초기화: `handleNewSession`(`:185-196`)과 `handleOpenSession`(`:198-210`)에
  `setPromptResult(null)` 추가(ragResult와 나란히).
- `handleSubmitAnswers`(`:340-354`): folder 분리 뒤 **요구사항 답변 추출**을 추가하고
  submission에 실어 보낸다:
  ```ts
  const req = answers.find((a) => a.id === "userRequest");
  const requirement = typeof req?.value === "string" ? req.value.trim() : "";
  setAnswerSubmission((s) => ({ wire, display, requirement, nonce: (s?.nonce ?? 0) + 1 }));
  ```
  (`answerSubmission` state 타입 `:172-176`에 `requirement: string` 추가.
  요구사항 답변은 wire에서 빼지 않는다 — `formatClarifyAnswers`가 "1. 무엇을 하고
  싶으신가요? → …"로 포함하는 것이 최적 프롬프트 생성의 입력이 된다.)
- ChatPanel에 `onPromptResult={handlePromptResult}` prop 전달(`:359-386` 블록),
  CanvasPanel에 `promptResult={promptResult}` 전달(`:403-426` 블록).

### 5. `src/components/CanvasPanel.tsx` — '프롬프트' 탭 렌더

- props에 `promptResult: string | null` 추가.
- `effectiveTab` 가드(`:213-234`)에 분기 추가: `tab === "prompt" ? (promptResult ? "prompt" : "files") : …`
  (rag 분기와 동형 — 결과가 생기면 세션 동안 pill 유지).
- pill(`:291-299`): `{!!promptResult && tabBtn("prompt", "프롬프트")}` — '검색 결과' 앞에 배치.
- 본문(`:350-` 분기): 새 branch —
  - 상단 안내줄: "제출한 요구사항으로 AI가 구성한 최적 프롬프트입니다. 이렇게 요청하면 더
    정확한 결과를 얻을 수 있어요." + **복사 버튼**(`src/lib/clipboard.ts`의 `copyText`,
    FileViewer의 Copy→Check 1.5초 패턴 재사용).
  - 본문: `overflow-auto` 컨테이너 안 `<pre className="whitespace-pre-wrap font-mono …">`
    (토큰 유틸리티 사용 — `bg-panel`, `text-ink` 등. iframe 불필요: 클라이언트가 다루는 평문).

### 6. `src/components/ChatPanel.tsx` — 핵심 배선

**(a) props**: `onPromptResult: (text: string) => void` 추가.

**(b) 신규 ref 2개** (기존 ref 군집 `:204-231` 옆):
```ts
// 내장 프롬프트 최적화 스킬(D65): 대화당 1회, 첫 실제 작업 턴에 주입. 로드 세션은 없음.
const promptSkillPendingRef = useRef(!initialSession);
// 이 턴의 end에서 ```prompt 블록을 파싱해야 함을 표시(inflightStepRef와 동형).
const promptInflightRef = useRef(false);
```

**(c) 주입 — `send()`의 스킬 조립 직후(`:708-729` 다음), `unwindSkills` 정의(`:732`) 전**:
```ts
let promptInjectedNow = false;
if (!isPrefill && promptSkillPendingRef.current) {
  promptSkillPendingRef.current = false;
  promptInjectedNow = true;
  promptInflightRef.current = true;
  skillBodies.unshift(PROMPT_OPTIMIZER_SKILL);   // 단계 스킬들보다 앞 = wire 최상단
}
```
`unwindSkills`(`:732-734`)에 되감기 동봉:
```ts
const unwindSkills = () => {
  for (const id of injectedNow) injectedSkillIdsRef.current.delete(id);
  if (promptInjectedNow) { promptSkillPendingRef.current = true; promptInflightRef.current = false; }
};
```
이로써 기존 3개 되감기 지점이 자동으로 커버됨: preflight 중지(`:761`), **preflight 스킵 →
재귀 send 체인**(`:773` — guide의 첫 단계 rag가 스킵돼도 다음 생성형 단계 턴에 재주입됨),
spawn 실패 catch(`:888`). 스텝 arming과 독립적이라 "첫 단계"가 아니라 **첫 실제 에이전트
턴**에 확실히 실린다(요구: 모든 카테고리에서 항상).

**(d) 파싱/스트립 — `end` 핸들러(`:450-530`)**: prefill 분기(`:457-472`) 뒤,
`inflightStepRef` 소비(`:474`) 근처에서:
```ts
if (promptInflightRef.current) {
  promptInflightRef.current = false;
  if (ev.status === "succeeded") {
    const msgs = messagesRef.current;
    for (let i = msgs.length - 1; i >= 0; i--) {
      if (msgs[i].role !== "assistant") continue;
      const block = parsePromptBlock(msgs[i].content);
      if (block) {
        const idx = i;
        mutateMessages((prev) =>
          prev.map((m, j) => (j === idx ? { ...m, content: stripPromptBlock(m.content) } : m)),
        );
        onPromptResult(block);
      }
      break;
    }
  }
}
```
- **반드시 `mutateMessages` 경유**(D55 동기 커밋). `persist(messagesRef.current)`(`:528`)보다
  **앞**에 두어 스트립된 내용이 저장·transcript에 반영되게 한다(세션리스 재전송 시 블록
  재노출 방지, 에이전트의 재출력 유도 방지).
- 파싱 실패(블록 없음) → 아무 일도 안 함(우아한 폴백, D30 관례). 실패/취소 end → 파싱 생략.
  스트림 도중 실패한 턴의 재시도(`retry`)는 기존 스킬 dedupe와 동일한 의미론(스트림 실패는
  되감지 않음 — wire가 이미 에이전트에 도달)이라 추가 처리 없음.
- 스트리밍 중에는 펜스 원문이 잠시 보이다가 완료 시 PROMPT_NOTE로 치환됨(수용 — D57
  "스트리밍 중 평문" 관례와 동일).

**(e) 요구사항 필드 자동 채움 — 부트 효과(`:982-999`)**:
```ts
if (optionQuestions.length > 0) {
  onClarify(optionQuestions);
  const seed = seedPromptRef.current.trim();
  if (seed) {
    onPrefill({ userRequest: seed });   // 즉시 클라이언트 채움(폼 리마운트)
    void send("", { system: true, prefill: true, display: "…" });   // 기존 프리필 턴 유지
  }
}
```
프리필 턴 완료 분기(`:463-464`)는 **병합 호출**로 변경(에이전트 답이 요구사항 값을 지우지
않도록 — `handlePrefill`은 `setClarifyPrefill(answers)` 전체 교체이므로):
```ts
const answers = parsePrefill(msgs[i].content, optionQuestions);
const seed = seedPromptRef.current.trim();
const merged = seed ? { userRequest: seed, ...answers } : answers;
if (Object.keys(merged).length) onPrefill(merged);
```
(`"userRequest"` 리터럴 대신 `REQUIREMENT_QUESTION.id`를 import해 사용.)

**(f) '원래 요청' 중복 제거 — `answerSubmission` 효과(`:1012-1025`)**:
요구사항 답변이 이제 `submission.wire`에 포함되므로, requirement가 있으면 seed 덧붙임을
생략하고 사용자 버블(display)도 요구사항 텍스트를 우선한다:
```ts
const seed = seedPromptRef.current.trim();
const requirement = submission.requirement?.trim() ?? "";
const wire = !requirement && seed ? `${submission.wire}\n\n원래 요청:\n${seed}` : submission.wire;
void send(wire, { display: requirement || seed || submission.display }) …
```
(`lastAnswersWireRef`(RAG 질의)는 그대로 `submission.wire` — 요구사항이 포함돼 질의 품질 향상.)

### 7. 설계문서 갱신 (CLAUDE.md 규칙 1 — 같은 작업 단위 필수)

- **`docs/design/05-decisions.md`**: **D65** 신설 — "요구사항 우선 필드 + 프롬프트 최적화
  내장 스킬 + '프롬프트' 캔버스 탭". 결정: (1) `REQUIREMENT_QUESTION` 전 카테고리 프리펜드
  + `noPrefill` + seed 클라이언트 채움 + '원래 요청' 대체, (2) 내장 스킬은 레지스트리 밖
  런타임 주입(D39/D40의 사용자 override와 무관하게 항상 — 근거: 전체 교체형 레지스트리에서
  유실 방지, 사용자 확정), 첫 실제 작업 턴 기준(스텝 아님 — guide rag 스킵 체인 대응),
  `unwindSkills` 동승 되감기, (3) ` ```prompt ` 펜스 계약(태그 일치만, 폴백 없음) + end
  스트립 + '프롬프트' 탭(rag 탭 동형, 세션 유지·자동 전환). 대안 기각: 별도 생성 턴/확인
  게이트(턴 추가·마찰 — 사용자 기각), `DEFAULT_SKILLS` 등록(override 사용자 미적용).
- **`docs/design/07-workspace-and-runs.md`**: "카테고리 워크플로우" 절 wire 조립 순서에
  내장 프롬프트 스킬(첫 작업 턴 1회, 최상단) 추가; CanvasPanel 탭 목록에 '프롬프트' 추가;
  "커서(ChatPanel)" 절에 `promptSkillPendingRef`/`promptInflightRef` 언급.
- **`docs/design/08-guided-flows-and-skills.md`**: wire 조립 블록에 내장 스킬 라인 추가;
  "흐름 한눈에" 다이어그램에 요구사항 질문·```prompt 파싱 반영; 쿡북 D(새 결과 형식)에
  promptCraft를 실전 예로 언급.
- **`docs/design/04-ui-and-design-system.md`**: WorkspaceView 캔버스 탭 나열에 '프롬프트'
  탭(제출 후 최적 프롬프트 표시 + 복사) 추가.
- **`docs/design/01-overview.md`**: 증분 요약(카테고리 가이드 플로우 항목)에 한 줄 반영.

---

## 수정 파일 요약

| 파일 | 변경 |
|---|---|
| `src/lib/promptCraft.ts` **(신규)** | 내장 스킬 본문 + `parsePromptBlock`/`stripPromptBlock`/`PROMPT_NOTE` |
| `src/lib/clarify.ts` | `ClarifyQuestion.noPrefill` + prefill 지시문/파서 제외 |
| `src/lib/options.ts` | `REQUIREMENT_QUESTION` + `optionsFor` 프리펜드 |
| `src/components/WorkspaceView.tsx` | `"prompt"` 탭 + `promptResult` 상태/핸들러/초기화 + submission `requirement` |
| `src/components/CanvasPanel.tsx` | 탭 가드/pill/본문(복사 버튼) |
| `src/components/ChatPanel.tsx` | 주입 ref 2개 + unwind 동승 + end 파싱/스트립 + 부트 채움/병합 + 원요청 대체 |
| `docs/design/{01,04,05,07,08}*.md` | 상기 문서 갱신 (D65) |

백엔드/타입 미러(`lib/types.ts`)/`RequirementsForm`/`settings.rs` 무변경. 신규 의존성 0.

## 엣지 케이스 체크리스트 (구현 시 확인)

- guide: 첫 단계 rag 스킵 → unwind → 재귀 send에서 재주입 → 실제 첫 에이전트 턴에 블록 출력.
- 프리필 턴: `!isPrefill` 가드로 완전 격리(스킬·커서·영속화 불변).
- 로드 세션: `promptSkillPendingRef = !initialSession` → 주입 없음.
- 세션리스(gemini/aipro/fabrix/plain): 1회 주입, 스트립된 content가 transcript로 흘러 재출력 없음.
- plain 에이전트가 펜스를 안 지키면 → 파싱 실패 → 탭 없음·작업은 그대로(폴백 없음이 의도).
- 폼 제출이 프리필 스트리밍과 겹침 → 기존 nonce 재시도(D55) 그대로. 병합 prefill이 늦게
  도착해도 제출 후엔 `clarify=null`이라 무해.
- 홈 seed 없이 카테고리 카드 시작 → 프리필 턴 없음, 사용자가 요구사항 직접 입력(필수 검증).

## 검증

1. `npm run build` (tsc + vite — 프론트 타입/번들 통과; 프론트 테스트 러너 없음, 백엔드 미접촉).
2. 시나리오 확인(Windows 앱 실행 환경에서):
   - plan/query/change/guide 각각: 진입 → 폼 맨 앞 요구사항 필드(필수) → 제출 → 첫 턴 wire
     최상단에 `PROMPT_OPTIMIZER_SKILL` → end에서 블록 파싱 → '프롬프트' 탭 자동 전환 →
     채팅엔 `PROMPT_NOTE`.
   - 홈 프롬프트 시작: 요구사항 필드가 seed로 채워지고, 에이전트 프리필 도착 후에도 유지(병합).
   - 사용자 버블이 요구사항 텍스트로 표시되고 '원래 요청' 중복 없음.
   - 새 세션/기록 열기: '프롬프트' 탭 소멸(`promptResult` 초기화).

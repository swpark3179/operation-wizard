# 04. UI & 디자인 시스템

## 컨셉

차분하고 따뜻한 중립 톤의 제품 크롬(chrome). **Open Design**의 팔레트/토큰을
그대로 이식했다(원본: `apps/web/src/styles/tokens.css`).

## 레이아웃 셸

```
┌─────────────────────────────────────────┐
│ TopBar (상단바 · 로고/제목; 폴더 표시 없음)│
├──────┬──────────────────────────────────┤
│ Nav  │  main (현재 뷰)                   │
│ Rail │   - HomeArea  (Home → Workspace)  │
│ (좌) │   - AgentsView  (Agents)          │
│      │   - FlowSettingsView (Flows)      │
└──────┴──────────────────────────────────┘
```

- `AppShell` = `TopBar`(로고·제목·배지만; 작업 폴더 표시 없음 — R1) + (`NavRail` + `main`).
  `main`은 `overflow-hidden`이고, 각 뷰가 자체 스크롤을 소유한다(워크스페이스의 2패널 전체 높이용).
- `NavRail`: 폭 56px 고정, 아이콘 버튼 3개 — **Home**(`Home`), **Agents**(`Boxes`), **Flows**(`Workflow`).
  활성 항목은 accent-tint 배경 + accent 색. (별도 Settings 뷰는 폐지 — 경로 설정은 Agents 카드로 통합, D38.)
- 뷰 전환 상태는 `App.tsx`의 `view`(`"home" | "agents" | "flows"`)가 보유. Home 재선택은
  `HomeArea`를 런처로 리셋한다.

## 화면

### Home 뷰 (워크스페이스 진입)
- `HomeArea`가 `screen`(`home`/`workspace`) 상태를 소유한다(디자인의 진입 구조).
- `HomeView`: 히어로("무엇을 도와드릴까요?") + 프롬프트 컴포저 + **작업 폴더 지정 버튼**(컴포저 바로 아래;
  미지정 시 새 프로젝트 폴더 자동 생성) + **4개 업무 카테고리**(개발 계획 수립 / 운영 가이드 생성 / 데이터
  조회 / 데이터 변경·권한) + 최근 작업(프로젝트 목록). 카테고리/전송 → **새 프로젝트** + 워크스페이스 진입.
  진입 시 프롬프트 대화가 아니라 **카테고리별 고정 선택지 폼을 먼저** 보여주고(프롬프트로 시작했으면 아는
  값 자동 채움), 폼 제출이 첫 작업 턴을 발사한다([07](07-workspace-and-runs.md)·[08](08-guided-flows-and-skills.md) D36).
- `WorkspaceView`: 좌 `ChatPanel`(412px, 실행 스트리밍; **요구사항 폼 대기 중에는 컴포저 비활성** — D41) +
  우 `CanvasPanel`(작업 폴더 파일 뷰어 — `.md`는 마크다운+mermaid 미리보기(D42); **'요구사항' 탭은 폼이
  대기 중일 때만 표시**). 상세 동작은 [07-workspace-and-runs.md](07-workspace-and-runs.md).

### Flows 뷰 (워크플로우·스킬 설정)
- `FlowSettingsView`: 카테고리 탭(segmented pill) + **단계 편집기** + **스킬 라이브러리**
  ([05](05-decisions.md) D39/D40).
  - 단계 카드: 이름 / 종류 셀렉트(조사·문서 생성·대화) / 지시문 textarea(자동 확장) / 문서 생성이면
    산출물 파일 경로 / 스킬 연결 칩(멀티 토글, 삭제된 스킬은 warn 칩) / ▲▼ 순서 이동 / 삭제 / 단계 추가
    (종단 대화 단계 앞에 삽입). 인라인 검증(마지막 단계는 '대화') 실패 시 저장 비활성.
  - 스킬 카드: 접이식(이름/본문 textarea/사용처 힌트 배지/삭제). 저장은 레지스트리 전체 교체.
  - 각 섹션에 **저장 / 기본값으로 되돌리기** + `사용자 정의`/`기본값` 배지 + `Saved.` 표시(AgentCard 패턴).

### Agents 뷰
- 제목 + 설명 + **에이전트당 카드 1개**(`AgentCard`)를 레지스트리 순서로 세로 나열.
  - `AgentsView`는 `list_agents` 결과를 `map`하여 렌더하고, 각 카드에 `detected[id]`,
    per-id `loading`/`error`를 전달한다(병렬 탐지 결과가 도착 순서로 카드를 재정렬하지 않음).
- `AgentCard`(범용, props 기반 — 제목·아이콘은 `info.name`에서) 표시 요소:
  - 상태 점(`StatusDot`) + 라벨: `Detecting…` / `Detected` / `Not detected`.
  - source 배지: `via custom path`(warn 톤) / `on PATH`(중립).
  - 버전 칩(모노스페이스), 해석된 경로(모노스페이스, 줄바꿈 허용).
  - 미탐지 시 진단 힌트(`DIAGNOSTIC_HINT`)와 에러 메시지.
  - 모델 목록: 접이식, `N models` + `live`/`fallback` 배지.
  - **Refresh** 버튼 → 그 에이전트만 재탐지.
  - **커스텀 경로**(접이식, `SlidersHorizontal` 토글; 별도 Settings 뷰였던 것을 통합 — D38):
    - 사용자 지정 경로 입력 + **Browse…**(Tauri `dialog` 실행 파일 선택).
    - **Save & detect**(저장 후 그 에이전트 자동 재탐지), **Clear**(경로 해제), 저장 시 "Saved." 표시.
    - 해당 에이전트의 env override(`info.envVar`, 예 `OPENCODE_BIN`)와 동등함을 안내(있을 때만). 경로가
      설정돼 있으면 토글에 `set` 배지.
  - `AgentsView`는 `settings`(경로 맵) + `onSave`(→ `set_agent_bin` 후 재탐지)를 각 카드에 전달한다.

## 디자인 토큰 (`styles/tokens.css`)

- CSS 변수로 정의: 배경 단계(`--bg*`), 경계선(`--border*`), 텍스트 단계(`--text*`),
  accent(테라코타 계열 `#c96442`), 시맨틱 색(green/blue/red/amber), 그림자, radius,
  이징/지속시간, 폰트 스택(serif/sans/mono).
- **라이트/다크 동시 지원**:
  - 명시적 `[data-theme="dark"]`
  - OS 선호(`@media (prefers-color-scheme: dark)` + `html:not([data-theme])`)
  - 두 경로 모두 동일한 다크 변수 세트를 사용.
- accent는 **앱 크롬 전용 액션 색**으로 한정(콘텐츠 강조에 남발하지 않음).

## Tailwind v4 매핑 (`styles/global.css`)

- `@import "tailwindcss"` + `@theme inline { ... }`로
  **디자인 토큰(CSS 변수) → Tailwind 유틸리티 색/폰트**를 매핑한다.
  - 예: `--color-app: var(--bg)`, `--color-ink: var(--text)`,
    `--color-accent: var(--accent)`, `--color-ok/bad/warn ...`,
    `--color-info: var(--blue)` / `--color-info-bg`(운영 가이드 카테고리 등에서 사용).
  - `inline` 사용으로 유틸리티가 `var(--…)`를 직접 참조 →
    **라이트/다크 전환이 자동 반영**(클래스 교체 불필요).
- base 레이어에서 전체 높이 100%, 기본 폰트/배경, 테마에 맞춘 스크롤바 스타일 지정.
- 컴포넌트는 의미 기반 유틸리티명(`bg-panel`, `text-ink-muted`, `border-line`,
  `text-accent`, `bg-ok-bg` 등)을 사용 → 토큰만 바꾸면 전역 테마가 바뀐다.

## 규칙

- 색/간격/폰트는 **토큰을 통해서만** 쓴다(하드코딩 hex 지양; 토큰 매핑으로 추가).
- 새 시맨틱 색이 필요하면 `tokens.css`에 변수 추가 → `global.css` `@theme`에 매핑.
- 아이콘은 `lucide-react`를 사용한다.

# Operation Wizard — 프로젝트 이해 가이드

> 이 문서는 프로젝트를 **처음 보는 사람이 전체 그림을 잡을 수 있도록** 쓴 종합 안내서입니다.
> 개념 → 구조 → 동작 → 확장 순서로 설명하며, 깊은 세부는 `docs/design/`의 설계문서로 연결합니다.
> (설계문서가 "정답의 단일 출처(SSOT)"이고, 이 문서는 그것을 이어 붙여 읽기 쉽게 풀어쓴 지도입니다.)

## 목차

1. [한 문장 요약](#1-한-문장-요약)
2. [무엇을 하는 앱인가 — 3대 기능](#2-무엇을-하는-앱인가--3대-기능)
3. [기술 스택 & 전체 아키텍처](#3-기술-스택--전체-아키텍처)
4. [코드 지도 (어디에 무엇이 있나)](#4-코드-지도-어디에-무엇이-있나)
5. [핵심 개념 깊이 보기](#5-핵심-개념-깊이-보기)
6. [사용자 관점 흐름 (홈 → 대화 → 캔버스)](#6-사용자-관점-흐름-홈--대화--캔버스)
7. [최근 개편 내용 (이번 작업)](#7-최근-개편-내용-이번-작업)
8. [빌드 · 실행 · 테스트](#8-빌드--실행--테스트)
9. [확장 가이드](#9-확장-가이드)
10. [알아두면 좋은 제약·한계](#10-알아두면-좋은-제약한계)
11. [용어 사전](#11-용어-사전)
12. [더 읽을거리](#12-더-읽을거리)

---

## 1. 한 문장 요약

**Operation Wizard**는 Windows 데스크톱 앱(Tauri v2 + React/TS + Tailwind v4)으로,
로컬에 설치된 **CLI 코딩 에이전트를 탐지·관리**하고, **대화 패널 + 캔버스 패널 워크스페이스에서 실제로
실행**해 개발/운영 업무를 진행한다. 지원 에이전트는 6종 — **OpenCode, Claude Code, Codex, Gemini CLI,
Antigravity, AI Pro(사내 Gemini 호환 도구)**.

- 제품명 `Operation Wizard` · 식별자 `com.shi.operationwizard` · 제작 Samsung SDS · 버전 `0.1.0`
- 대상: Windows 11 (WebView2 내장 가정)
- 혈통: 탐지·실행 로직은 검증된 오픈 참조 프로젝트 **Open Design**의 데몬 동작(`apps/daemon/src/runtimes/`)을
  **Rust로 재구현/이식**한 것이다. (원본 사양은 open-design 저장소의 `docs/cli-agent-detection-and-daemon.ko.md`.)

---

## 2. 무엇을 하는 앱인가 — 3대 기능

앱은 크게 세 가지 일을 한다. 이 세 개만 이해하면 전체가 보인다.

### ① 로컬 CLI 에이전트 탐지 (Agents 화면)
사용자가 PATH·설치 경로를 몰라도, 앱이 각 에이전트의 **설치 여부·경로·버전·사용 가능한 모델**을 자동으로
찾아 카드로 보여준다. 자동 탐지가 실패하는 환경(사내망/비표준 설치)에서는 **커스텀 경로**를 지정해 보완한다.

### ② 대화·캔버스 워크스페이스에서 실행 (Home → Workspace)
홈에서 업무를 시작하면 **좌측 대화 패널(ChatPanel) + 우측 캔버스 패널(CanvasPanel)**로 들어간다.
대화는 실제 에이전트 프로세스를 띄워 응답을 **스트리밍**하고(정지·세션 이어가기 지원), 캔버스는 작업 폴더의
파일을 트리 + 코드/HTML 미리보기로 보여준다. 대화는 파일(JSON)로 **영속화**되어 나중에 열람·이어가기가 된다.

### ③ 카테고리 가이드 플로우 (옵션 우선 + 시스템 스킬)
그냥 자유 대화가 아니라, **업무 카테고리별로 초반 단계를 유도/강제**한다.
- 카테고리를 고르면 **프롬프트 대화 대신 "선택지(옵션) 폼"을 먼저** 보여주고, 그것부터 결정한다.
- 홈에서 프롬프트로 시작했다면, 그 프롬프트에서 **알 수 있는 항목은 자동으로 채우고** 미확인 항목만 다시 묻는다.
- 옵션 제출 시 이 시스템이 제공하는 **"스킬"(지시문 묶음)**이 대화에 주입되어 방향/품질/제약을 잡는다.
- `plan`(개발 계획 수립)은 여기에 더해 **소스 조사 → 계획서(`docs/plan.md`) 작성** 단계까지 자동 진행한다.

> ③이 이번 작업에서 크게 개편된 부분이다. 자세한 흐름은 [5.4](#54-카테고리-가이드-플로우-3층-구조),
> [6](#6-사용자-관점-흐름-홈--대화--캔버스), [7](#7-최근-개편-내용-이번-작업)에서 다룬다.

---

## 3. 기술 스택 & 전체 아키텍처

| 영역 | 선택 |
|------|------|
| 앱 셸 | **Tauri v2** (Rust 네이티브 + 시스템 WebView2) |
| 프론트엔드 | **React 19 + TypeScript**, **Vite 7** |
| 스타일 | **Tailwind CSS v4** + CSS 변수 토큰(Open Design 팔레트) |
| 아이콘 | `lucide-react` |
| 백엔드 | **Rust** (Tauri 커맨드 + `Channel` 스트림) |
| Tauri 플러그인 | `opener`, `dialog` |

핵심 설계 원칙: **로컬 단독 데스크톱 앱**이라 별도 서버가 없다. 프론트(WebView)와 백엔드(Rust)는
**Tauri IPC(커맨드 호출 + `Channel` 스트림)**로만 통신한다. 새 외부 의존성은 최소화한다.

```
┌───────────────────────────────────────────────────────────┐
│ WebView2 (프론트엔드: React / TypeScript)                  │
│                                                             │
│   App ── AppShell(TopBar + NavRail + main)                  │
│    ├─ HomeArea ── HomeView ─(카테고리/프롬프트)→ WorkspaceView │
│    │                          ├─ ChatPanel  (대화·실행 스트림)  │
│    │                          └─ CanvasPanel(파일 뷰어 / 요구사항 폼) │
│    └─ AgentsView ── AgentCard[] (탐지 표시 + 커스텀 경로 설정) │
│                                                             │
│    lib/api.ts  ──  invoke() + Channel  (@tauri-apps/api)    │
└──────────────────────────┬──────────────────────────────────┘
                           │  Tauri IPC (커맨드 + Channel 스트림)
┌──────────────────────────┴──────────────────────────────────┐
│ Rust 백엔드 (src-tauri/src)                                  │
│   lib.rs   커맨드 등록/디스패치 + RunRegistry(취소용)          │
│    ├─ agents.rs   에이전트 정의(AgentDef) + 레지스트리 + RunSpec │
│    ├─ resolve.rs  실행 파일 경로 해석                          │
│    ├─ exec.rs     .cmd/.bat shim 래핑 + 타임아웃 프로브          │
│    ├─ detect.rs   탐지 파이프라인 → DetectedAgent + 모델 파서    │
│    ├─ run.rs      실행 엔진 → RunEvent를 Channel로 스트리밍      │
│    ├─ files.rs    캔버스용 list_dir / read_file (read-only)     │
│    ├─ projects.rs 세션/프로젝트 영속화 (fs)                     │
│    └─ settings.rs settings.json (에이전트별 커스텀 경로)         │
└──────────────────────────┬──────────────────────────────────┘
                           │  프로세스 실행 (cmd.exe /d /s /c …)
                           ▼
       로컬 CLI 에이전트들 (opencode / claude / codex / gemini / agy / aipro)
```

프론트의 **타입(`src/lib/types.ts`)은 백엔드 serde 구조체의 수동 미러**다. 백엔드 직렬화는 `camelCase`이며,
한쪽을 바꾸면 반드시 다른 쪽도 맞춘다(코드 생성 도구 미사용).

---

## 4. 코드 지도 (어디에 무엇이 있나)

### 백엔드 (`src-tauri/src/`)

| 파일 | 책임 |
|------|------|
| `main.rs` | 바이너리 진입점 (`run()` 호출) |
| `lib.rs` | Tauri 커맨드 등록/디스패치, `AgentInfo`, 플러그인 초기화, `RunRegistry`(취소용) managed state |
| `agents.rs` | 에이전트 정의(`AgentDef`) + 정적 레지스트리(`AGENT_DEFS`) + 실행 스펙(`RunSpec`/`StreamFormat`/`RunCtx`) |
| `resolve.rs` | def 기반 실행 파일 경로 해석 (커스텀 경로 → env → PATH + 툴체인 디렉터리) |
| `exec.rs` | `.cmd`/`.bat` shim을 `cmd.exe /d /s /c`로 래핑, `CREATE_NO_WINDOW`, 타임아웃 프로브 |
| `detect.rs` | resolve → `--version` → models 파이프라인 → `DetectedAgent`, 모델 파서, 진단 분류 |
| `run.rs` | 에이전트 실행 엔진: 자식 spawn + stdout 파싱 → `RunEvent`를 `Channel`로. 프로세스 트리 취소 |
| `files.rs` | 캔버스 파일 뷰어용 `list_dir` / `read_file` (2 MiB 상한) |
| `projects.rs` | 대화 영속화: `~/.operation-wizard/projects/<projectId>/…` (projectId-keyed) |
| `settings.rs` | `settings.json` 로드/저장 (에이전트별 커스텀 경로 맵) |

### 프론트엔드 (`src/`)

| 영역 | 파일 | 책임 |
|------|------|------|
| 진입/상태 | `App.tsx` | 뷰 전환(`home`/`agents`), 에이전트·탐지·설정 상태, 초기 로드 |
| IPC 래퍼 | `lib/api.ts` | `invoke()`·`Channel` 래퍼 (모든 커맨드) |
| 타입 | `lib/types.ts` | 백엔드 serde 구조체의 TS 미러 + 진단 힌트 맵 |
| 셸 | `components/AppShell, TopBar, NavRail` | 상단바 + 좌측 내비레일(Home/Agents) + 본문 |
| 에이전트 화면 | `components/AgentsView, AgentCard, AgentIcon, StatusDot` | 에이전트당 카드 1개(탐지 상태·경로·버전·모델 + **접이식 커스텀 경로 설정**) |
| 워크스페이스 | `components/HomeArea, HomeView, WorkspaceView, ChatPanel, AssistantMessage, CanvasPanel, FileViewer, RequirementsForm` | 홈 런처 + 좌 대화/우 캔버스, 실행 스트리밍·파일 뷰어·세션 영속화 |
| **가이드 플로우** | `lib/options.ts`, `lib/skills.ts`, `lib/workflow.ts`, `lib/clarify.ts` | 카테고리별 **고정 선택지 + 시스템 스킬 + 단계 오케스트레이터 + 프리필/폼 프로토콜** |
| 공용 | `components/workspace.ts`, `lib/useAutoGrow.ts` | `Category`/`ChatMessage` 등 공용 타입·상수, 자동 확장 textarea 훅 |

---

## 5. 핵심 개념 깊이 보기

### 5.1 에이전트 def + 공통 probe (탐지)

탐지는 **"런타임 정의(def) + 공통 probe"** 구조다. 에이전트별 데이터는 `agents.rs`의 정적 배열
`AGENT_DEFS`에 모으고, `detect.rs`의 공통 파이프라인이 그 정의를 받아 동작한다. 새 에이전트 추가 =
**레지스트리에 def 1개 추가**(+출력 형식이 다르면 파서 1개).

파이프라인 3단계:
```
resolve(경로 찾기) → version probe(--version) → models probe(있으면) → DetectedAgent
```
- **resolve**: 커스텀 경로(설정) → `*_BIN` 환경변수 → PATH + `%APPDATA%\npm`·scoop·bun·cargo·deno·volta·
  fnm 등 잘 알려진 툴체인 디렉터리 + def별 추가 경로(`~/.opencode/bin` 등)를 `PATHEXT`(`.EXE;.CMD;.BAT`)와
  조합해 스캔. (GUI 앱은 축소된 PATH로 뜨는 경우가 많아 명시적 보강이 필요하다.)
- **version probe**: `--version` 실행. spawn 성공 = "있음(available)". `.cmd`/`.bat`는 `cmd.exe /d /s /c`로 감싼다.
- **models probe**: opencode(`models`, 줄 단위)·codex(`debug models`, JSON)만 라이브 목록. 나머지(claude/gemini/
  antigravity/aipro)는 **정적 fallback 카탈로그**(`live`/`fallback` 배지로 출처 표시).

결과 `DetectedAgent`: `available` / `path` / `version` / `source`(custom-path·path·not-found) / `models` /
`modelsSource` / `diagnostic`(not-on-path·not-executable·missing-target).
→ 상세: [`docs/design/03-agent-detection.md`](design/03-agent-detection.md)

### 5.2 실행(run) 엔진 & 스트림 이벤트

`run_agent`는 워커 스레드에서 자식 프로세스를 spawn하고 `runId`를 즉시 반환한다. stdout을 에이전트별
포맷 파서로 파싱해 `RunEvent`를 **Tauri `Channel`로 스트리밍**한다(HTTP/SSE 없음).

`RunEvent` 종류: `status`(세션/모델), `textDelta`, `thinkingDelta`, `toolUse`, `toolResult`, `usage`,
`stdout`(plain 폴백), `error`, `end`(succeeded/failed/canceled).

에이전트별 실행 등급 & 세션 전략:
| 에이전트 | 등급 | 세션 전략 |
|----------|------|-----------|
| claude | 1급 (`stream-json`) | **mint**: 프론트가 UUID를 만들어 첫 턴 `--session-id`, 이후 `--resume` |
| codex | 1급 (`exec --json`) | **capture**: 스트림의 `thread_id`를 잡아 `exec resume` |
| gemini / aipro | 1급 (`stream-json`) | **세션리스**: CLI 세션 없음 → 매 턴 transcript 재전송 |
| opencode / antigravity | plain 폴백 (`-p`) | best-effort, 원시 stdout |

**정지(취소)**는 Windows에서 프로세스 트리를 종료한다(`.cmd → cmd.exe → node` 손자 구조라 직접 kill만으론
안 됨 → `taskkill /PID <pid> /T /F` + `child.kill()`).
→ 상세: [`docs/design/07-workspace-and-runs.md`](design/07-workspace-and-runs.md)

### 5.3 세션/프로젝트 영속화

대화는 인메모리만이 아니라 디스크에 저장된다.
```
~/.operation-wizard/projects/<projectId>/
  project.json                        # 프로젝트 매니페스트(제목·카테고리·workdir)
  workspace/                          # 기본 작업 폴더(cwd + 캔버스 루트; 폴더 미지정 시 자동 생성)
  sessions/<sessionId>/session.json   # 한 대화(메타 + 메시지)
```
- **프로젝트 ≠ 작업 폴더.** `projectId`는 프론트가 `crypto.randomUUID()`로 mint한다. **새 대화/카테고리 =
  새 프로젝트.** 홈 최근목록은 프로젝트 단위이며, 클릭 시 그 프로젝트의 마지막 세션을 이어 연다.
- **작업 폴더(workdir)**는 프로젝트별 값이다. 홈에서 폴더를 지정하면 그 폴더를, 아니면 프로젝트 전용
  `workspace/`를 자동 생성해 쓴다. `ensure_project`가 첫 send에서 폴더/매니페스트를 만든다(지연 생성).
- `sessionId`(폴더용, 프론트 mint)와 CLI 세션 id(`cliSessionId`, claude UUID / codex thread_id)는 별개다.

### 5.4 카테고리 가이드 플로우 (3층 구조)

로컬 CLI 스트림에는 **우리가 제어하는 표준 도구(tool-call) 채널이 없다.** 그래서 모든 "단계 강제"는
**클라이언트 오케스트레이션**으로 한다: 프롬프트에 지시문을 (사용자에게 안 보이게) 주입하고, 에이전트가 낸
텍스트를 **약속된 fenced 코드블록**으로 해석한다. 백엔드/새 IPC는 필요 없다.

세 층이 협력한다.

| 층 | 파일 | 무엇 | 언제 |
|----|------|------|------|
| ① 고정 선택지(옵션) | `lib/options.ts` (`CATEGORY_OPTIONS`) + `lib/clarify.ts`(프리필) | 카테고리 진입 시 즉시 보여줄 선택지 카탈로그. 프롬프트로 시작하면 아는 값 자동 채움 | 워크플로우 **이전**(프리플로우). 폼 제출이 첫 작업 턴을 발사 |
| ② 시스템 스킬 | `lib/skills.ts` (`SKILLS` + `CATEGORY_SKILL` + `skillFor`) | 카테고리별 **지속 지시문(페르소나·방법·제약)**. CLI 자체 스킬과 무관 | 첫 작업 턴에 **1회** 주입(세션이 이후 유지) |
| ③ 워크플로우 단계 | `lib/workflow.ts` (`Step`/`WORKFLOWS`) + `ChatPanel` 스텝 커서 | 옵션 제출 이후의 에이전트 턴들(조사/문서작성/대화). 자동전진·정지 제어 | 매 턴 단계 지시문 주입 + `end`에서 `kind`로 분기 |

주입은 모두 `ChatPanel.send()`의 **wire(에이전트에 실제로 보내는 프롬프트) 조립**에서 일어난다:
```
wire = [ (첫 작업 턴이면) skill.body,  (단계 armed면) step.instruction,  prompt ].join("\n\n")
```

**프리필(자동채움)**은 clarify와 대칭이다. 홈 프롬프트가 있으면 **숨김 격리 턴**을 돌려(세션 오염·영속화·
스텝 전진 없음) 에이전트가 요청에서 **확신 가능한 항목만** ` ```prefill ` JSON으로 채우게 하고, 검증 후 폼을
미리 채운다. 실패하면 빈 폼으로 정상 진행(대화 안 깨짐).

카테고리별 현재 구성:
- `plan`(개발 계획 수립): 선택지 → **소스 조사** → **계획서(`docs/plan.md`) 작성** → 자유 대화(생성형 단계 자동 진행).
- `guide`(운영 가이드) / `query`(데이터 조회) / `change`(데이터 변경·권한): 선택지 → 자유 대화(방향은 스킬이 담당).

→ 편집/확장 가이드(어디를 고치나): [`docs/design/08-guided-flows-and-skills.md`](design/08-guided-flows-and-skills.md)

---

## 6. 사용자 관점 흐름 (홈 → 대화 → 캔버스)

`plan`(개발 계획 수립)을 홈 프롬프트로 시작하는 경우를 예로 든 전체 여정이다.

1. **홈(HomeView)**: 히어로 + 프롬프트 컴포저 + 4개 카테고리 카드 + (선택) 작업 폴더 지정 + 최근 프로젝트.
   프롬프트를 쓰고 카테고리를 누르거나 전송하면 → **새 프로젝트 mint** + 워크스페이스 진입.
2. **워크스페이스 진입**: 좌 `ChatPanel` + 우 `CanvasPanel`. plan은 선택지 카탈로그가 있으므로 **곧바로
   우측 '요구사항' 탭에 선택지 폼**이 뜬다(대화창이 아니라 선택지부터).
3. **자동채움(프리필)**: 홈 프롬프트가 있으면 백그라운드 프리필 턴이 돌아, 프롬프트에서 알 수 있는 항목
   (예: 변경 유형/영향 영역)이 **미리 선택되어** 폼에 채워진다. 미확인 항목만 비어 있다.
4. **폼 제출**: 사용자가 남은 항목을 채우고 제출하면 → **첫 작업 턴**이 발사된다. 이 턴에 **스킬(계획 수립
   방법론) + 1단계(소스 조사) 지시 + 답변 + 원 요청**이 함께 주입된다.
5. **소스 조사 단계**: 에이전트가 파일 검색/읽기 도구로 관련 소스를 조사(스트리밍되는 도구 호출/결과가
   대화에 표시). 완료되면 **자동으로 다음 단계로 전진**.
6. **계획서 작성 단계**: 에이전트가 `docs/plan.md`를 작성하고, 캔버스가 **'파일' 탭으로 전환되어 그 문서를
   연다**(트리 리로드 + 파일 선택).
7. **자유 대화**: 종단 `chat` 단계. 이후는 일반 대화로, 사용자가 자유롭게 이어간다.
8. **정지·이어가기·기록**: 진행 중 **중지**(프로세스 트리 종료), 헤더에서 **새 세션**(에이전트 재선택) /
   **기록**(이 프로젝트의 저장된 세션 열기). 매 턴 자동 저장되어 나중에 이어볼 수 있다.

> 프롬프트 없이 카테고리만 클릭하면 3(프리필)만 건너뛴다 — 빈 선택지 폼이 즉시 뜨고 사용자가 채운다.
> 저장 세션을 다시 열면 옵션/스킬/단계 없이 **일반 대화**로 이어간다(가이드 상태는 transient, 저장 안 함).

---

## 7. 최근 개편 내용 (이번 작업)

open-design의 세 기법(①시작 시 구조화된 선택지 + 자동채움, ②시스템이 주입하는 스킬, ③강제 단계/결과형식)을
이식하고, 에이전트 관리 화면 중복을 정리했다. 요청 4건과 그 결과:

### 요청 1 — 카테고리 시작 = 선택지 우선 + 프롬프트 자동채움
- **무엇**: 카테고리 진입 시 프롬프트가 아니라 **고정 선택지 폼을 먼저** 표시. 홈 프롬프트로 시작하면 아는
  값 자동 채움, 미확인만 다시 노출.
- **어떻게**: `lib/options.ts`의 정적 카탈로그를 즉시 렌더 + `clarify.ts`의 프리필 프로토콜(숨김 격리 턴).
- **왜 이렇게**: 즉시성·결정성이 높고(open-design `od.inputs` 대응), 기존 요구사항 폼/캔버스 탭을 재사용해
  신규 백엔드 0. (결정: [`05-decisions.md`](design/05-decisions.md) **D36**)

### 요청 2 — 초반 단계 강제 + 시스템 제공 스킬 적용
- **무엇**: CLI 자체 스킬이 아니라 **이 시스템이 제공하는 스킬**(카테고리별 지시문)을 대화에 주입.
- **어떻게**: `lib/skills.ts`에 스킬을 **앱 번들**로 정의하고 **카테고리별 매핑**, 첫 작업 턴에 1회 주입.
  단계 강제는 `lib/workflow.ts` + `ChatPanel` 스텝 커서로.
- **왜 이렇게**: 현재 요구는 카테고리별 페르소나 고정이라 정적 상수가 가장 단순·안전. (결정: **D37**)

### 요청 3 — 다단계/결과형식을 강제할 때 어디를 고치는지 가이드
- **결과물**: [`docs/design/08-guided-flows-and-skills.md`](design/08-guided-flows-and-skills.md) —
  3층 구조, "무엇을 하려면 어디를 고치나" 변경 지점 쿡북, 주입 지점 지도, 에이전트별 한계.

### 요청 4 — 에이전트 화면 중복 제거
- **무엇**: 별도 **Settings 뷰를 폐지**하고 커스텀 경로 설정을 **Agents 카드**로 통합. NavRail은 Home/Agents 2개.
- **어떻게**: `AgentCard` 하단에 접이식 경로 설정(입력/Browse/Save & detect/Clear/env 안내)을 이식,
  `SettingsView.tsx` 삭제. 백엔드(`set_agent_bin`/`get_settings`)는 무변경.
- **왜 이렇게**: 채팅의 에이전트 선택 vs Settings의 에이전트 목록이 중복으로 보였고, 실제 진짜 중복은
  Agents(탐지 표시)+Settings(경로 편집)라 한 카드로 합치는 게 자연스럽다. (결정: **D38**)

> 전 범위 **프론트엔드 전용 변경**(Rust/serde/IPC 무변경). `tsc` + `vite build` 통과.
> 실제 앱 실행 검증(`npm run tauri dev`)은 MSVC 환경에서 수동으로 한다(아래 8절).

---

## 8. 빌드 · 실행 · 테스트

> ⚠️ **Rust/Tauri 빌드는 MSVC 환경 전용.** Git Bash 금지(`link.exe` 충돌), 순수 PowerShell도 링커 못 찾음.
> **"Developer PowerShell for VS 2022"** 또는 `vcvars64.bat` 초기화 후 실행.

| 작업 | 셸 | 명령 |
|------|-----|------|
| 의존성 설치 | 아무 셸 | `npm install` |
| 프론트 타입/번들 확인 | 아무 셸 | `npm run build` (= `tsc && vite build`) |
| 앱 실행(핫리로드) | **MSVC** | `npm run tauri dev` |
| 배포 빌드(인스톨러) | **MSVC** | `npm run tauri build` |
| 백엔드 테스트 | **MSVC** | `cargo test --manifest-path src-tauri\Cargo.toml` |

- 프론트 전용 명령(`npm run dev`/`build`)은 어느 셸에서나 된다.
- 사내망 제약(VS Installer 다운로드 실패 등)과 SDK 설정은 [`06-build-and-environment.md`](design/06-build-and-environment.md) 참조.

---

## 9. 확장 가이드

### 새 에이전트 추가
1. `src-tauri/src/agents.rs`의 `AGENT_DEFS`에 `AgentDef` 1개 추가(bin 후보/env override/타임아웃/fallback 모델).
2. 모델을 나열한다면 `models_probe`를 채우고, 출력 형식이 기존과 다르면 `detect.rs`에 파서 추가.
3. 실행까지 하려면 `RunSpec`(build_args/stream_format 등) 추가.
4. 프론트는 자동 반영(레지스트리를 `list_agents`로 받아 카드/셀렉터 렌더).

### 새 카테고리 / 스킬 / 단계 (가이드 플로우)
- **새 카테고리**: `components/workspace.ts`(`Category` + `CATEGORIES`) → `lib/options.ts`(선택지) →
  `lib/skills.ts`(스킬 + 매핑) → `lib/workflow.ts`(단계 배열).
- **선택지 추가/수정**: `lib/options.ts`의 `CATEGORY_OPTIONS`만 편집(`ClarifyQuestion` 스키마 재사용).
- **스킬 작성/수정**: `lib/skills.ts`의 `SKILLS`/`CATEGORY_SKILL`.
- **단계 추가/강제**: `lib/workflow.ts`의 `WORKFLOWS` 배열 + `ChatPanel`의 `end` 분기.
- **새 결과형식 강제**: `lib/clarify.ts`의 fenced-block 파서 패턴 복제(지시문 + `fencedBlocks` 파서 + `end` 반영).
- 자세한 절차와 주입 지점 지도: [`08-guided-flows-and-skills.md`](design/08-guided-flows-and-skills.md).

---

## 10. 알아두면 좋은 제약·한계

- **가이드 상태는 transient**: 옵션 카탈로그·단계 커서·스킬 arm·대기 폼은 저장하지 않는다. 저장 세션을 다시
  열면 일반 대화로 이어간다. 생성된 파일(`docs/plan.md` 등)만 실제로 남는다.
- **에이전트별 degrade**: claude/codex(세션형)는 스킬·단계가 세션에 유지돼 가장 풍부. gemini/aipro(세션리스)는
  매 턴 transcript 재전송이라 스킬/지시가 첫 턴 이후 lossy(크기 증가). opencode/antigravity(plain)는 도구
  스트림/파일 쓰기 보장이 약해 `plan`의 조사/문서 단계가 degrade.
- **codex 사내 TLS 오류**(`invalid peer certificate: BadSignature`): codex CLI 자체의 rustls 검증 실패
  (사내 프록시 재서명 인증서 불신)로, 앱 코드로 "고칠" 수 없다. 앱은 한글 안내 + **새 세션 복구**만 제공.
- **claude 라이브 모델 목록·인증 상태 프로브**는 미포팅(자체 프록시 인프라 의존) — 정적 fallback만.
- **플랫폼**: Windows 전용(경로 해석·프로세스 실행이 Windows 툴체인에 특화). macOS/Linux 미지원.
- **후속 예정**: 디자인의 5개 캔버스 아티팩트 전용 탭, opencode/antigravity 1급 실행 파서, SQLite/전문 검색,
  스킬 디스크 로더(재빌드 없는 추가) 등.

---

## 11. 용어 사전

| 용어 | 의미 |
|------|------|
| Agent | 로컬 CLI 코딩 에이전트 (opencode/claude/codex/gemini/antigravity/aipro) |
| Def / 레지스트리 | 에이전트별 정의(`AgentDef`)와 그 정적 배열(`AGENT_DEFS`) |
| Resolve / Probe | 실행 파일 경로 찾기 / 짧게 실행해 버전·모델 캡처 |
| Source | 실행 파일 출처: `custom-path` / `path` / `not-found` |
| Models source | 모델 목록 출처: `live`(CLI 실측) / `fallback`(정적 카탈로그) |
| Run / RunEvent | 실행 엔진 / 스트리밍되는 이벤트(text/tool/usage/end 등) |
| 세션(CLI) | claude UUID / codex thread_id — 대화 이어가기용 (`cliSessionId`) |
| 프로젝트 / workdir | 프론트 mint한 논리 단위 / 실제 실행 cwd·캔버스 루트 |
| 카테고리 | 업무 유형 `plan`/`guide`/`query`/`change` |
| 옵션(선택지) | 카테고리별 고정 질문 카탈로그(`CATEGORY_OPTIONS`, 진입 시 폼으로 표시) |
| 프리필 | 홈 프롬프트에서 옵션 답을 자동 채우는 숨김 격리 턴 |
| 스킬 | 이 시스템이 주입하는 카테고리별 지시문 묶음(CLI 스킬과 무관) |
| Step / Workflow | 옵션 제출 이후의 에이전트 턴 단위(`search`/`document`/`chat`)와 그 배열 |
| wire | 에이전트에 실제로 보내는 프롬프트(스킬+단계지시+사용자입력 조립; 화면엔 안 보임) |
| transient | 저장하지 않고 세션 종료 시 사라지는 상태 |

---

## 12. 더 읽을거리

설계문서(`docs/design/`)가 개념 수준의 단일 출처다. 코드 변경 시 함께 갱신한다.

| 문서 | 내용 |
|------|------|
| [README.md](design/README.md) | 설계문서 인덱스 |
| [01-overview.md](design/01-overview.md) | 프로젝트 개요·목표·범위·용어 |
| [02-architecture.md](design/02-architecture.md) | 전체 아키텍처(Tauri/IPC/데이터 흐름) |
| [03-agent-detection.md](design/03-agent-detection.md) | 핵심 기능: 에이전트 탐지 |
| [04-ui-and-design-system.md](design/04-ui-and-design-system.md) | UI 셸 + 디자인 시스템 |
| [05-decisions.md](design/05-decisions.md) | 주요 결정 로그(D1~D38) |
| [06-build-and-environment.md](design/06-build-and-environment.md) | 빌드/실행 환경 제약(MSVC·사내망) |
| [07-workspace-and-runs.md](design/07-workspace-and-runs.md) | 실행 엔진 + 대화/캔버스 워크스페이스 |
| [08-guided-flows-and-skills.md](design/08-guided-flows-and-skills.md) | 가이드 플로우 + 스킬 + 결과형식 강제 가이드 |

- 개발자용 빌드/실행 요약: 루트 [`README.md`](../README.md)
- 프로젝트 규칙·설계문서 자동 로드: 루트 [`CLAUDE.md`](../CLAUDE.md)

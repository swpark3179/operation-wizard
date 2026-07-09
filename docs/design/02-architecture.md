# 02. 아키텍처

## 기술 스택

| 영역 | 선택 |
|------|------|
| 앱 셸 | **Tauri v2** (Rust 네이티브 + WebView2 프론트) |
| 프론트엔드 | **React 19 + TypeScript**, **Vite 7** 번들러 |
| 스타일 | **Tailwind CSS v4** (`@tailwindcss/vite` 플러그인) + CSS 변수 토큰 |
| 아이콘 | `lucide-react` |
| 마크다운 렌더 | `react-markdown` + `remark-gfm` + `mermaid`(dynamic import, 캔버스 미리보기 — D42) |
| 백엔드 | **Rust** (Tauri 커맨드) |
| HTTP 클라이언트 | `reqwest` (blocking + native-tls=schannel — Confluence 수집·RAG 어댑터 전용, D48) |
| Tauri 플러그인 | `opener`, `dialog` |

선택 근거는 [05-decisions.md](05-decisions.md) 참조.

## 큰 그림

```
┌──────────────────────────────────────────────┐
│ WebView2 (프론트엔드: React/TS)               │
│   App → AppShell(TopBar + NavRail + main)     │
│   ├─ HomeArea → HomeView / WorkspaceView       │
│   │              (ChatPanel + CanvasPanel)     │
│   └─ AgentsView → AgentCard[] (탐지 표시+경로설정)│
│         │  invoke(...) + Channel  via @tauri-apps/api │
└─────────┼────────────────────────────────────┘
          │  Tauri IPC (커맨드 + Channel 스트림)
┌─────────┼────────────────────────────────────┐
│ Rust 백엔드 (src-tauri/src)                   │
│   lib.rs  ── 커맨드 등록/디스패치 + RunRegistry│
│   ├─ agents.rs   에이전트 정의(def) + 레지스트리 + RunSpec│
│   ├─ resolve.rs  실행파일 경로 해석           │
│   ├─ exec.rs     command_for(shim) + 프로브    │
│   ├─ detect.rs   탐지 파이프라인 → DetectedAgent│
│   ├─ run.rs      실행 엔진 → RunEvent(Channel) │
│   ├─ files.rs    list_dir / read_file          │
│   ├─ projects.rs 세션/프로젝트 영속화(fs)      │
│   ├─ settings.rs settings.json 영구화          │
│   ├─ knowledge.rs 지식 항목 CRUD(fs)           │
│   ├─ rag.rs      사용자 RAG API 어댑터(스텁)   │
│   └─ confluence.rs 크롤+수집 → IngestEvent(Channel)│
└───────────────────────────────────────────────┘
          │ 프로세스 실행/실행 (cmd.exe /d /s /c 등)
          ▼
   로컬 CLI 에이전트들 (opencode / claude / codex / gemini / agy / aipro ...)
```

## 백엔드 모듈 책임 (src-tauri/src)

| 모듈 | 책임 |
|------|------|
| `lib.rs` | Tauri 커맨드 정의·등록(`invoke_handler`), `AgentInfo`, 플러그인 초기화, `RunRegistry` managed state, 앱 실행 진입 |
| `agents.rs` | 에이전트 정의(`AgentDef`)와 정적 레지스트리(`AGENT_DEFS`), `find`/`all`, 실행 스펙(`RunSpec`/`StreamFormat`/`RunCtx`) |
| `resolve.rs` | 에이전트 def 기반으로 실행 파일의 실제 경로를 찾음 (우선순위 기반) |
| `exec.rs` | `.cmd`/`.bat` shim 래핑 + `CREATE_NO_WINDOW` 커맨드 빌더(`command_for`) + 타임아웃 프로브(`run_capture`) |
| `detect.rs` | def 기반 resolve→version→models 파이프라인, `DetectedAgent` 조립, 모델 파서, 진단 분류 |
| `run.rs` | 에이전트 실행 엔진: 자식 spawn + stdout 스트림 파싱 → `RunEvent`를 Tauri `Channel`로. `RunRegistry`(취소). ([07](07-workspace-and-runs.md)) |
| `files.rs` | 캔버스 파일 뷰어용 read-only 커맨드(`list_dir`/`read_file`) |
| `projects.rs` | 대화 영속화: `~/.operation-wizard/projects/<projectId>/{workspace,sessions/<sessionId>}`. **projectId=프론트 mint**(workdir와 분리), `ensure_project`(workdir resolve)/`save_session`/`list_sessions`/`load_session`. 모두 projectId-keyed. ([07](07-workspace-and-runs.md)) |
| `settings.rs` | 앱 설정(`settings.json`) 로드/저장: 에이전트별 경로 맵 + **스킬 레지스트리(`SkillDef`, `dir?` 리소스 폴더 포함)·카테고리별 워크플로우(`StepDef`, `output?` 포함) override**(D39/D45/D47) + **`ConfluenceConfig`/`RagConfig`**(D48; `RagConfig`는 endpoint+secretKey+passKey 헤더 값 — D50. PAT/키는 평문 저장 — 읽기 전용 키 권장) + 저장 시 검증(`validate_skills`/`validate_steps` — `STEP_KINDS` 6종·`STEP_OUTPUTS`). `CATEGORIES` 상수는 프론트 `workspace.ts`와 동기화 |
| `knowledge.rs` | 지식 베이스 CRUD: `~/.operation-wizard/knowledge/<id>.json`(항목당 1파일, upsert 타임스탬프 스탬프). knowledge 기반 단계 주입용 (D48) |
| `rag.rs` | **사용자가 채우는 RAG API 어댑터**: `RagClient::{ingest_page, search}` TODO(user) 스텁(미구현 시 한글 안내 Err) + `rag_search` 커맨드(spawn_blocking). 요약·임베딩은 사용자 RAG 서비스 담당 (D48) |
| `confluence.rs` | Confluence Server/DC REST 크롤(반복 BFS, visited dedupe, 상한) + 페이지 원문을 `rag.rs`로 전달. `IngestEvent`를 `Channel`로 스트리밍, `IngestRegistry`(취소 플래그) managed state (D48) |
| `main.rs` | 바이너리 진입점 (`run()` 호출) |

## 프론트엔드 구조 (src)

| 영역 | 파일 | 책임 |
|------|------|------|
| 진입/상태 | `App.tsx` | 뷰 전환(`home`/`agents`/`flows`/`knowledge`), 에이전트/탐지/설정 상태, 초기 로드 |
| IPC 래퍼 | `lib/api.ts` | `invoke()`·`Channel` 래퍼 (`listAgents`/`detectAgent`/`runAgent`/`cancelRun`/`listDir`/`readFile`/`pickFolder` + `ensureProject`/`setProjectCodebase`/`saveSession`/`listSessions`/`loadSession`/`listProjects` + `ragSearch`/`listKnowledge`/`saveKnowledge`/`deleteKnowledge`/`setRagConfig`/`setConfluenceConfig`/`startConfluenceIngest`/`cancelIngest`/`probeConfluence`) |
| 타입 | `lib/types.ts` | 백엔드 serde 구조체의 TS 미러(`AgentInfo`/`RunEvent`/`RunArgs`/`FileEntry` + `ProjectMeta`/`ProjectSummary`/`SessionMeta`/`StoredSession` + `RagHit`/`IngestEvent`/`KnowledgeEntry`/`ConfluenceConfig`/`RagConfig`) + 진단 힌트 맵 |
| 셸 | `components/AppShell, TopBar, NavRail` | 상단바(로고·제목, 폴더 표시 없음) + 좌측 내비레일(Home/Agents/Flows/지식) + 본문 |
| 화면 | `components/AgentsView` | 에이전트 관리 뷰(카드당 탐지 표시 + 경로 설정 통합; 별도 Settings 뷰 폐지 — [05](05-decisions.md) D38) |
| 화면 | `components/FlowSettingsView` | **Flows 설정 뷰**: 카테고리별 단계 편집기(추가/삭제/순서/kind/지시문/산출물 파일/**결과 형태(output)**/스킬 연결; **기반 3단계는 pinned + 비-plan 카테고리 토글** — D44/D47) + 스킬 라이브러리(추가/수정/삭제/사용처 힌트/**리소스 폴더(dir)** — D45) + 기본값 복원 ([05](05-decisions.md) D39/D40) |
| 화면 | `components/KnowledgeView` (+ `lib/ingest.ts`) | **지식 뷰**(D48): RAG 검색 설정(endpoint/secretKey/passKey/topK + 연결 테스트 — D50) + Confluence 수집(설정·수집 시작/중지·진행 표시) + 지식 베이스 CRUD. 수집 진행 상태는 **`lib/ingest.ts` 모듈 싱글턴 스토어**가 Channel과 함께 소유해 뷰 전환을 생존하고(`useIngestState` 구독; NavRail '지식' pulse 점 포함 — D51) |
| 워크스페이스 | `components/HomeArea, HomeView, WorkspaceView, ChatPanel, AssistantMessage, CanvasPanel, FileViewer, Markdown, RequirementsForm` (+ `lib/options.ts`, `lib/skills.ts`, `lib/workflow.ts`, `lib/clarify.ts`) | HOME 런처(최근 프로젝트, 새 채팅=새 프로젝트) + 좌 대화(새 세션·기록, **폼 대기 중 컴포저 차단** — D41; **채팅↔캔버스 폭은 경계 드래그로 조절** — D49)/우 캔버스(**파일 목록 탭 + 파일별 뷰어 탭(닫기 가능)** — D49; 요구사항 탭은 **폼 대기 중에만 표시**), 실행 스트리밍·파일 뷰어(**md+mermaid 미리보기** — D42)·세션 영속화·**카테고리 가이드 플로우**(고정 선택지 우선+프리필 자동채움·단계별 스킬 주입·소스 조사·문서 생성) ([07](07-workspace-and-runs.md), [08](08-guided-flows-and-skills.md)) |
| 가이드 플로우 | `lib/options.ts`, `lib/skills.ts`, `lib/workflow.ts` | 카테고리별 고정 선택지 카탈로그(`CATEGORY_OPTIONS`, 코드 정적) + **settings-aware 스킬 레지스트리(`DEFAULT_SKILLS`/`resolveSkills`)·워크플로우(`DEFAULT_WORKFLOWS`/`workflowFor(category, settings)`)** — 사용자 override는 settings.json, 코드 기본값은 폴백=샘플 ([08](08-guided-flows-and-skills.md), D39/D40) |
| 위젯 | `components/AgentCard, AgentIcon, StatusDot` | 범용 에이전트 카드(탐지 표시 + 접이식 커스텀 경로 설정) 및 상태 표시 |
| 스타일 | `styles/tokens.css, global.css` | 디자인 토큰 + Tailwind 테마 매핑 (blue→`info` 매핑 추가) |

> **타입 동기화 규칙**: `lib/types.ts`는 `detect.rs`/`settings.rs`의 serde 구조체를
> 수동으로 미러링한다. 백엔드 직렬화는 `camelCase`(`#[serde(rename_all)]`)이므로
> 한쪽을 바꾸면 다른 쪽도 반드시 맞춘다.

## IPC 계약 (Tauri 커맨드)

| 커맨드 | 입력 | 출력 | 비고 |
|--------|------|------|------|
| `list_agents` | — | `AgentInfo[]` | 레지스트리 메타(`id`/`name`/`envVar`). 프론트가 카드/설정 행을 이 순서로 렌더 |
| `detect_agent` | `agentId: string` | `DetectedAgent` | `async` + `spawn_blocking`. 알 수 없는 id는 에러. 느린 `models` 프로브가 UI 스레드를 막지 않도록 블로킹 스레드에서 실행 |
| `get_settings` | — | `Settings` | config 디렉터리에서 로드(에이전트별 경로 맵 + `workdir`) |
| `set_agent_bin` | `agentId: string`, `path: string \| null` | `Settings` | id 검증 후 저장. 빈 문자열/`null`이면 해당 에이전트 경로 해제 |
| `set_skills` | `skills: SkillDef[] \| null` | `Settings` | 스킬 레지스트리 전체 교체(검증: id 유일·이름 필수). `null` = 기본값 복원(필드 삭제) |
| `set_workflow` | `category: string`, `steps: StepDef[] \| null` | `Settings` | 카테고리 검증(`CATEGORIES`) + 단계 검증(≥1개·id 유일·kind 유효·마지막 `chat`). `null` = 기본값 복원(키 삭제) |
| `run_agent` | `args: RunArgs`, `onEvent: Channel<RunEvent>` | `runId: string` | 워커 스레드에서 자식 spawn 후 `RunEvent` 스트리밍. 즉시 `runId` 반환 |
| `cancel_run` | `runId: string` | — | 해당 실행의 프로세스 트리 종료(Windows `taskkill /T /F` + `child.kill()`) |
| `list_dir` | `path: string` | `FileEntry[]` | 캔버스 파일 트리(디렉터리 우선, 노이즈 dir 스킵) |
| `read_file` | `path: string` | `string` | 파일 내용(2 MiB 상한) |
| `ensure_project` | `projectId: string`, `workdir: string`(빈값 허용), `title: string`, `category: string` | `Project` | 프로젝트 폴더+`project.json` 생성(idempotent). workdir 빈값→`workspace/` resolve 후 반환 |
| `save_session` | `projectId: string`, `session: StoredSession` | — | `session.json` 기록(세션 폴더 자동 생성, `updatedAt` 스탬프). 매니페스트는 생성 안 함 |
| `list_sessions` | `projectId: string` | `SessionMeta[]` | 프로젝트 세션 메타 목록(최근 갱신 순). 폴더 없으면 `[]` |
| `load_session` | `projectId: string`, `sessionId: string` | `StoredSession` | 세션 전체(메타+메시지) 로드 |
| `list_projects` | — | `ProjectSummary[]` | 모든 프로젝트 요약, 최근 활동순. 홈 최근목록용 |
| `set_project_codebase` | `projectId: string`, `codebasePath: string \| null` | `Project` | 매니페스트의 코드베이스 경로 갱신/해제(D45) |
| `set_confluence_config` | `config: ConfluenceConfig \| null` | `Settings` | Confluence 수집 설정 저장/해제(빈 baseUrl=해제, D48) |
| `set_rag_config` | `config: RagConfig \| null` | `Settings` | RAG endpoint 설정 저장/해제 |
| `rag_search` | `query: string`, `topK?: number` | `RagHit[]` | rag 기반 단계 검색(사용자 어댑터 `rag.rs`; 미구현/미설정 시 한글 Err) |
| `list_knowledge` / `save_knowledge` / `delete_knowledge` | — / `entry` / `id` | `KnowledgeEntry[]` / `KnowledgeEntry` / — | 지식 베이스 CRUD |
| `start_confluence_ingest` | `onEvent: Channel<IngestEvent>` | `ingestId: string` | 크롤+수집 워커 시작, 진행 스트리밍 |
| `cancel_ingest` / `probe_confluence` | `ingestId` / — | — / `string` | 수집 취소 / 연결 테스트 |

> **카테고리 워크플로우의 실행은 여전히 클라이언트 오케스트레이션이다**: 단계 진행(소스 조사·문서 생성 등)은
> 프롬프트로 지시문·스킬을 주입하고 스트림 텍스트/도구 이벤트를 **클라이언트에서 해석**해 캔버스로 렌더한다
> (도구 채널 없음, 파싱 실패 시 일반 chat 폴백). 백엔드는 단계/스킬 **정의의 영속화만** 담당한다
> (`set_skills`/`set_workflow` — D39). 오케스트레이션은 `lib/workflow.ts`,
> 상세는 [07-workspace-and-runs.md](07-workspace-and-runs.md).

`DetectedAgent` / `Settings` / `AgentDef`의 필드는 [03-agent-detection.md](03-agent-detection.md),
`RunArgs` / `RunEvent` / `FileEntry` / 실행 흐름은 [07-workspace-and-runs.md](07-workspace-and-runs.md) 참조.

## 데이터 흐름 (대표 시나리오)

1. **앱 시작** → `App`이 `getSettings()`로 저장된 경로 맵을 읽고, `listAgents()`로
   레지스트리를 받은 뒤 각 에이전트에 대해 `detectAgent(id)`를 **병렬로** 호출.
2. 백엔드가 `agents::find(id)`로 def를 찾고 설정의 `agents[id].customBin`을 읽어
   resolve→probe 파이프라인 수행 후 `DetectedAgent` 반환.
3. `AgentsView`가 레지스트리 순서대로 `AgentCard`를 렌더. 카드는 에이전트별 상태
   (탐지됨/미탐지/로딩), 경로, 버전, 모델 목록(live/fallback), 진단 힌트를 표시하고,
   하단 접이식 섹션에서 **커스텀 경로 설정**(입력/Browse/Save & detect/Clear)을 제공.
4. **경로 설정 변경** → `AgentCard`의 접이식 경로 섹션에서 저장 → `set_agent_bin`
   후 그 에이전트만 자동 재탐지(D38).

## 설정·권한 (Tauri 구성)

- 윈도우: 1100×720 (최소 760×520), 제목 `Operation Wizard`.
- `tauri.conf.json`: `beforeDevCommand=npm run dev`, `devUrl=http://localhost:1420`,
  `frontendDist=../dist`. CSP는 현재 `null`.
- Vite dev 서버는 고정 포트 **1420**(`strictPort`), `src-tauri`는 watch 제외.
- capabilities(`default.json`) 권한: `core:default`, `opener:default`,
  `dialog:default`, `dialog:allow-open` (Agents 카드의 실행 파일 선택 + 홈의 작업 폴더 지정
  다이얼로그용). 실행 스트리밍 `Channel`은 core IPC를 타므로 별도 권한 불필요.
  `list_dir`/`read_file`은 커스텀 커맨드라 `fs` 플러그인/권한 없이 동작.

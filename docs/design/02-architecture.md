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
| HTTP 클라이언트 | `reqwest` (blocking + native-tls=schannel — RAG 어댑터·Confluence MCP(JSON-RPC over streamable HTTP)·원격 에이전트 전용, D48/D82) |
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
│   ├─ agents.rs   에이전트 정의(def, kind) + 레지스트리 + RunSpec│
│   ├─ resolve.rs  실행파일 경로 해석           │
│   ├─ exec.rs     command_for(shim) + 프로브    │
│   ├─ detect.rs   탐지 파이프라인 → DetectedAgent│
│   ├─ run.rs      실행 엔진 → RunEvent(Channel) │
│   ├─ fabrix.rs   원격 HTTP API(Fabrix): detect/run(SSE)/probe│
│   ├─ aipro.rs    원격 HTTP API(AI Pro, OpenAI 호환): detect/run(SSE)/probe│
│   ├─ files.rs    list_dir / read_file / write_file│
│   ├─ projects.rs 세션/프로젝트 영속화(fs)      │
│   ├─ settings.rs settings.json 영구화          │
│   ├─ knowledge.rs 지식 항목 CRUD(fs)           │
│   ├─ rag.rs      RAG 검색(Fabrix rag-chat)   │
│   ├─ mcp.rs      MCP JSON-RPC/streamable-HTTP 클라이언트│
│   └─ confluence.rs MCP 크롤 → 지식 베이스 artifact(IngestEvent)│
└───────────────────────────────────────────────┘
          │ 로컬: 프로세스 실행(cmd.exe /d /s /c)  ·  원격(fabrix·aipro)·Confluence MCP: HTTPS(POST+SSE/JSON)
          ▼
   로컬 CLI 에이전트들 (opencode / claude / codex / gemini / agy ...) + 원격 API (fabrix / aipro)
```

## 백엔드 모듈 책임 (src-tauri/src)

| 모듈 | 책임 |
|------|------|
| `lib.rs` | Tauri 커맨드 정의·등록(`invoke_handler`), `AgentInfo`, 플러그인 초기화, `RunRegistry` managed state, 앱 실행 진입 + **부트 진단**(panic hook, `~/.operation-wizard/startup-error.log`, 부팅 실패 시 한글 안내 다이얼로그 — D56) |
| `agents.rs` | 에이전트 정의(`AgentDef`, `kind: AgentKind {Local, Remote}` — D64)와 정적 레지스트리(`AGENT_DEFS` 7종 — 로컬 5 + 원격 2(fabrix D64·aipro D71)), `find`/`all`, 실행 스펙(`RunSpec`/`StreamFormat`/`RunCtx`) |
| `resolve.rs` | 에이전트 def 기반으로 실행 파일의 실제 경로를 찾음 (우선순위 기반) |
| `exec.rs` | `.cmd`/`.bat` shim 래핑 + `CREATE_NO_WINDOW` 커맨드 빌더(`command_for`) + 타임아웃 프로브(`run_capture`) |
| `detect.rs` | def 기반 resolve→version→models 파이프라인, `DetectedAgent` 조립, 모델 파서, 진단 분류 |
| `run.rs` | 에이전트 실행 엔진: 자식 spawn + stdout 스트림 파싱 → `RunEvent`를 Tauri `Channel`로. `RunRegistry`(취소; `RunHandle.child`는 `Option` — 원격 런은 자식 없이 취소 플래그만, `next_id`/`register_remote`/`unregister` 헬퍼 — D64). `run_agent`는 `kind==Remote`면 **`def.id`로 분기**해 `fabrix::run_fabrix`/`aipro::run_aipro`로 위임(D71). ([07](07-workspace-and-runs.md)) |
| `fabrix.rs` | **원격 HTTP API 에이전트(Fabrix, D64)**: `detect_fabrix(cfg, force)`(캐시 우선 — `force=false`+캐시 있으면 무네트워크 반환, 아니면 GET all-models → ko-name 매핑; D66), `run_fabrix`(POST messages + SSE 스트리밍 워커 → `RunEvent`; 파일 무접근 → 문서 산출물은 클라이언트가 `write_file`로 저장 — D67), `probe_fabrix`(연결 테스트 + 모델 캐시 저장), 순수 파서 `parse_models_json`/`parse_fabrix_sse_data`. `chat_body`의 `max_new_tokens`=8192(D67). `reqwest` blocking+native-tls 재사용(rag/confluence 레시피) + **`.no_proxy()`로 환경 프록시 우회**(D66), 신규 crate 0 |
| `aipro.rs` | **원격 HTTP API 에이전트(AI Pro, OpenAI 호환 — D71/D73/D74)**: `detect_aipro(cfg, _force)`(네트워크 없음 — 설정 있으면 `available`+정적 카탈로그, 없으면 `not-configured`; D73), `run_aipro`(POST `/chat/completions` + SSE 스트리밍 워커 → `RunEvent`; 파일 무접근 → 문서 산출물은 클라이언트가 `write_file`로 저장 — D67), `probe_aipro`(연결 테스트 = **최소 비스트림 `POST /chat/completions`**(`chat_probe`); 모델은 정적 카탈로그라 `/models` 미조회, D73), 순수 파서 `parse_openai_sse_data`(`delta.content`→TextDelta, `delta.reasoning`→ThinkingDelta(D74), `usage`→Usage, `error`→Error, `[DONE]` 종료). 인증 `Authorization: Bearer <apiKey>`. **`build_client`가 `User-Agent: opencode/<ver>`를 필수 부착**(사내 게이트웨이 allowlist + 백엔드 `ua.split("/")` — 누락 시 500/406, **D74**). `chat_body`의 `max_tokens`=8192(D67). 모델 목록 정적(glm-5.1/gpt-oss-120b/qwen3.6-27b). `fabrix.rs`와 동일한 `reqwest` blocking+native-tls + `.no_proxy()`(D66), 신규 crate 0 |
| `files.rs` | 캔버스 파일 커맨드: read-only `list_dir`/`read_file` + **`write_file`**(부모 dir 생성·5 MiB 가드 — 원격 에이전트 문서 산출물 저장용, D67) |
| `projects.rs` | 대화 영속화: `~/.operation-wizard/projects/<projectId>/{workspace,sessions/<sessionId>}`. **projectId=프론트 mint**(workdir와 분리), `ensure_project`(workdir resolve)/`save_session`/`list_sessions`/`load_session` + 매니페스트 갱신(`set_project_codebase`/`set_project_title` — D45/D60). 모두 projectId-keyed. ([07](07-workspace-and-runs.md)) |
| `settings.rs` | 앱 설정(`~/.operation-wizard/settings.json` — 홈 루트, D72; `settings.json.corrupt` 백업 동일 폴더) 로드/저장. `load`가 새 위치에 파일이 없고 레거시 `%APPDATA%\com.shi.operationwizard\settings.json`이 있으면 **1회 비파괴 자동 이전**(`migrate_legacy_settings`). 저장 내용: 에이전트별 경로 맵 + **스킬 레지스트리(`SkillDef`, `dir?` 리소스 폴더 포함)·카테고리별 워크플로우(`StepDef`, `output?` 포함) override**(D39/D45/D47) + **`ConfluenceConfig`**(url+authKey — 공식 MCP 서버 연결, D82) + **`RagConfig`**(D48; endpoint+secretKey+passKey 헤더 값 — D50. 키는 평문 저장 — 읽기 전용 키 권장) + **`FabrixConfig`**(endpointUrl+client+openapiToken+allowInvalidCerts — fabrix 원격 에이전트 연결, `set_fabrix` — D64) + **`AiProConfig`**(endpointUrl+apiKey+allowInvalidCerts — aipro 원격 에이전트 연결, OpenAI 호환 Bearer 인증, `set_aipro` — D71) + **`FabrixConfig`/`AiProConfig`/`RagConfig`의 `models: Vec<ModelOption>` 모델 캐시**(저장/새로고침/연결 테스트에서 갱신, 캐시 우선 — D66) + 저장 시 검증(`validate_skills`/`validate_steps` — `STEP_KINDS` 6종·`STEP_OUTPUTS`). `CATEGORIES` 상수는 프론트 `workspace.ts`와 동기화 |
| `knowledge.rs` | 지식 베이스 CRUD: `~/.operation-wizard/knowledge/<id>.json`(항목당 1파일, upsert 타임스탬프 스탬프). knowledge 기반 단계 주입용 (D48). **산출물 지식(D59)**: `kind`/`files`/출처 필드 + `save_knowledge_files`(산출물 파일을 `knowledge/artifacts/<id>/`로 staged-swap 복사) + `get_knowledge_root`(주입 인덱스·extraDirs용 절대경로); 삭제 시 폴더 동반 제거 |
| `rag.rs` | **RAG 검색 어댑터**(D65): `search`는 Fabrix rag-chat API 실연동(POST `/openapi/rag-chat/v1/messages` → `parse_rag_response`: 요약 답변 + 출처 청크 → `RagHit[]`; 헤더 `x-fabrix-client`/`x-openapi-token`, 모델 GLM 5.2 고정, `knowledgeAssetId` 설정·샘플 폴백) + `rag_search`(spawn_blocking) + `probe_rag`(연결 테스트 = `/models` `fetch_models` 조회 후 `rag.models`에 캐시 저장 — D66; `fabrix::parse_models_json` 재사용). 클라이언트는 **`.no_proxy()`로 환경 프록시 우회**(D66). `ingest_page`는 **`#[allow(dead_code)]` 스텁**(rag-chat에 ingest 없음; Confluence가 MCP로 전환해 호출부 없음 — D65/D82) |
| `mcp.rs` | **MCP(streamable HTTP, JSON-RPC 2.0) 클라이언트**(D82): `McpSession`(`connect`=initialize→`Mcp-Session-Id` 캡처→`notifications/initialized`→`tools/list`, `list_tools`, `call_tool`). 응답은 `application/json`/`text/event-stream` 양쪽 처리(본문 통째로 읽어 content-type 분기). operation당 1회 handshake(mid-op 404면 1회 재handshake). 헤더 `x-auth`+`Mcp-Session-Id`+`MCP-Protocol-Version`, `User-Agent: opencode/*`(게이트웨이 UA gate — D74/D75). 순수 파서(`parse_jsonrpc_body`/`sse_event_to_result`/`tool_result_text`/`parse_tools`/`arg_key_for`). `reqwest` blocking+native-tls+`.no_proxy()` 재사용, 신규 crate 0 |
| `confluence.rs` | **공식 Confluence MCP 서버로 수집**(D82): `crawl`(반복 BFS, visited dedupe, 상한)의 `ConfluenceApi` 구현을 `McpConfluence`(`mcp.rs` 사용 — getPageById/getChild/searchContent, 도구 이름·인자 키는 `tools/list` 스키마에서 해석, 결과 텍스트 관대 파싱)로 교체. `start_confluence_ingest(target: ConfluenceTarget{rootPageId,searchQuery}, …)`가 페이지들을 모아 **트리 전체를 지식 베이스 artifact 1개**로 저장(`knowledge::save_knowledge_docs`; 취소 시 부분 저장). `probe_confluence`=initialize+tools/list. `IngestEvent`를 `Channel`로 스트리밍, `IngestRegistry`(취소 플래그) managed state. (구 REST 크롤·`HttpConfluence`·WebView 스파이크(D75/D77)는 제거) |
| `main.rs` | 바이너리 진입점 (`run()` 호출) |

## 프론트엔드 구조 (src)

| 영역 | 파일 | 책임 |
|------|------|------|
| 진입/상태 | `App.tsx` | 뷰 전환(`home`/`agents`/`flows`/`knowledge`), 에이전트/탐지/설정 상태, 초기 로드(실패 시 **재시도 가능한 배너** — D56) |
| 안정성 | `components/ErrorBoundary` | 렌더 오류 격리: root(main.tsx) + 뷰 단위 keyed 바운더리 — 백지 화면 대신 폴백/복구 UI (D56) |
| IPC 래퍼 | `lib/api.ts` | `invoke()`·`Channel` 래퍼 (`listAgents`/`detectAgent`(id, `force` — 캐시 우선 D66)/`runAgent`/`cancelRun`/`listDir`/`readFile`/`writeFile`(D67)/`pickFolder`/`openInExplorer`(`openPath` — 캔버스 폴더 칩 탐색기 열기, D69) + `ensureProject`/`setProjectCodebase`/`setProjectTitle`/`saveSession`/`listSessions`/`loadSession`/`listProjects` + `ragSearch`/`probeRag`/`listKnowledge`/`saveKnowledge`/`deleteKnowledge`/`setRagConfig`/`setConfluenceConfig`/`startConfluenceIngest`/`cancelIngest`/`probeConfluence` + `setFabrixConfig`/`probeFabrix` + `setAiProConfig`/`probeAiPro` — D64/D65/D71) |
| 타입 | `lib/types.ts` | 백엔드 serde 구조체의 TS 미러(`AgentInfo`/`RunEvent`/`RunArgs`/`FileEntry` + `ProjectMeta`/`ProjectSummary`/`SessionMeta`/`StoredSession` + `RagHit`/`IngestEvent`/`KnowledgeEntry`/`ConfluenceConfig`/`RagConfig`/`FabrixConfig`/`AiProConfig`) + 진단 힌트 맵(`not-configured`/`unreachable` 포함, 에이전트 중립 문구 — D71) |
| 셸 | `components/AppShell, TopBar, NavRail` | 상단바(로고·제목, 폴더 표시 없음) + 좌측 내비레일(Home/Agents/Flows/지식) + 본문 |
| 화면 | `components/AgentsView` (+ `AgentCard`/`FabrixCard`/`AiProCard`) | 에이전트 관리 뷰(카드당 탐지 표시 + 경로 설정 통합; 별도 Settings 뷰 폐지 — [05](05-decisions.md) D38). **`id=="fabrix"`면 `FabrixCard`**(endpoint/client/token 3필드 — D64), **`id=="aipro"`면 `AiProCard`**(endpoint+API 키 2필드, OpenAI 호환 Bearer, endpoint는 알려진 상수로 프리필 — D71), 그 외는 `AgentCard`. 두 원격 카드는 탐지·모델 + 연결 테스트를 공유 |
| 화면 | `components/FlowSettingsView` | **Flows 설정 뷰**: 카테고리별 단계 편집기(추가/삭제/순서/kind/지시문/산출물 파일/**결과 형태(output)**/스킬 연결; **기반 3단계는 pinned + 비-plan 카테고리 토글** — D44/D47) + 스킬 라이브러리(추가/수정/삭제/사용처 힌트/**리소스 폴더(dir)** — D45) + 기본값 복원 ([05](05-decisions.md) D39/D40) |
| 화면 | `components/KnowledgeView` (+ `lib/ingest.ts`) | **지식 뷰**(D48): RAG 검색 설정(endpoint/x-fabrix-client/x-openapi-token/knowledgeAssetId/topK + 연결 테스트=`probe_rag` — D50/D65) + **캐시된 모델 목록 표시**(저장/연결 테스트가 조회·저장, 접이식 — D66) + Confluence 수집(설정·수집 시작/중지·진행 표시) + 지식 베이스 CRUD. 수집 진행 상태는 **`lib/ingest.ts` 모듈 싱글턴 스토어**가 Channel과 함께 소유해 뷰 전환을 생존하고(`useIngestState` 구독; NavRail '지식' pulse 점 포함 — D51) |
| 워크스페이스 | `components/HomeArea, HomeView, WorkspaceView, ChatPanel, AssistantMessage, WorkflowStepper, CanvasPanel, ArtifactsPanel, DiagramGallery, KnowledgeSavePanel, FileViewer, Markdown, RequirementsForm` (+ `lib/options.ts`, `lib/skills.ts`, `lib/workflow.ts`, `lib/clarify.ts`, `lib/clipboard.ts`, `lib/artifacts.ts`, `lib/knowledgeSave.ts`, `lib/ragRelevance.ts`, `lib/useArtifactExistence.ts`) | HOME 런처(최근 프로젝트, 새 채팅=새 프로젝트; **미탐지 에이전트 온보딩 배너** — D57) + 좌 대화(새 세션·기록, **폼 대기 중 컴포저 차단** — D41; **채팅↔캔버스 폭은 경계 드래그로 조절** — D49; **진행 스테퍼·채팅 마크다운 렌더+복사·같은 세션 재시도·이탈 확인·하단 고정 스크롤** — D57)/우 캔버스(**파일 목록 탭 + 파일별 뷰어 탭(닫기 가능)** — D49; 요구사항 탭은 **폼 대기 중에만 표시**; **산출물 허브 + 다이어그램 갤러리 탭, 워크플로우 산출 파일은 허브로 라우팅** — D58; **워크플로우 완료 배너 + '지식 저장' 탭**(산출물 선택+요약 생성→지식 베이스 저장) — D59; **RAG '검색 결과' 탭은 관련성 LLM 판단 통과 시에만 정리된 뷰로 표시** `lib/ragRelevance.ts`+`foundation.ts::ragCuratedHtml` — D70), 실행 스트리밍·파일 뷰어(**md+mermaid 미리보기** — D42, **목차 드롭다운** — D58, **html 본문 복사 버튼** `copyHtml` — D62)·세션 영속화·**카테고리 가이드 플로우**(고정 선택지 우선+프리필 자동채움·단계별 스킬 주입·소스 조사·문서 생성) ([07](07-workspace-and-runs.md), [08](08-guided-flows-and-skills.md)) |
| 가이드 플로우 | `lib/options.ts`, `lib/skills.ts`, `lib/workflow.ts` | 카테고리별 고정 선택지 카탈로그(`CATEGORY_OPTIONS`, 코드 정적) + **settings-aware 스킬 레지스트리(`DEFAULT_SKILLS`/`resolveSkills`)·워크플로우(`DEFAULT_WORKFLOWS`/`workflowFor(category, settings)`)** — 사용자 override는 settings.json, 코드 기본값은 폴백=샘플. `plan`·`query`(D61)·`change`(D62)·`guide`(D63)는 다단계 기본값(기반 단계 포함; `CATEGORY_FOUNDATION`이 pin할 종류 결정 — `guide`는 코드베이스 제외 rag+knowledge), `change`는 종류별 DC Manager 신청양식(HTML), `guide`는 사내 문서 시각화 + 운영 가이드(HTML) 산출 ([08](08-guided-flows-and-skills.md), D39/D40/D61/D62/D63) |
| 위젯 | `components/AgentCard, AgentIcon, StatusDot` | 범용 에이전트 카드(탐지 표시 + 접이식 커스텀 경로 설정) 및 상태 표시 |
| 스타일 | `styles/tokens.css, global.css` | 디자인 토큰 + Tailwind 테마 매핑 (blue→`info` 매핑 추가) |

> **타입 동기화 규칙**: `lib/types.ts`는 `detect.rs`/`settings.rs`의 serde 구조체를
> 수동으로 미러링한다. 백엔드 직렬화는 `camelCase`(`#[serde(rename_all)]`)이므로
> 한쪽을 바꾸면 다른 쪽도 반드시 맞춘다.

## IPC 계약 (Tauri 커맨드)

| 커맨드 | 입력 | 출력 | 비고 |
|--------|------|------|------|
| `list_agents` | — | `AgentInfo[]` | 레지스트리 메타(`id`/`name`/`envVar`). 프론트가 카드/설정 행을 이 순서로 렌더 |
| `detect_agent` | `agentId: string`, `force: boolean` | `DetectedAgent` | `async` + `spawn_blocking`. 알 수 없는 id는 에러. 느린 `models` 프로브가 UI 스레드를 막지 않도록 블로킹 스레드에서 실행. **`kind==Remote`면 `def.id`로 분기**(`fabrix`→`fabrix::detect_fabrix`, `aipro`→`aipro::detect_aipro`). **fabrix**: `force=false`+캐시 있으면 네트워크 없이 캐시 반환, `force=true`면 라이브 `all-models` 조회 후 `fabrix.models`에 저장(캐시 우선 — D64/D66). **aipro**: 네트워크 없이 설정 유무로만 판정 + 정적 카탈로그(`/models` 미사용 — D73) |
| `get_settings` | — | `Settings` | `~/.operation-wizard/settings.json`에서 로드(홈 루트 — D72). 없으면 레거시 `%APPDATA%` 파일에서 1회 자동 이전 |
| `set_agent_bin` | `agentId: string`, `path: string \| null` | `Settings` | id 검증 후 저장. 빈 문자열/`null`이면 해당 에이전트 경로 해제 |
| `set_skills` | `skills: SkillDef[] \| null` | `Settings` | 스킬 레지스트리 전체 교체(검증: id 유일·이름 필수). `null` = 기본값 복원(필드 삭제) |
| `set_workflow` | `category: string`, `steps: StepDef[] \| null` | `Settings` | 카테고리 검증(`CATEGORIES`) + 단계 검증(≥1개·id 유일·kind 유효·마지막 `chat`). `null` = 기본값 복원(키 삭제) |
| `run_agent` | `args: RunArgs`, `onEvent: Channel<RunEvent>` | `runId: string` | 워커 스레드에서 자식 spawn 후 `RunEvent` 스트리밍. 즉시 `runId` 반환. **`kind==Remote`(fabrix)면 `fabrix::run_fabrix`로 위임**(POST+SSE — D64) |
| `cancel_run` | `runId: string` | — | 해당 실행의 프로세스 트리 종료(Windows `taskkill /T /F` + `child.kill()`). 원격(fabrix) 런은 자식이 없어 취소 플래그만 set → SSE 루프가 연결 종료(D64) |
| `list_dir` | `path: string` | `FileEntry[]` | 캔버스 파일 트리(디렉터리 우선, 노이즈 dir 스킵) |
| `read_file` | `path: string` | `string` | 파일 내용(2 MiB 상한) |
| `write_file` | `path: string`, `contents: string` | — | 파일 쓰기(부모 dir 생성, 5 MiB 상한). 원격 에이전트(Fabrix) 문서 산출물을 `<workdir>/<step.file>`로 저장 — D67 |
| `ensure_project` | `projectId: string`, `workdir: string`(빈값 허용), `title: string`, `category: string` | `Project` | 프로젝트 폴더+`project.json` 생성(idempotent). workdir 빈값→`workspace/` resolve 후 반환 |
| `save_session` | `projectId: string`, `session: StoredSession` | — | `session.json` 기록(세션 폴더 자동 생성, `updatedAt` 스탬프). 매니페스트는 생성 안 함 |
| `list_sessions` | `projectId: string` | `SessionMeta[]` | 프로젝트 세션 메타 목록(최근 갱신 순). 폴더 없으면 `[]` |
| `load_session` | `projectId: string`, `sessionId: string` | `StoredSession` | 세션 전체(메타+메시지) 로드 |
| `list_projects` | — | `ProjectSummary[]` | 모든 프로젝트 요약, 최근 활동순. 홈 최근목록용 |
| `set_project_codebase` | `projectId: string`, `codebasePath: string \| null` | `Project` | 매니페스트의 코드베이스 경로 갱신/해제(D45) |
| `set_project_title` | `projectId: string`, `title: string` | `Project` | 프로젝트 제목 변경(홈 최근 목록 인라인 편집 — D60). 빈 제목 거부, 100자 상한 |
| `set_confluence_config` | `config: ConfluenceConfig \| null` | `Settings` | Confluence MCP 연결 설정 저장/해제(url+authKey; 빈 url=해제, D82) |
| `set_rag_config` | `config: RagConfig \| null` | `Settings` | RAG 연결 설정 저장/해제(endpoint·헤더 키·`knowledgeAssetId` trim·정규화 — D50/D65). `models` 캐시는 백엔드 소유 — 연결 동일 재저장 시 이월, 변경 시 무효화(D66) |
| `set_fabrix_config` | `config: FabrixConfig \| null` | `Settings` | Fabrix 연결 설정 저장/해제(빈 endpointUrl=해제, D64). `models` 캐시는 백엔드 소유 — 연결 동일 재저장 시 이월, 변경 시 무효화(D66) |
| `probe_fabrix` | — | `string` | Fabrix 연결 테스트: 모델 목록 조회(프록시 우회) 후 `fabrix.models`에 캐시 저장 + "연결됨 (N개 모델)"(D64/D66) |
| `set_aipro_config` | `config: AiProConfig \| null` | `Settings` | AI Pro 연결 설정 저장/해제(빈 endpointUrl=해제, D71). `models` 캐시는 백엔드 소유 — 연결(endpoint+apiKey) 동일 재저장 시 이월, 변경 시 무효화(D66) |
| `probe_aipro` | — | `string` | AI Pro 연결 테스트: 최소 비스트림 `POST /chat/completions`(model=glm-5.1) 호출(프록시 우회, `User-Agent: opencode/*` 필수 — D74) → HTTP 2xx면 "연결됨 — AI Pro 응답 정상". 모델은 정적 카탈로그(`/models` 미조회, D73) |
| `rag_search` | `query: string`, `topK?: number` | `RagHit[]` | rag 기반 단계 검색: Fabrix rag-chat `/messages` 실연동 → 요약 답변+출처 청크(D65); 미설정 시 한글 Err |
| `probe_rag` | — | `string` | RAG 연결 테스트: rag-chat `/models` 조회(프록시 우회) 후 `rag.models`에 캐시 저장 + "연결됨 (N개 모델)"(D65/D66) |
| `list_knowledge` / `save_knowledge` / `delete_knowledge` | — / `entry` / `id` | `KnowledgeEntry[]` / `KnowledgeEntry` / — | 지식 베이스 CRUD(삭제는 artifact 폴더 동반 제거 — D59) |
| `save_knowledge_files` | `entry: KnowledgeEntry`, `sources: string[]` | `KnowledgeEntry` | 산출물 파일을 `knowledge/artifacts/<id>/`로 복사(staged swap) + artifact 엔트리 upsert(D59). `files`는 복사된 이름으로 서버가 채움 |
| `get_knowledge_root` | — | `string` | 지식 루트 절대경로(주입 인덱스·extraDirs용 — D59) |
| `start_confluence_ingest` | `target: ConfluenceTarget`, `onEvent: Channel<IngestEvent>` | `ingestId: string` | MCP 크롤 워커 시작(rootPageId/searchQuery 대상), 페이지→지식 베이스 artifact, 진행 스트리밍 (D82) |
| `cancel_ingest` / `probe_confluence` | `ingestId` / — | — / `string` | 수집 취소 / MCP 연결 테스트(initialize+tools/list → "연결됨 — N개 도구", D82) |

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
- capabilities(`default.json`) 권한: `core:default`, `opener:default`, `opener:allow-open-path`,
  `dialog:default`, `dialog:allow-open` (Agents 카드의 실행 파일 선택 + 홈의 작업 폴더 지정
  다이얼로그 + 캔버스 폴더 칩의 **"탐색기에서 열기"**(`openPath` — `opener:default`에 없어 별도 추가,
  D69)용). 미리보기 HTML·마크다운의 **외부 링크 열기**(`openUrl` — D76)는 `opener:default`가 이미 포함하는
  `allow-open-url`로 커버되어 **신규 grant 불필요**(`allow-open-path`(D69)와 대조). 실행 스트리밍
  `Channel`은 core IPC를 타므로 별도 권한 불필요.
  `list_dir`/`read_file`/`write_file`은 커스텀 커맨드라 `fs` 플러그인/권한 없이 동작(D67).

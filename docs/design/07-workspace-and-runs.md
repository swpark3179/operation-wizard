# 07. 워크스페이스 & 에이전트 실행(run) 엔진

탐지(detection)에 이어, 이 문서는 **에이전트를 실제로 실행**하고 그 출력을
좌측 대화 패널 + 우측 캔버스 패널 워크스페이스로 스트리밍하는 기능을 다룬다.

이 기능은 Open Design 데몬의 **run/stream 절반**
(`apps/daemon/src/runtimes/runs.ts` + `defs/claude.ts` + `claude-stream.ts`)을
Rust로 포팅한 것이다. 단, 전송 계층은 HTTP+SSE 대신 **Tauri `Channel`**을 쓴다.
(탐지 절반은 [03-agent-detection.md](03-agent-detection.md).)

## 큰 그림

```
Webview (React)                         Rust 백엔드 (src-tauri/src)
  HomeView ─카테고리/프롬프트 선택─┐
  WorkspaceView                     │  invoke("run_agent", {args, onEvent: Channel})
    ├ ChatPanel ───────────────────┼───────────────►  run.rs
    │   Channel.onmessage(ev) ◄──────┤  Channel<RunEvent> 스트림   ├ resolve (resolve.rs 재사용)
    │                               │                              ├ 자식 프로세스 spawn (cwd, stdin)
    └ CanvasPanel                   │  invoke("cancel_run",{runId}) ├ reader 스레드: stdout 라인 파싱
        listDir / readFile ◄────────┘  invoke("list_dir"/"read_file")│   → RunEvent → channel.send
                                                                     └ RunRegistry(Mutex<HashMap>) 취소용
```

**핵심**: 새 Cargo 의존성 없음. 스트리밍은 `std::process` + reader 스레드
(`exec.rs`와 같은 패턴) + `tauri::ipc::Channel`(Tauri v2 내장)로 구현한다.

## 실행 스펙(`RunSpec`) — `agents.rs`

`AgentDef`에 실행용 필드 `run: Option<RunSpec>`를 추가했다(탐지 필드는 그대로).
Open Design의 `RuntimeAgentDef.buildArgs`/`streamFormat`/`promptViaStdin`에 대응.

- `RunSpec { build_args: fn(&RunCtx)->Vec<String>, prompt_via_stdin, prompt_format, stream_format, env }`
- `StreamFormat`: `ClaudeStreamJson` | `CodexJson` | `GeminiJson` | `Plain`
- `PromptFormat`(stdin 프레이밍): `ClaudeJson`(`{"type":"user",...}` 한 줄) | `Text`(원문)
- `RunCtx`: `cwd`, `model`(None/`"default"`이면 생략), `session_id`, `resume`(bool), `prompt`,
  `extra_dirs`(cwd 외 추가 읽기 폴더 — 프로젝트 코드베이스 경로 + armed 스킬 리소스 폴더, D45)
- **extraDirs 에이전트별 지원**: **claude** = entry마다 `--add-dir`(호출 단위라 매 턴 전달) /
  **gemini** = entry마다 `--include-directories`(gemini CLI 멀티 워크스페이스 플래그 — D52;
  이전에는 무시되어 코드베이스를 아예 읽지 못했다) / **codex** = workspace-write 샌드박스라
  별도 부여 플래그 없음(읽기는 대체로 넓게 허용, 쓰기만 워크스페이스로 제한 — D80) /
  **plain** = 무시(프론트가 경로를 항상 프롬프트에도 언급하므로 best-effort로 degrade) /
  **원격(fabrix·aipro)** = 파일시스템 무관이라 무시(코드베이스 실독 불가 — D67 한계, D71).
- **claude**(1급): `-p --output-format stream-json --verbose --input-format stream-json
  --permission-mode bypassPermissions --add-dir <cwd> [--add-dir <extraDir>...]` (+`--model`). 세션은 **클라이언트 mint**:
  첫 턴 `--session-id <uuid>`, 이후 `--resume <uuid>`. `ClaudeJson`/`ClaudeStreamJson`.
  env `BASH_DEFAULT_TIMEOUT_MS`/`BASH_MAX_TIMEOUT_MS`(도구 명령 타임아웃 상향, D53 — 사용자 env가
  이미 있으면 앱이 덮어쓰지 않음).
- **codex**(1급): `exec [resume] --json --skip-git-repo-check`(샌드박스: create
  `--sandbox workspace-write` / resume `-c sandbox_mode="workspace-write"` — cwd 쓰기 허용 +
  사내 관리 정책 상한과 호환, danger-full-access는 정책 거부 대상, D80) `[--model M]`,
  create-only `-C <cwd>`, resume 시 세션 id는 끝 위치 인자. 프롬프트 stdin(`Text`), `CodexJson`.
  세션은 **capture**(`thread.started.thread_id`).
- **gemini**(1급): `--output-format stream-json --yolo [--model M]`, 프롬프트 stdin(`Text`),
  `GeminiJson`, env `GEMINI_CLI_TRUST_WORKSPACE=true`. CLI 세션 없음 → 맥락은 매 턴 재전송.
  (aipro는 과거 gemini 호환 CLI였으나 D71에서 원격 HTTP로 이전 — 아래 참조.)
- **opencode·antigravity**: `RunSpec::plain()` = `-p "<prompt>"`, `Plain`(원시 stdout). best-effort
  이며 `.cmd` shim + cmd 메타문자(`&|><`) 조합에선 취약할 수 있다([05](05-decisions.md) D19).
- **fabrix**(원격 HTTP API, `kind: Remote` — D64): `RunSpec`을 쓰지 않는다(`run: None`). `run_agent`가
  최상단에서 `def.id`로 분기해 `fabrix::run_fabrix`로 위임 — 프로세스 spawn/stdin 없이
  `POST {endpoint}/openapi/chat/v1/messages`
  (헤더 `x-fabrix-client`/`x-openapi-token`, body `{modelIds:[model], contents:[prompt], llmConfig(max_new_tokens
  8192 — 긴 문서 잘림 방지, D67), isStream:true, systemPrompt}`)를 보내고 **SSE 응답**을 파싱해 `RunEvent`로
  방출한다(`TextDelta`만 — 도구/파일 이벤트 없음). `extra_dirs`/`cwd`는 무시(파일시스템 무관). 세션리스
  (매 턴 transcript 재전송). SSE 파서 `parse_fabrix_sse_data`
  (`event_status=="CHUNK" && content`→`TextDelta`; SUCCESS/R20000 마커→무이벤트; 실패 status→`Error`).
  파일을 못 쓰므로 **문서 단계 산출물은 클라이언트가 `write_file`로 저장**한다(`ChatPanel` end 분기 — D67).
- **aipro**(원격 HTTP API, `kind: Remote` — D71): fabrix와 동일 패턴이나 **OpenAI 호환** 프로토콜.
  `run: None` → `run_agent`가 `def.id`로 `aipro::run_aipro`에 위임 —
  `POST {endpoint}/chat/completions`(헤더 `Authorization: Bearer <apiKey>`, body `{model,
  messages:[{system},{user}], stream:true, stream_options:{include_usage:true}, temperature,
  max_tokens:8192}`)를 보내고 **SSE 응답**(`chat.completion.chunk`)을 파싱한다. SSE 파서
  `parse_openai_sse_data`(`choices[0].delta.content`→`TextDelta`, `usage`→`Usage`, `error`→`Error`,
  `[DONE]` 종료). `extra_dirs`/`cwd` 무시(파일시스템 무관), 세션리스(매 턴 transcript 재전송). fabrix처럼
  문서 산출물은 클라이언트 `write_file`로 저장(D67).

## 파이프라인 — `run.rs`

`run_agent`는 resolve → 자식 spawn을 **워커 스레드**에서 수행하고 `runId`를 즉시 반환한다.
느린 I/O는 IPC 스레드를 건드리지 않는다. (**`kind==Remote`면 최상단에서 `def.id`로 분기해
`fabrix::run_fabrix`/`aipro::run_aipro`로 위임** — 아래 5. 원격 경로 참조, D64/D71.)

1. `agents::find(id)` → def + `run` 스펙. `resolve_agent`로 실행 파일 해석(설정 customBin 반영).
2. `(run.build_args)(ctx)`로 인자 조립. `exec::command_for`로 `.cmd`/`.bat` shim 래핑 +
   `CREATE_NO_WINDOW` 적용(탐지와 공유하는 헬퍼).
3. 워커 스레드: 자식 spawn(+`run.env` 주입 — 부모 env에 이미 있는 키는 덮어쓰지 않음, D53) →
   파이프(stdin/stdout/stderr)를 **take**(자식 락을 잡지 않고 읽기 위함) → `RunRegistry`에 등록 →
   prompt 전달(`prompt_format`별: `ClaudeJson`은 `{"type":"user",...}` 한 줄, `Text`는 원문) 후
   stdin 닫기 → stderr는 별도 스레드로 드레인 → stdout을 `stream_format`별 파서로 파싱해
   `RunEvent`를 `channel.send` → EOF 후 `wait` → 레지스트리에서 제거 → 종료/실패/취소에 따라
   `error?` + `end` 전송. stdout/stderr 디코딩은 **lossy**(`stream_lines`: `read_until` +
   `from_utf8_lossy`) — 비 UTF-8 1바이트가 스트림 나머지를 유실시키지 않는다(D55).
4. `cancel_run(runId)`: `canceled` 플래그 set + **프로세스 트리 종료**. Windows에선 `.cmd` shim이
   node 손자를 띄우므로 직접 자식(cmd.exe)만 kill하면 에이전트가 살아남는다 → 자식 pid로
   `taskkill /PID <pid> /T /F`로 트리를 종료(실패 대비 `child.kill()` 병행). reader가 stdout EOF를
   만나 정상 종료 경로로 `end{status:"canceled"}`를 보낸다([05](05-decisions.md) D24). `RunHandle.child`는
   `Option`이라 **원격(fabrix·aipro) 런은 플래그만 set**하고 taskkill을 건너뛴다(SSE 루프가 플래그를 관측해 연결 종료).
5. **원격 경로(fabrix D64·aipro D71)**: `run_fabrix`/`run_aipro`가 config 로드 → `RunRegistry`에서 id
   민팅(`next_id`) + 자식 없는 취소 플래그 등록(`register_remote`) → 워커 스레드에서 POST(fabrix
   `.../messages` / aipro `.../chat/completions`) → 응답을 `BufReader`로 읽어 `data:` 라인을 lossy
   디코드(`run::stream_lines`와 동일 방식) → `parse_fabrix_sse_data`/`parse_openai_sse_data`로 `RunEvent`
   방출(루프 사이 취소 플래그 확인) → 스트림 종료 후 `unregister` + 단일 `end`(실패면 `Error` 선행).

> **취소 지연(원격)**: 취소 플래그는 SSE `data:` 라인 사이에서 확인되므로 서버가 조용하면 다음 바이트까지
> 지연될 수 있다(confluence 취소와 동형). 토큰 스트림은 자주 방출되어 실사용상 문제 없다.

## 이벤트 모델(`RunEvent`) — 직렬화 `camelCase`, `type` 태그

Open Design `DaemonAgentPayload`의 부분집합. 프론트 미러는 `lib/types.ts`.

| type | 필드 | 의미 |
|------|------|------|
| `status` | `label`, `model?`, `sessionId?` | 라이프사이클(init에서 model + 세션 id 캡처) |
| `textDelta` | `delta` | 어시스턴트 텍스트 조각(append) |
| `thinkingDelta` | `delta` | 추론(reasoning) 조각 |
| `toolUse` | `id`, `name`, `input` | 도구 호출 |
| `toolResult` | `toolUseId`, `content`, `isError` | 도구 결과 |
| `usage` | `inputTokens?`, `outputTokens?` | 토큰 사용량 |
| `stdout` | `chunk` | 원시 stdout(plain 폴백) |
| `error` | `message` | 실패 메시지 |
| `end` | `code?`, `status` | 종료(`succeeded`/`failed`/`canceled`) |

### 스트림 파서 (`run.rs`, 모두 `fn(&str)->Vec<RunEvent>` · 단위 테스트)

- **`parse_claude_stream_line`**(`claude-stream.ts` 포팅, 메시지 단위): `system/init`→`status`
  (+세션 id), `assistant`의 `text`→`textDelta`·`thinking`→`thinkingDelta`·`tool_use`→`toolUse`,
  `user`의 `tool_result`→`toolResult`, `result`→`usage`. `message.content`가 bare string이어도
  `textDelta`로 수용(방어, D55).
- **`parse_codex_event_line`**(`handleCodexEvent` 포팅, JSONL): `thread.started`→`status`
  (+`thread_id` 캡처), `item.started` `command_execution`→`toolUse{name:"Bash"}`, `item.completed`
  `command_execution`→`toolResult`(exit_code≠0=에러) / `agent_message`→`textDelta`,
  `turn.completed`→`usage`, `error`/`turn.failed`→`error`.
- **`parse_gemini_event_line`**(`handleGeminiEvent` 포팅, JSONL): `init`→`status{model}`,
  `message`+`role:assistant`+`content`(string 또는 파트 배열 — 배열은 평탄화, D55)→`textDelta`,
  `tool_use`/`tool_result`→도구, `result.stats`→`usage`, `error`→`error`(severity `warning`은 `status`).
- **`parse_fabrix_sse_data`**(`fabrix.rs`, SSE `data:` 페이로드 — D64): `event_status=="CHUNK" && content`
  →`textDelta`, 실패 status(FAIL/ERROR)→`error`, `status=="SUCCESS"`/`R20000` 종료 마커→무이벤트(워커가
  종료 후 단일 `end` 방출). 도구 이벤트는 없음(텍스트 스트림만).
- **`parse_openai_sse_data`**(`aipro.rs`, SSE `data:` 페이로드 — D71): OpenAI `chat.completion.chunk`의
  `choices[0].delta.content`→`textDelta`, **`delta.reasoning`/`reasoning_content`→`thinkingDelta`**
  (glm-5.1 추론 토큰 — D74), `usage`→`usage`, 최상위 `error`→`error`, `[DONE]` 종료 마커→무이벤트(워커가
  종료 후 단일 `end` 방출). 도구 이벤트는 없음. 모든 요청은 `User-Agent: opencode/<ver>` 부착(필수 — D74).

알 수 없는 타입/비JSON/빈 줄은 무시.

### 세션 전략 (에이전트별)

- **claude — mint**: 프론트가 대화 시작 시 `crypto.randomUUID()`로 세션 id를 만들어 첫 턴
  `--session-id`, 이후 `--resume`로 넘긴다. 취소가 아무리 일찍 일어나도 id를 이미 알고 있어
  **정지 후 재질문이 확실히 이어진다**([05](05-decisions.md) D25, D20 갱신).
- **codex — capture**: 첫 턴은 id 없이 `exec`, `thread.started.thread_id`를 `status`로 캡처해
  다음 턴 `exec resume <id>`로 이어간다.
- **gemini — 세션리스**: CLI 세션이 없어 프론트가 지금까지의 대화를 transcript로 만들어
  매 턴 프롬프트에 포함한다. `uuid` 크레이트는 불필요(프론트 `crypto.randomUUID`).
- **fabrix·aipro — 세션리스**(fabrix D64/aipro D71): API에 세션 개념이 없어 gemini와 동일하게
  transcript를 매 턴 재전송한다(fabrix `contents:[transcript]`, aipro `messages`의 user content).
  둘 다 ChatPanel `SESSION_AGENTS`에 미포함 → 자동으로 세션리스 경로.

## 캔버스 파일 뷰어 — `files.rs`

`fs` 플러그인 대신 최소 커맨드 3개(read 2 + write 1 — [05](05-decisions.md) D21/D67):
- `list_dir(path) -> FileEntry[]`(`{name, path, isDir}`; 디렉터리 우선 정렬, `.git`/`node_modules`/
  `target`/`.next` 스킵).
- `read_file(path) -> String`(2 MiB 상한, UTF-8 lossy).
- `write_file(path, contents) -> ()`(부모 dir 생성, 5 MiB 상한). 원격 에이전트(Fabrix)는 파일 접근이
  없어 문서 산출물을 못 쓰므로, 클라이언트가 스트리밍 텍스트를 `<workdir>/<step.file>`로 저장할 때
  쓴다([05](05-decisions.md) D67).

프론트 `FileViewer`는 텍스트는 코드로, `.html`은 **샌드박스 iframe**
(`sandbox="allow-scripts"`, same-origin 없음)로 미리보기(`buildSrcdoc`: 완결 문서는 그대로,
조각은 doctype 셸로 감쌈 — Open Design `runtime/srcdoc.ts` 최소 포팅). **미리보기 HTML의 외부
링크는 OS 브라우저로 연다(D76)**: `buildSrcdoc`이 `lib/linkGuard.ts::withLinkGuard`로 캡처단계 클릭
가드 스크립트를 주입해, http/https/mailto 링크 클릭을 `preventDefault`하고 `window.parent.postMessage`
로 부모에 넘긴다. `App.tsx`의 단일 리스너가 이를 받아 `openExternal`(opener `openUrl`)로 OS 기본
브라우저를 띄운다(뷰는 이동하지 않음; iframe 내 `#앵커`·`javascript:`는 그대로 둠). `.md`는 **마크다운 미리보기**
(`react-markdown`+`remark-gfm`, ` ```mermaid ` 펜스는 `mermaid` dynamic import로 실제 다이어그램 렌더,
`securityLevel: "strict"`; 렌더 실패 시 원본 코드 폴백 — [05](05-decisions.md) D42). 미리보기/소스 토글은
html·md 공통이며, `refreshNonce`로 열린 파일을 재읽는다(후속 단계가 같은 파일을 재작성한 경우).
md 미리보기 파일바의 **목차 버튼**(D58)은 렌더된 DOM에서 h1~h3을 추출한 드롭다운을 열고, 항목 클릭 시
엘리먼트 인덱스로 해당 섹션에 점프한다(슬러그/anchor 없음 — 한글·중복 헤딩 안전, 코드펜스 안 헤딩 자동 제외).
`.html` 파일바에는 **"본문 복사" 버튼**(D62)이 있어 문서 `<body>` 내용을 `text/html`로 클립보드에 복사한다
(DC Manager 등 리치 텍스트 편집기에 붙여넣으면 서식이 유지됨). 샌드박스 iframe은 opaque origin이라
읽을 수 없으므로 부모가 가진 원본 문자열을 `DOMParser`로 파싱해 `body.innerHTML`을 추출하고,
`clipboard.ts`의 `copyHtml`(`ClipboardItem` `text/html`+`text/plain`; 실패 시 contenteditable 선택 +
`execCommand` 폴백)로 복사한다.

## 세션/프로젝트 영속화 — `projects.rs`

대화는 더 이상 인메모리만이 아니라 디스크에 저장된다([05](05-decisions.md) D26, **D32/D33로 개정**).

**폴더 구조**(Windows, 홈 루트 = `%USERPROFILE%` — resolver와 동일 해석):
```
~/.operation-wizard/projects/<projectId>/
  project.json                        # 프로젝트 매니페스트
  workspace/                          # 기본 작업 폴더(cwd + 캔버스 루트)
  sessions/<sessionId>/session.json   # 한 대화(메타 + 메시지)
```

- **프로젝트 ≠ 작업 폴더.** `projectId`는 프론트가 `crypto.randomUUID()`로 **mint**한다(폴더명 = id).
  새 대화/카테고리 = 새 프로젝트. 백엔드 커맨드는 모두 **projectId-keyed**([05](05-decisions.md) D32).
- **작업 폴더(workdir, = 실행 cwd + 캔버스 루트)는 프로젝트별 값**이다. `ensure_project`가 resolve한다:
  외부 폴더가 주어지면 그대로, 없으면 프로젝트 전용 `workspace/` 하위폴더(생성 후 그 경로를 매니페스트에
  기록·반환). 홈에서 지정하는 `settings.workdir`은 **새 프로젝트용 선택적 외부 기본값**이다.
  `workspace/` 하위폴더는 `project.json`/`sessions/`를 캔버스 트리에서 분리한다.
- **`sessionId`는 프론트가 mint**(`crypto.randomUUID`)한 폴더용 id로, **CLI 세션 id와 별개**다
  (claude uuid / codex thread_id는 `cliSessionId`로 저장 → 재개용).
- **저장 시점**: 첫 질문 시 `ensure_project`(프로젝트 폴더+매니페스트) → `save_session`(세션) + 매 턴
  `end`마다 재저장. `save_session`은 **매니페스트를 만들지 않는다**(첫 저장 전 `ensure_project` 선행이
  불변식). `updatedAt`(및 `createdAt==0`이면)만 백엔드에서 `SystemTime`으로 스탬프한다.
- **데이터 모델**(serde `camelCase`): `Project{id,workdir,title,category,createdAt,codebasePath?}`
  (`category`/`codebasePath`는 `#[serde(default)]`로 구 매니페스트 하위호환; `codebasePath`는 기반
  단계의 분석 대상 폴더로 workdir와 **별개** — D45, 폼 답변이 ensure 이후 도착하므로
  `set_project_codebase`로 갱신), `SessionMeta{id,title,agentId,model,category,
  cliSessionId?,createdAt,updatedAt,messageCount}`, `StoredSession = SessionMeta + messages`(백엔드는
  `serde_json::Value`로 보관 → 프론트 `ChatMessage[]` 형태에 결합되지 않음). 홈 최근목록용
  `ProjectSummary{id,workdir,title,category,createdAt,updatedAt,sessionCount,lastSessionId?,codebasePath?}`(매니페스트 +
  `list_sessions_at` 롤업; 폴더명을 id로 사용해 구 결정적-id 폴더도 롤업). **새 Cargo 의존성 0**(id=프론트
  mint, 시각=`SystemTime`).
- 핵심 fn은 `root: &Path`를 받아(=`settings.rs` 스타일) temp root로 단위 테스트한다(잘못된 projectId
  거부, 기본 workspace 하위폴더 resolve, 외부 workdir 통과, save→list→load 왕복, 잘못된 sessionId 거부).

**하위호환**: 기존 결정적-id 폴더는 폴더명 == `project.id`이므로 projectId-keyed 커맨드가 그대로
해석한다. 열기/이어가기/저장 모두 무변경(외부-workdir 프로젝트처럼 보임). 마이그레이션 불필요.

**이어가기(로드)**: `load_session`이 준 `messages`/`agentId`/`model`/`cliSessionId`로 `ChatPanel`을
재수화한다. claude/codex는 `cliSessionId`로 CLI 세션 재개(있으면 `resume=true`), gemini/aipro는
로드된 메시지로 transcript를 재구성해 이어간다.

## IPC 커맨드 (추가분)

| 커맨드 | 입력 | 출력 | 비고 |
|--------|------|------|------|
| `run_agent` | `args: RunArgs`, `onEvent: Channel<RunEvent>` | `runId: string` | 워커 스레드에서 스트리밍. `RunArgs {agentId, prompt, cwd, model?, sessionId?, resume?, extraDirs?}`. `kind==Remote`면 `def.id`로 분기해 `fabrix::run_fabrix`/`aipro::run_aipro`로 위임(POST+SSE — D64/D71) |
| `cancel_run` | `runId: string` | — | 프로세스 트리 종료(`taskkill /T /F` + `child.kill()`) → `end{status:"canceled"}`. 원격(fabrix·aipro)은 자식 없이 취소 플래그만(SSE 루프가 연결 종료 — D64/D71) |
| `list_dir` | `path: string` | `FileEntry[]` | 캔버스 파일 트리 |
| `read_file` | `path: string` | `string` | 파일 내용(상한 2 MiB) |
| `write_file` | `path: string`, `contents: string` | — | 파일 쓰기(부모 dir 생성, 상한 5 MiB). 원격 에이전트(Fabrix) 문서 산출물을 `<workdir>/<step.file>`로 저장 — D67 |
| `ensure_project` | `projectId: string`, `workdir: string`(빈값 허용), `title: string`, `category: string`, `codebasePath?: string` | `Project` | 프로젝트 폴더+매니페스트 생성(idempotent). workdir 빈값→`workspace/` 하위폴더 resolve해 반환. 홈에서 지정한 폴더면 그 경로 |
| `set_project_codebase` | `projectId: string`, `codebasePath: string \| null` | `Project` | 기존 매니페스트의 코드베이스 경로 갱신/해제(D45) |
| `set_project_title` | `projectId: string`, `title: string` | `Project` | 프로젝트 제목 변경(홈 최근 목록 인라인 편집 — D60; 빈 제목 거부, trim + 100자 상한) |
| `save_session` | `projectId: string`, `session: StoredSession` | — | `session.json` 기록(세션 폴더 자동 생성; 매니페스트는 생성 안 함) |
| `list_sessions` | `projectId: string` | `SessionMeta[]` | 프로젝트 세션 목록(최근 갱신 순, 없으면 `[]`) |
| `load_session` | `projectId: string`, `sessionId: string` | `StoredSession` | 세션 전체 로드 |
| `list_projects` | — | `ProjectSummary[]` | 모든 프로젝트 요약(최근 활동순). 홈 최근목록용 |
| `rag_search` | `query: string`, `topK?: number` | `RagHit[]` | rag 기반 단계의 검색: Fabrix rag-chat `/messages` 실연동 → 요약 답변+출처 청크를 `RagHit[]`로(D65); 미설정/실패 시 한글 Err → 프론트가 "건너뜀"으로 처리(D48) |
| `probe_rag` | — | `string` | RAG 연결 테스트(지식 뷰): rag-chat `/models` 조회(프록시 우회) 후 `rag.models`에 캐시 저장 + "연결됨 (N개 모델)". assetId 불필요 — D65/D66 |
| `list_knowledge` / `save_knowledge` / `delete_knowledge` | — / `entry` / `id` | `KnowledgeEntry[]` / `KnowledgeEntry` / — | 지식 베이스 CRUD(`~/.operation-wizard/knowledge/<id>.json`; knowledge 단계 주입용. 삭제는 artifact 폴더 동반 제거 — D59) |
| `save_knowledge_files` / `get_knowledge_root` | `entry`, `sources: string[]` / — | `KnowledgeEntry` / `string` | 산출물 지식 저장(D59): 산출물 파일을 `knowledge/artifacts/<id>/`로 staged-swap 복사 + artifact 엔트리 upsert / 지식 루트 절대경로(주입 인덱스·extraDirs용) |
| `set_confluence_config` / `set_rag_config` | `config \| null` | `Settings` | 지식 뷰의 수집/검색 설정 저장·해제 |
| `set_fabrix_config` / `probe_fabrix` | `config \| null` / — | `Settings` / `string` | Agents 뷰 Fabrix 카드: 연결 설정 저장·해제(빈 endpointUrl=해제; `models` 캐시 이월/무효화) / 연결 테스트(프록시 우회 조회 → `fabrix.models` 캐시 저장 + "연결됨 (N개 모델)") — D64/D66 |
| `set_aipro_config` / `probe_aipro` | `config \| null` / — | `Settings` / `string` | Agents 뷰 AI Pro 카드: 연결 설정 저장·해제(빈 endpointUrl=해제) / 연결 테스트 = **최소 비스트림 `POST /chat/completions`**(model=glm-5.1, 프록시 우회, `User-Agent: opencode/*` 필수 — D74) → HTTP 2xx면 "연결됨 — AI Pro 응답 정상". 모델은 정적 카탈로그(`/models` 미조회) — D71/D73/**D74** |
| `start_confluence_ingest` | `target: ConfluenceTarget`, `onEvent: Channel<IngestEvent>` | `ingestId: string` | Confluence **MCP** BFS 크롤(워커 스레드) → 페이지들을 지식 베이스 artifact 1개로 저장(`IngestEvent` 스트리밍: `started`/`pageFetched`/`pageIngested`/`pageFailed`/`error`/`end`; D82) |
| `cancel_ingest` / `probe_confluence` | `ingestId` / — | — / `string` | 수집 취소(`IngestRegistry` 플래그) / MCP 연결 테스트(initialize+tools/list → 도구 요약, D82) |

## UI (Home / Workspace)

- **NavRail**: **Home** / **Agents** / **Flows** / **지식** 4개 항목.
  `View = "home"|"agents"|"flows"|"knowledge"`.
  (별도 Settings 뷰는 폐지되고 경로 설정이 Agents 카드로 통합됨 — [05](05-decisions.md) D38.
  Flows는 워크플로우 단계·스킬 설정 화면 — D39; 지식 뷰는 RAG 연결·Confluence 수집·지식 베이스
  CRUD — D48, [04](04-ui-and-design-system.md).)
- **HOME**(`HomeView`): 히어로(운영 도구 프레이밍 — "운영 작업 마법사" + 진행 절차 안내, D60) +
  프롬프트 컴포저 + 4개 업무 카테고리(개발 계획 수립/운영 가이드
  생성/데이터 조회/데이터 변경·권한) + **최근 작업 = 프로젝트 목록**(`listProjects`, 전역·모든 프로젝트,
  최근 활동순, **세션 0인 프로젝트는 숨김**). 각 항목은 **프로젝트 제목** + 세션 수·시각을 보여주고,
  클릭 시 그 프로젝트의 **id + 저장된 workdir**로 워크스페이스에 진입해 **가장 마지막 세션**
  (`lastSessionId`→`loadSession(projectId)`)을 연다(세션이 없으면 새 대화). 행 hover의 **연필 버튼 →
  인라인 제목 편집**(Enter 저장/Escape 취소, `set_project_title` — D60). 카테고리/전송(새 채팅) →
  **새 프로젝트**(id mint) + 워크스페이스 진입(첫 턴 프롬프트 seed). **컴포저 전송(프롬프트로 시작)은
  카테고리를 지정하지 않고 `autoCategory=true`로 넘겨 진입 후 자동 분류하고(D81 — 아래 "카테고리
  워크플로우 ⓪"), 카테고리 카드 클릭은 그 카테고리로 명시 진입한다.** 컴포저 하단에 **에이전트·모델
  셀렉트**가 있어 시작 전에 실행 조합을 고른다(선택값은 `initialAgentId`/`initialModel`로 ChatPanel에
  시드되고 첫 턴 전까지 변경 가능; 명시 선택은 탐지 기반 자동 기본값이 덮어쓰지 않음 — D60). 컴포저
  **바로 아래에 작업 폴더 지정
  버튼**(`pickFolder`)이 있어, 지정 시 그 폴더가 프로젝트 폴더가 되고 **미지정 시 자동 생성**된다(프로젝트별
  transient 선택; [05](05-decisions.md) D33 R1). Home nav 재선택 시 런처로 리셋(`resetNonce`). 프롬프트
  컴포저는 입력에 따라 자동 확장 후 상한(≈200px)에서 스크롤(`useAutoGrow`).
- **WORKSPACE**(`WorkspaceView`): 좌 `ChatPanel`(기본 412px — **패널 경계를 드래그해 폭 조절**,
  clamp 후 localStorage `ow.chatWidth`에 기억; 드래그 중에는 전역 오버레이로 캔버스 iframe의 이벤트
  삼킴을 차단 — D49) + 우 `CanvasPanel`. `WorkspaceView`가
  **세션 remount 키(`sessionNonce`) + `loadedSession` + `resolvedWorkdir` + `openFiles`(파일 뷰어 탭
  목록, D49) + `artifacts`(워크플로우 산출물 목록, 세션당 고정)/`artifactSel`/`stepProgress` 미러
  (산출물 탭, D58)**를 소유한다 — 새 세션/기록
  열기는 키를 올려 `ChatPanel`을 통째로 리마운트해 모든 state/ref를 한 번에 초기화하되, **`projectId`와
  `resolvedWorkdir`는 유지**(같은 프로젝트)한다([05](05-decisions.md) D27, D33). `projectId`는 `HomeArea`가
  소유(새 채팅=mint, 최근 열기=채택). 프레시 채팅은 첫 send에서 `ensureProject`를 **항상 1회 호출**
  (`ChatPanel.ensuredRef`; 홈에서 지정한 폴더가 이미 있어도 매니페스트를 기록)해 workdir을 확정하고
  `onResolveWorkdir`로 올린다.
  - `ChatPanel`: 메시지 목록 + 컴포저(에이전트/모델 선택), 전송 시 user+assistant 메시지 추가 후
    `Channel` 생성 → `runAgent` → `RunEvent`를 마지막 assistant 메시지에 접합(text→content,
    thinking→thinking, tool/usage→`events[]` 타임라인, `end`→종료). **중지**=`cancelRun`.
    입력은 `textarea`: **Enter=전송, Shift+Enter/Ctrl+Enter=줄바꿈**, 내용에 따라 자동 확장 후
    상한(≈160px)에서 스크롤(`useAutoGrow`). 세션형(claude/codex)은
    최신 턴만 + `sessionId`/`resume`으로 이어가고, 세션리스(gemini/aipro)는 transcript를 매 턴
    전송한다. 대화 시작 후 에이전트 select는 고정(**새 세션**에서만 변경). **요구사항 폼이 대기 중이면
    (`formPending`) 컴포저(textarea/전송)를 비활성**하고 `send()`도 가드한다 — 숨김 프리필/자동전진
    턴(system)은 예외, 정지 버튼은 차단하지 않음(D41). 헤더에 **홈·기록·새 세션**
    버튼: **기록**=`listSessions` 팝오버(선택 시 `loadSession`→이어보기), **새 세션**=대화 초기화 +
    에이전트 재선택(워크스페이스 유지). 첫 질문 시 `ensureProject`로 프로젝트 폴더가 만들어지고(workdir
    resolve), `saveSession(projectId)`로 세션이 기록되며 매 턴 재저장된다. 카테고리별 워크플로우 진행은
    아래 "카테고리 워크플로우" 참조. **사용성(D57)**: 헤더 아래 **`WorkflowStepper`**(단계별
    `pending/active/done/skipped/halted` 상태 — transient, 로드 세션·단일 chat 워크플로우는 미표시),
    스트리밍 중 홈/새 세션/기록 열기는 **확인 다이얼로그**(`plugin-dialog` `ask`; NavRail 이동은
    App 레벨에서 동일 가드 — busy 상태 리프트), 모든 정지·중단 경로의 **통일된 시스템 노트**,
    rag/knowledge preflight 동안 "검색 중…" 일시 노트, **하단 고정형 자동 스크롤 + '최신으로' 버튼**,
    미탐지 에이전트 선택 시 경고 라인(+Agents 이동), 세션 열기 실패 배너. **지식 저장(D59)**: 워크플로우가
    종단 chat에 도달하면(파일을 생성한 단계 ≥1, 세션당 1회) 컴포저 위에 **완료 배너**를 띄워 캔버스
    '지식 저장' 탭을 제안한다(dismissible; 캔버스 탭 자동 전환 없음).
  - `AssistantMessage`: 텍스트 + reasoning(접이식) + 도구 행 + usage + 에러 렌더. **완료된 턴의
    텍스트는 마크다운(mermaid 포함)으로 렌더**하고(스트리밍 중에는 평문 — D57), 응답/코드블록 **복사
    버튼**과 헤더의 **실행 에이전트명**을 표시한다. 마크다운 링크는 `Markdown.tsx`의 `a` 오버라이드가
    클릭을 `preventDefault`하고 외부 URL만 `openExternal`로 OS 브라우저에 열어 앱이 이동하지 않는다(D76). **스트리밍 중에는 하단 라이브니스 상태줄**(D60)이
    경과 시간을 1초 단위로 보여주고, 스트림이 15초 이상 조용하면 "마지막 응답 N초 전", 90초 이상이면
    "오래 걸리는 작업일 수 있음 — 멈춘 것 같으면 중지 후 재시도" 안내를 덧붙인다(장시간 코드베이스
    분석과 행을 구분). 에러에는 원문 +
    **한글 안내 힌트**(`errorHint`: TLS/인증서/스트림 끊김 시그니처 인식) + 1차 액션 **"다시 시도"**
    (같은 세션 재전송 — 실패 쌍 제거 + 워크플로우 커서 복원, D57) + 2차 액션 **"새 세션으로 다시
    시도"**를 함께 노출한다. codex의 `invalid peer certificate: BadSignature`는 codex CLI 자체의 사내
    TLS 프록시 인증서 불신 오류로, 앱은 안내+새 세션 복구만 제공한다([05](05-decisions.md) D28).
  - `CanvasPanel`: 툴바에 **요구사항 / 프롬프트 / 검색 결과 / 산출물 / 다이어그램 / 지식 저장 / 파일 + 열린
    파일별 뷰어 탭** 토글(D49/D58/D59/D78). **'파일' 탭은 파일 트리 목록 전용**(`listDir`, 지연 확장, `refreshNonce`로 문서 생성 후
    리로드)이고, 트리에서 파일을 클릭하면 그 파일의 **`file:<path>` 뷰어 탭**(`FileViewer`, 닫기 × 버튼)이
    생성·활성화된다 — 파일 목록과 문서 내용을 동시에 볼 필요 없이 탭으로 오간다. **워크플로우 단계가
    생성한 산출물은 파일 탭 대신 '산출물' 탭으로 라우팅**된다(D58 — D49 개정). 닫힌/없는 파일 탭은
    `effectiveTab`이 '파일'로 폴백하고, 새 세션/기록 열기 시 `openFiles`가 초기화된다.
    **'산출물' 탭**(`ArtifactsPanel`, D58)은 런타임 워크플로우의 문서 산출물을 좌측 목록(단계 상태 칩:
    대기/생성 중/완료/건너뜀/중단, 로드 세션은 생성됨/미생성) + 우측 미리보기(`FileViewer` 재사용)로
    집계하고, **'다이어그램' 탭**(`DiagramGallery`, D58)은 산출물 md의 ` ```mermaid ` 펜스를 lazy 스캔해
    카드 갤러리로 렌더한다 — 카드 클릭 시 **창 전체를 덮는 확대 모달**(확대/축소 컨트롤 0.25×~4×,
    CSS `zoom` 배율 + 스크롤, D60)이 열린다. 두 pill은 workdir 확정 + 산출물 ≥1일 때만 표시된다.
    산출물 탭 행에는 **hover '지식으로 저장' 액션**(존재하는 산출물만)이 있고, **'지식 저장' 탭**
    (`KnowledgeSavePanel`, D59)은 완료 배너/행 액션으로 열리는 조건부 탭이다 — 산출물 체크박스(존재
    프로브 `useArtifactExistence` 공용 훅, 미생성은 disabled)+제목+**격리 요약 턴**(오픈 시 자동 시작,
    ` ```summary ` 펜스 계약+평문 폴백, 편집 가능·저장 비블록)+저장(`save_knowledge_files`). entryId는
    세션당 고정이라 같은 세션 재저장은 upsert. 또는 요구사항 폼(`RequirementsForm`, **accent 카드 그리드** — [05](05-decisions.md) D35).
    **'요구사항' 탭 pill은 폼이 사용자 답변을 기다리는 동안만 렌더**되고, 제출/초기화로 `clarify`가 비면
    탭이 사라진다(`effectiveTab` 파생으로 pill 없는 상태 방어 — D41). **'검색 결과' 탭**은 rag 기반 단계가
    결과를 얻고 **관련성 판단 턴(D70)을 통과**했을 때만 나타나 세션 동안 유지되며, 클라이언트가 판단 턴이
    정리한 섹션(`ragCuratedHtml`, 관련 없다고 판단되면 탭 미표시; 판단 실패 시 fail-open으로 원본 `RagHit[]`
    → `ragResultHtml`)에서 생성한 이스케이프된 자립형 HTML을 sandbox iframe(srcdoc)으로 렌더한다(D46/D70).
    이 iframe도 `withLinkGuard`를 통과해 출처 링크 클릭이 앱 뷰를 이동시키지 않고 OS 브라우저로 열린다(D76).
    **'프롬프트' 탭**(D78)은 첫 작업 턴에서 에이전트가 낸 최적 프롬프트(내장 `prompt-optimizer` 스킬)를
    `end`에서 ` ```prompt ` 펜스로 파싱해 인메모리 평문(`PromptPanel`, 복사 버튼)으로 표시한다 — rag 탭처럼
    결과가 생기면 세션 동안 pill이 유지되고 도착 시 자동 전환되며, 새 세션/기록 열기 시 초기화된다. 프로젝트에 **코드베이스 경로**가 있으면 파일 탭
    툴바에 **루트 전환 세그먼트(작업 폴더 ↔ 코드베이스)**가 나타나 분석 대상 코드베이스를 브라우징할
    수 있다(트리 `key=treeRoot`로 리셋 — D45). 폴더 준비 전(auto 프로젝트, 첫 send 전)에는 "첫 메시지를
    보내면 폴더가 자동 생성됨" 안내만(버튼 없음 — 폴더 지정은 홈에서). 홈에서 폴더를 지정했으면 진입
    즉시 그 폴더의 파일을 표시.
- **TopBar**: 로고·제목·배지만 표시하며 **작업 폴더는 표시하지 않는다**(R1). 활성 프로젝트 폴더는 캔버스
  툴바의 폴더 칩이 보여준다 — **실제 절대경로**를 좌측 말줄임으로 표시하고 툴팁이 전체 경로를
  보여준다(D60; basename만 보이던 "workspace" 표기 대체). 폴더 칩은 툴바에서 **유일하게 축소 허용**
  (`min-w-0`/`max-w-[240px]`)되어 긴 경로가 좌측 버튼(루트 전환·새로고침·탐색기)을 짓누르지 않으며,
  칩 오른쪽의 **"탐색기에서 열기"** 버튼(`ExternalLink`)이 현재 트리 루트를 `openInExplorer`(opener
  `openPath`)로 OS 탐색기에 연다(D69).

## 카테고리 워크플로우 (`lib/options.ts` + `lib/skills.ts` + `lib/workflow.ts` + `lib/clarify.ts` + 캔버스)

카테고리마다 **고정 선택지 우선 시작 → 기반 3단계(필수) → 단계 오케스트레이터(단계별 스킬 주입)**를
클라이언트에서 구동한다([05](05-decisions.md) D34/D36/D39/D40/D44). 단계·스킬의 **정의**는 Flows 설정
화면에서 사용자가 등록하고 `settings.json`에 영속화되며(`set_skills`/`set_workflow` —
[02](02-architecture.md)), 코드 내 기본값(`DEFAULT_WORKFLOWS`/`DEFAULT_SKILLS`)이 폴백=샘플이다. 실행은
여전히 도구 채널 없는 클라이언트 오케스트레이션(D30 재확인). 상세 편집 가이드는
[08-guided-flows-and-skills.md](08-guided-flows-and-skills.md).

- **⓪ 카테고리 자동 분류(옵션 이전, D81)** — 홈 컴포저로 프롬프트를 입력해 시작하면(카드 클릭 아님)
  카테고리가 잠정 `plan` + `autoCategory=true`로 진입한다. `WorkspaceView`가 워크스페이스 진입 시
  `ensureProject`로 cwd를 확정한 뒤 `lib/categorize.ts::classifyCategory`(격리 에이전트 턴 —
  `judgeRagRelevance`(D70) 패턴 복제, `` ```category `` fenced JSON `{category, reason}` 파싱, 실패 시
  `null`)로 프롬프트에 가장 맞는 카테고리를 정하고, 그 카테고리로 `ChatPanel`을 **1회 마운트**한다(리마운트
  없음). 분류 중에는 채팅 컬럼에 "작업 유형 분석 중…+중지" placeholder를 보인다. 실패/취소/`null` →
  `plan` 폴백. 카테고리 카드·loaded session·빈 프롬프트는 분류하지 않고 그 카테고리로 바로 마운트한다.
- **① 고정 선택지(옵션, 프리플로우)** — `lib/options.ts`의 `CATEGORY_OPTIONS: Record<Category,
  ClarifyQuestion[]>` + `optionsFor(category, settings)`. 카테고리(자동 분류된 경우 포함) 진입 시
  **즉시**(에이전트 대기 없이) `onClarify`로 캔버스 '요구사항' 폼에 렌더한다. **기반 단계가 활성인 카테고리는 폼 맨 앞에 필수
  `codebasePath` folder 질문**(네이티브 폴더 픽커)이 프리펜드된다(D45) — 답변은 wire가 아니라
  구조적으로(`WorkspaceView.codebasePath` 상태) 전달·영속된다. 홈 프롬프트로 시작했으면 **숨김 프리필
  턴**을 격리 실행(세션 id/resume 미사용, 영속화·스텝커서 불변; `send(_,{prefill:true})`)해
  `prefillInstruction`으로 아는 항목만 ` ```prefill ` JSON으로 채우고(**folder 질문은 프리필 제외**)
  `parsePrefill`로 검증 후 `onPrefill`로 폼을 미리 채운다. 폼 제출이 **첫 작업 턴**을 발사한다. 옵션이 빈
  카테고리는 이 단계를 건너뛰고 seed를 첫 작업 턴으로 자동 전송. **폼이 답변을 기다리는 동안 채팅 컴포저는
  차단**되고(프리필/자동전진 턴만 실행), '요구사항' 탭은 이 대기 동안만 표시된다(D41).
- **①.5 기반 3단계(foundation, D44)** — 워크플로우 맨 앞에 고정(pinned)되는 필수 단계:
  `codebase`(코드베이스 분석 — 매 턴 `extraDirs`로 접근 부여(claude `--add-dir`, gemini
  `--include-directories` — D52; 원격 fabrix·aipro는 파일 접근이 없어 실독 불가, D67/D71) + 절대경로
  컨텍스트(`pathContext`)가 "이 폴더에서 탐색 시작"을 지시) →
  `rag`(preflight가 `rag_search`로 사내 문서 발췌를 조회한 뒤 **관련성 판단 턴**(D70)이 이번 작업과의
  관련성을 판정 — 관련 없으면 표시·주입 없이 건너뛰고, 관련 있으면 정리된 캔버스 '검색 결과' 탭 표시 +
  발췌를 wire에 첨부) → `knowledge`(preflight가 `list_knowledge` 항목을 16KB 상한으로 주입;
  **artifact 엔트리(D59)는 요약+첨부 문서 절대경로 인덱스로 주입**되고 `knowledge/artifacts` 루트가
  extraDirs에 등록되어 에이전트가 원문 전체를 직접 읽는다).
  rag/knowledge는 미설정·0건·실패 시 **에이전트 턴 없이 건너뛴다**(system 안내 + 커서 전진 + 다음
  생성형 단계로 체인; preflight 중 Stop은 취소 폴백). `plan`은 항상 활성, 그 외 카테고리는 Flows
  토글(저장 배열에 기반 kind 존재 = 플래그). **기반 단계는 완전 트리오가 필수는 아니다** — 카테고리별
  `CATEGORY_FOUNDATION`이 pin할 종류를 정하며(`guide`는 코드베이스를 뺀 **rag+knowledge**만 — D63),
  `coerceSteps`가 그 종류들을 canonical 순서로 pin한다(codebase가 없으면 필수 폴더 질문·루트 전환도 없음).
  이후 기존 설정 단계들이 오늘과 동일하게 이어진다.
- **② 단계별 스킬** — `lib/skills.ts`의 `resolveSkills(settings)`(사용자 레지스트리 ?? `DEFAULT_SKILLS`).
  각 단계의 `skillIds`가 가리키는 스킬 body들을 **그 단계가 armed된 턴**의 wire 앞(스텝 지시문 위)에
  주입한다. 세션형은 같은 스킬을 대화당 1회만(dedupe, 전송 실패 시 되감기), 세션리스는 transcript로 자연
  재노출. 알 수 없는 id는 무시. CLI 자체 스킬과 무관한 앱/사용자 지시문(D40).
- **③ 워크플로우 단계(`lib/workflow.ts`)**: `StepDef{id,name,kind,instruction,file?,skillIds,output?}` +
  `workflowFor(category, settings)`(사용자 override ?? `DEFAULT_WORKFLOWS`, `coerceSteps(steps,
  {foundationKinds})`로 방어 — 잘못된 항목 드롭 + **기반 단계 pinned 프리펜드**(`mandatoryFoundation`이
  준 종류를 canonical 순서로, 사용자 편집 병합·누락 보충) + 종단 `chat` 자동 보강). 기본 `plan` = `[코드베이스 분석(codebase, docs/codebase-analysis.md) →
  사내 문서 RAG 검색(rag) → 지식 베이스 반영(knowledge) → 소스코드 분석(document,
  docs/source-analysis.md) → 계획 생성(document, docs/plan.md) → 변경영향분석서 생성(document,
  docs/impact-analysis.md) → 테스트 계획서 생성(document, docs/test-plan.md) → 마무리 대화(chat)]`
  (기존 컨플루언스 search 단계는 rag 기반 단계로 대체). 기본 `query`(데이터 조회, D61) = `[참조 SQL·
  테이블 사용처 탐색(codebase, output:file, docs/query-references.md) → 사내 문서 RAG 검색(rag) →
  지식 베이스 반영(knowledge) → 테이블 정보·ERD 정리(document, docs/table-info.md — mermaid erDiagram)
  → 참고 SQL 작성(document, docs/query-sql.md) → 마무리 대화(chat)]`(기반 3단계 기본 활성). 기본
  `change`(데이터 변경·권한, D62) = `[변경 대상·영향 탐색(codebase, output:file, docs/change-references.md) →
  사내 문서 RAG 검색(rag) → 지식 베이스 반영(knowledge) → 테이블 정보·ERD 정리(document,
  docs/change-table-info.md — mermaid erDiagram) → DC Manager 신청양식 생성(document,
  docs/dc-manager-form.html — 초반 선택한 변경 종류에 맞춰 `dc-manager-form` 스킬이 폼 분기) →
  마무리 대화(chat)]`(기반 3단계 기본 활성). 기본 `guide`(운영 가이드 생성, D63) = `[사내 문서 RAG
  검색(rag) → 지식 베이스 반영(knowledge) → 운영 가이드 작성(document, docs/operation-guide.md,
  output:html) → 마무리 대화(chat)]` — **코드베이스 단계를 제외한 부분 foundation(rag+knowledge)**으로
  Confluence/지식 시각화(rag '검색 결과' 탭 D46)를 강점으로 삼고, `output:"html"`이 보기 좋은
  operation-guide.html을 자동 생성한다. `kind`: `search`·`document`·기반 3종(생성형,
  `isGenerative`) / `chat`(종단). **`output`("chat"/"file"/"html", D47)**: 미지정 시 kind에서 파생;
  오케스트레이터는 `runtimeWorkflowFor` = `expandOutputSteps(workflowFor(...))`를 실행해 `"html"` 단계
  뒤에 `html-render` 스킬을 단 합성 렌더 서브스텝(`<file>.html`)을 삽입한다(편집기는 미확장 뷰만 봄).
  진행 노트는 `progressLabel(i, steps)` = "N/M단계 · <name> 중…"으로 **파생**(고정 문자열 없음).
- **커서(ChatPanel)**: `stepIndexRef`(현재 단계) + `stepArmedRef`(단계당 1회 주입) +
  `injectedSkillIdsRef`(세션형 스킬 dedupe) + `inflightStepRef`(파싱 대상 턴) + `prefillInflightRef`(프리필
  턴 표시) + `promptSkillPendingRef`/`promptInflightRef`(내장 프롬프트 최적화 스킬 — 첫 실제 작업 턴 1회
  주입 + `end`에서 ` ```prompt ` 파싱, D78) + `codebasePathRef`/`skillDirsRef`(extraDirs 소스) +
  `preflightAbortRef`(preflight 중 Stop).
  `WF`/스킬 맵은 **마운트 시 고정**(remount가 리셋). 로드 세션은 커서를 끝으로 시작 → 일반
  대화(재주입 없음). `send()`는 **workdir 확정(`ensure_project`)을 preflight·wire 조립보다 먼저** 수행해
  첫 턴에도 절대경로를 알 수 있다(D52). wire 조립은 `[내장 프롬프트 최적화 스킬(첫 실제 작업 턴 1회,
  최상단 — D78) → step.skillIds의 스킬 body들(armed, 세션형 dedupe; dir 있으면 리소스 폴더 안내 부착) →
  step.instruction(armed) → 경로 컨텍스트(`pathContext` — 생성형 단계: 작업 폴더/코드베이스 절대경로 +
  탐색 시작점 지시, D52) → preflight 컨텍스트(기반 단계) → prompt]`.
  메시지 상태 변경은 전부 **동기 커밋 헬퍼(`mutateMessages`)** 경유(stale ref로 인한 응답 소실 방지,
  D55); 폼 제출/자동전진 nonce는 `send()`가 턴을 실제로 처리했을 때만 소비된다(스트리밍 중 제출은
  유실되지 않고 재시도 — D55). 커서와 나란히 **`stepProgress` state**(단계별 진행 상태)가 갱신되어
  `WorkflowStepper`에 표시되고, `lastTurnRef`(마지막 실전송 턴)가 실패 시 **같은 세션 재시도**의
  소스가 된다(둘 다 transient — D57).
- **`end` 분기**:
  - 프리필 턴: `parsePrefill` → `onPrefill`로 폼 채움 → 프리필 메시지쌍 제거. 스텝/스킬/영속화 불변.
  - 생성형(`isGenerative` — search/document/기반 3종): `file`이 있으면 `onOpenFile(join(cwd,file))`로
    캔버스에 연다(워크플로우 산출물은 '산출물' 탭에서 선택 — D58). **원격 에이전트(Fabrix)는 파일을 쓰지
    못하므로, 이 지점에서 앱이 마지막 어시스턴트 텍스트를 `extractDocBody`로 정제해 `write_file`로
    `step.file`에 저장한 뒤 열어야** 존재 프로브가 산출물을 찾는다(CLI는 자기 파일을 이미 씀 — 무변경;
    `detected[agentId].source==="remote"` 게이트, D67). → 다음 단계가 생성형이면 **자동 진행**
    (`setAutoTurn` nonce+effect; stale 클로저 회피), 종단 `chat`이면 멈춤. 검색 근거는 스트리밍되는
    `toolUse`/`toolResult`(에이전트 Grep/Read)로 표시된다.
  - `chat`: 종단(전진 없음). 취소/실패 → 워크플로우 중단(일반 대화). 단, **생성형 단계 턴이 일시적
    오류(타임아웃/스트림/네트워크 시그니처)로 실패하면 중단 전에 같은 단계를 자동으로 최대 2회
    재시도**한다(실패 쌍 제거 + 단계 re-arm + `setAutoTurn` 재발사; aipro 백엔드의 간헐적 45초 first-byte
    타임아웃 흡수 — D68). 치명적 오류(Model not found·TLS·인증/가드레일)와 사용자 취소는 재시도하지 않고
    즉시 중단. 소진 시 halt 노트 + 수동 '다시 시도'(D57).
- **리프트(WorkspaceView)**: `clarify`(옵션 질문)/`clarifyPrefill`(자동채움 답변)/`prefillNonce`/`canvasTab`/
  `streaming`/`answerSubmission` + `refreshNonce`(문서 생성 후 트리 리로드) + `resolvedWorkdir` +
  `codebasePath`(프로젝트 스코프 — 새 세션에도 유지) + `ragResult`(검색 결과 탭 HTML) +
  `artifacts`/`artifactSel`/`stepProgress` 미러(산출물 탭, D58 — ChatPanel의 `onStepProgress` 콜백) 소유.
  `handleClarify`·`handlePrefill`는 캔버스 탭을 '요구사항'으로 전환, `handleOpenFile`은 산출물이면
  '산출물' 탭 선택(D58), 아니면 '파일' 탭 전환 + 트리 리로드 + 파일 선택. 새 세션/기록 열기 시 초기화.
- **폼(RequirementsForm)**: single/multi = **accent 카드 그리드**(D35), text=`useAutoGrow`, `required` 검증,
  스트리밍/제출 중 비활성, `initialAnswers`로 프리필 반영(`prefillNonce`로 재초기화). 제출 →
  `formatClarifyAnswers`가 `{wire, display}` 생성 → `answerSubmission`(nonce) → ChatPanel effect가 원 요청을
  덧붙여 `send`로 첫 작업 턴(skill+step[0]) 발사. 채팅엔 원 요청(또는 압축 요약)만 표시.
- **안전장치**: 단조 커서 + 1회성 arm(단계) + 스킬 dedupe(세션형) + `succeeded`에만 자동발사 + 종단 `chat`
  (저장 검증 + `coerceSteps` 이중 보장) + 프리필/파싱 실패 폴백으로 무한진행·깨짐 방지. 일시적 실패
  자동 재시도는 **단계당 최대 2회**로 상한(치명적 오류·취소 제외)이라 무한 재시도가 되지 않는다(D68).
- **영속화**: 단계·스킬의 **정의**는 `settings.json`(D39), 옵션 카탈로그는 코드 정적. 실행 상태(단계 커서/
  주입 이력/대기 폼)는 **transient**(저장 안 함). 생성된 파일만 실제로 남는다. 저장 세션 재오픈 시 폼·커서
  없이 일반 대화로 이어간다.
- **에이전트별**: claude/codex는 동일 세션 resume로 스킬·모든 단계 연속(프리필은 격리 세션). gemini/aipro
  (세션리스)는 transcript 재전송이라 스킬/지시문 lossy·크기 증가. opencode/antigravity(plain)는 도구/파일쓰기
  보장이 약해 degrade.

## 범위 밖 (후속 증분)

- 디자인의 5개 캔버스 아티팩트 탭(저장소 분석/영향도/변경 가이드 등 전용 뷰) — **산출물 허브·다이어그램
  탭으로 집계 뷰는 구현됨(D58)**; 아티팩트별 전용 뷰/오케스트레이션은 후속. (요구사항 명확화·소스 조사·계획서
  생성은 `plan`, 참조 SQL·ERD·참고 SQL 산출은 `query`, 변경 종류별 DC Manager 신청양식(HTML) 생성은
  `change`에 한해 위 "카테고리 워크플로우"로 구현됨.)
- plan·query·change·guide는 다단계 기본값을 가짐(D40/D61/D62/D63; guide는 코드베이스를 뺀 부분
  foundation). 사용자는 Flows 설정에서 어느 카테고리든 단계를 추가/편집할 수 있음(D39).
- guide의 실제 운영 작업 자동화(가이드에 따른 작업 실행 연동) — 범위 밖(가이드 문서 산출까지).
- 실제 데이터 조회 실행·결과 표시(현재 `query`는 참고용 SQL 산출까지 — D61). 실제 변경 실행·승인 연동
  (현재 `change`는 DC Manager 신청양식 산출까지 — D62).
- 대화는 파일(JSON)로 영구화된다(위 "세션/프로젝트 영속화"). SQLite/전문 검색/여러 프로젝트 목록
  화면은 후속 증분.
- opencode·antigravity의 1급 실행 파서(현재 plain 폴백). codex/gemini는 1급 지원됨. fabrix·aipro는
  원격 HTTP+SSE(텍스트 스트림만).
- gemini/aipro의 CLI 세션 재개(세션 미지원 → transcript 재전송으로 대체).
- codex TLS 인증서 오류(BadSignature)의 앱 내 해결(환경변수/CA 주입) — 환경/OS 레벨 이슈로 제외
  ([05](05-decisions.md) D28).

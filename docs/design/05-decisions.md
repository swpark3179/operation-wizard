# 05. 주요 결정 로그 (Decision Log)

설계상 의미 있는 선택을 결정·근거·대안 형태로 기록한다.
새 결정이 생기면 이 목록 끝에 추가한다.

---

### D1. 앱 셸로 Tauri v2 채택
- **결정**: Electron이 아닌 **Tauri v2**(Rust + 시스템 WebView2)로 데스크톱 앱 구성.
- **근거**: 가벼운 번들/메모리, OS 프로세스·파일 제어를 Rust로 안전하게 수행,
  Windows 네이티브 동작(실행파일 해석·프로세스 실행)이 핵심이므로 Rust 백엔드가 적합.
- **영향**: Rust 빌드에 MSVC 툴체인 필요 → [06-build-and-environment.md](06-build-and-environment.md).

### D2. 탐지 로직을 "처음부터 설계"하지 않고 Open Design 데몬을 Rust로 포팅
- **결정**: Open Design `apps/daemon/src/runtimes/`의 검증된 동작을 Rust로 재구현.
- **근거**: 실행파일 해석/shim 처리/모델 파싱은 엣지케이스가 많고 이미 검증됨.
  바닥부터 재발명하지 않고 사양을 그대로 옮기는 편이 안전.
- **영향**: 함수/상수 이름과 동작을 원본과 1:1 대응시켜 추적성을 유지(코드 주석에 원본 위치 명시).

### D3. 실행파일 해석에 PATH + 잘 알려진 툴체인 디렉터리 + PATHEXT
- **결정**: PATH만 믿지 않고 npm 전역/scoop/bun/cargo/deno/volta/fnm/`~/.opencode/bin`을
  명시 보강하고, `PATHEXT`(+확장자 없음)로 후보를 확장.
- **근거**: GUI/패키징 앱은 stripped PATH로 뜨는 경우가 많아 설치돼 있어도 못 찾는 문제 발생.
- **대안 기각**: "PATH만 검색" → 실사용 환경에서 미탐지 빈번.

### D4. `.cmd`/`.bat` shim은 `cmd.exe /d /s /c`로 실행
- **결정**: shim 실행 시 직접 spawn하지 않고 `cmd.exe`로 감싼다.
- **근거**: Rust 표준 라이브러리의 BatBadBut 완화책이 `.bat/.cmd` 인자 전달을 거부함.
  실제 `.exe`인 `cmd.exe`를 거치면 우회 가능.
- **세부 규칙**: 공백 경로 따옴표가 `/s` 규칙에 먹히지 않도록 **항상 인자 ≥ 1개**로 호출.
  (자세히: [03-agent-detection.md](03-agent-detection.md))

### D5. 프로브는 타임아웃 + 콘솔 비표시 + 파이프 드레인
- **결정**: `--version` 3초 / `models` 15초 타임아웃, `CREATE_NO_WINDOW`,
  stdout/stderr를 스레드로 끝까지 읽기.
- **근거**: UI 멈춤·콘솔 깜빡임·대용량 출력 파이프 데드락을 방지.

### D6. `models` 실패 시 정적 fallback 카탈로그 제공
- **결정**: live 목록을 못 얻으면 고정 모델 목록을 보여주고 `fallback`로 표기.
- **근거**: 오프라인/오류 상황에서도 UI가 의미 있는 선택지를 제공. 출처를 명확히 구분.

### D7. 탐지 커맨드는 async + spawn_blocking
- **결정**: `detect_opencode`를 `async`로 두되 실제 작업은 블로킹 스레드에서 수행.
- **근거**: 최대 15초까지 걸릴 수 있는 `models` 프로브가 UI/IPC 스레드를 막지 않도록.

### D8. 사용자 지정 경로를 `settings.json`에 영구화 (`OPENCODE_BIN` 등가물)
- **결정**: 자동 탐지 보완 수단으로 사용자 지정 경로를 앱 config 디렉터리에 저장.
- **근거**: 사내망/비표준 설치 환경에서 자동 탐지가 실패할 수 있음.
  환경변수 `OPENCODE_BIN`과 동일한 의미로 제공해 일관성 확보.

### D9. 디자인 시스템은 Open Design 토큰을 Tailwind v4 `@theme`로 매핑
- **결정**: CSS 변수 토큰을 단일 출처로 두고 Tailwind 유틸리티에 inline 매핑.
- **근거**: 라이트/다크 자동 전환, 토큰만 바꾸면 전역 테마 변경, 의미 기반 클래스로 일관성.
  (자세히: [04-ui-and-design-system.md](04-ui-and-design-system.md))

### D10. 프론트 타입은 백엔드 serde 구조체의 수동 미러
- **결정**: 코드 생성 도구 없이 `lib/types.ts`로 수동 미러링, 직렬화는 `camelCase`.
- **근거**: v0.1 규모에서 도구 도입 비용이 과함. 대신 "한쪽 변경 시 양쪽 동기화" 규칙으로 관리.
- **재검토 조건**: 커맨드/모델이 늘어 동기화 부담이 커지면 `ts-rs` 등 자동화 검토.

### D11. 다중 에이전트를 "정의(def) + 레지스트리"로 일반화
- **결정**: OpenCode 전용 하드코딩을 걷어내고, `agents.rs`의 정적 `AGENT_DEFS`(opencode,
  claude, codex, gemini, antigravity)와 `detect_agent_blocking(def, custom)` 공통 파이프라인으로 전환.
  커맨드도 `list_agents`/`detect_agent`/`set_agent_bin`으로 일반화.
- **근거**: Open Design의 검증된 "def + 공통 probe" 구조를 그대로 따름. 새 에이전트 추가 비용이
  "레지스트리 항목 1개(+필요 시 파서)"로 최소화되고 프론트는 자동 반영.
- **세부**: `AgentDef`는 `static`(const 아님)으로 두어 `&'static AgentDef`를 `spawn_blocking`
  스레드로 그대로 넘긴다. version args는 5개 모두 `--version`이라 def 필드로 두지 않고 하드코딩.

### D12. claude/gemini/antigravity는 정적 fallback 모델 전용
- **결정**: 이 세 에이전트는 줄/JSON 기반 모델 나열 명령이 없어 `models_probe = None` → 항상 fallback.
  특히 claude는 Open Design에서 **MMS 라우트 fetch**로 라이브 목록을 얻지만, 이는 자체 프록시
  인프라 의존이라 이 로컬 탐지 앱에서는 **포팅하지 않는다**.
- **근거**: 로컬 단독 탐지 도구에는 해당 인프라가 없음. 오프라인에서도 의미 있는 선택지를 제공하고
  출처를 `fallback`으로 명확히 표기.
- **재검토 조건**: 각 CLI가 로컬에서 모델을 나열하는 안정적 명령을 제공하면 `models_probe` 추가.

### D13. codex 모델은 `debug models` JSON 파서로 처리
- **결정**: codex는 `debug models`가 JSON(`{models:[…]}`)을 내므로 `parse_codex_debug_models`
  (serde_json) 추가. `visibility=="hidden"` 스킵, id=`slug`||`id`, label=`display_name`||`name`||id,
  중복 제거, 유효 0개면 fallback. 파서 시그니처는 `fn(&str)->Option<Vec<ModelOption>>`로 통일.
- **근거**: 에이전트마다 모델 출력 형식이 달라(line vs JSON) 파서를 def에 주입하는 편이 깔끔.
  `serde_json`은 이미 의존성이라 추가 비용 없음.

### D14. 설정은 에이전트별 경로 맵 + 레거시 마이그레이션
- **결정**: `Settings`를 `{ agents: HashMap<id, { customBin }> }`로 변경. v0.1의 단일
  `opencodeBin`은 `skip_serializing` 레거시 필드로 받아 load 시 `agents.opencode`로 흡수(다음 save에서 제거).
- **근거**: N개 에이전트의 사용자 지정 경로를 보관해야 함. 마이그레이션으로 기존 테스터의 경로 보존(self-healing).

### D15. antigravity에 `ANTIGRAVITY_BIN` env override 추가 (upstream 편차)
- **결정**: Open Design은 antigravity(`agy`)에 env override를 매핑하지 않지만, 다른 에이전트와
  일관성을 위해 `ANTIGRAVITY_BIN`을 추가.
- **근거**: 무해하고 일관적. 사용자 지정 경로(settings)가 1차 수단이며 env는 보조 수단.

### D16. 에이전트별 검색 디렉터리는 슬라이스, IPC id는 검증
- **결정**: def별 추가 검색 경로를 `extra_search_subdirs: &[&str]`(없으면 `[]`)로 두고, 기존
  하드코딩 `.opencode\bin`을 opencode def로 이전. `detect_agent`/`set_agent_bin`은 알 수 없는
  `agentId`를 unwrap하지 않고 에러로 반환.
- **근거**: 공통 툴체인 경로는 공유하되 에이전트 고유 설치 위치는 def로 확장 가능. IPC는 임의
  문자열이 들어올 수 있어 방어적 검증이 필요.

### D17. 사내 도구 AI Pro는 빌트인 def로 추가 (런타임 프로필 로더 미포팅)
- **결정**: 사내 CLI 도구 **AI Pro**(`aipro`, Gemini CLI 호환)를 `AGENT_DEFS`에 6번째 빌트인
  `AgentDef`로 추가. Open Design에서는 `~/.open-design/agents.local.json`의 `baseAgent: "gemini"`
  로컬 프로필이었지만, **런타임 JSON 프로필 로더는 포팅하지 않는다**(사용자 선택: 최소 변경).
- **근거**: 현 시점 요구는 "AI Pro 1종 등록"뿐이라 정적 def 추가(약 10줄)가 가장 단순하고
  기존 5개와 동일 경로로 검증됨. gemini 호환이라 `models_probe = None`(fallback 전용),
  fallback = `glm-5.1`/`qwen3.6-27b`/`gpt-oss-120b`. 프로필의 `env`
  (`GEMINI_CLI_TRUST_WORKSPACE`)는 **에이전트 실행 시점** spawn env라 탐지 범위와 무관 → 미반영.
  `AIPRO_BIN`은 다른 에이전트와의 `*_BIN` 일관성용 보조 override로 추가.
- **재검토 조건**: 사내 에이전트가 늘거나 재빌드 없이 추가/수정해야 하면, Open Design의
  `readLocalAgentProfileDefs`(`agents.local.json` + `baseAgent` 상속)를 포팅하고 레지스트리를
  동적(`OnceLock<Vec<AgentDef>>` 등)으로 전환한다. 그때 AI Pro도 프로필로 이전 가능.

---

### D18. 실행(run) 엔진은 SSE 데몬이 아닌 네이티브 Tauri `Channel`
- **결정**: Open Design 데몬의 run/stream 절반(`runs.ts`/`defs/claude.ts`/`claude-stream.ts`)을
  `run.rs`로 포팅하되, HTTP+SSE 전송을 걷어내고 **`tauri::ipc::Channel<RunEvent>`**로 스트리밍.
  `run_agent`는 워커 스레드에서 자식 프로세스를 spawn하고 `runId`를 즉시 반환, `cancel_run`으로 취소.
  취소용 인메모리 `RunRegistry`(`Mutex<HashMap>`)를 managed state로 둔다. (상세: [07](07-workspace-and-runs.md).)
- **근거**: 로컬 단독 데스크톱 앱에는 장수 HTTP 서버가 불필요. Channel이 웹뷰로의 1:1 스트리밍에
  가장 관용적이며, **새 Cargo 의존성 0**(std 프로세스 + reader 스레드는 `exec.rs`와 동일 패턴).
- **세부**: `.cmd`/`.bat` shim 래핑 + `CREATE_NO_WINDOW`는 `exec.rs::command_for`로 추출해
  탐지(run_capture)와 실행(run.rs)이 공유.

### D19. Claude Code 1급 지원 + 나머지는 plain 폴백
- **결정**: 이번 증분은 **Claude Code**만 완전한 `stream-json` 파서(text/thinking/tool-call)로
  구동하고, 나머지 5종은 `RunSpec::plain()`(=`-p "<prompt>"` + 원시 stdout)로 best-effort 실행.
- **근거**: Claude는 구조화 스트림이 가장 풍부하고 open-design 레퍼런스가 있어 검증 용이. 모든
  에이전트의 파서(codex JSON, gemini/opencode json-event-stream 등)를 한 번에 포팅하는 비용은 과함.
- **한계**: plain 폴백은 `.cmd` shim + cmd 메타문자(`&|><`) 조합에서 취약할 수 있음(문서화된 제약).
- **재검토 조건**: 각 CLI를 실사용하게 되면 `StreamFormat`을 넓히고 전용 파서를 `run.rs`에 추가.

### D20. 세션 재개는 capture-style (uuid 크레이트 없이)
- **결정**: Claude의 `system/init`이 내는 `session_id`를 `status` 이벤트로 프론트가 캡처·보관하고,
  다음 턴에 `RunArgs.sessionId` → `--resume <id>`로 이어간다(첫 턴은 `--session-id` 미지정).
- **근거**: Open Design은 첫 턴에 UUID를 mint해 `--session-id`로 주입하지만, capture 방식이면
  `uuid` 의존성이 필요 없고 로컬 앱에 충분. codex의 capture-style과 같은 접근.

### D21. 캔버스 파일 접근은 `fs` 플러그인 대신 커스텀 커맨드
- **결정**: 파일 뷰어에 필요한 read-only 두 작업만 `files.rs`의 `list_dir`/`read_file` 커맨드로 제공
  (`fs` 플러그인 + capability 표면 미도입). `read_file`은 2 MiB 상한.
- **근거**: 필요한 표면이 극소이고, 플러그인 권한/스코프 설정 비용을 피함. HTML 미리보기는
  클라이언트에서 샌드박스 iframe(`allow-scripts`, same-origin 없음)로 렌더.

### D22. 증분 1 = HOME 런처 + 워크스페이스 셸 + 최소 파일 뷰어 캔버스
- **결정**: 임포트한 디자인(`Operation Wizard.dc.html`)의 HOME 런처(프롬프트 + 4 카테고리 + 최근)와
  좌 대화 / 우 캔버스 워크스페이스 셸을 충실히 구현하고 대화를 **실제 에이전트에 연결**하되,
  캔버스는 **작업 폴더 파일 트리 + 뷰어 + HTML 미리보기**로 최소 구현한다. 디자인의 5개 아티팩트
  탭(저장소 분석/영향도/계획서/변경 가이드/문서)과 스크립트형 요구사항 명확화 플로우는 **후속 증분**.
- **근거**: "핵심부터 차례대로"(사용자 선택). 아티팩트 탭은 아티팩트를 생성하는 에이전트
  오케스트레이션 파이프라인이 선행되어야 하므로 셸+연결을 먼저 세운다.
- **재검토 조건**: 에이전트가 구조화된 산출물(분석/계획/diff)을 낼 수 있게 되면, 캔버스를 파일
  뷰어에서 5탭 아티팩트 워크스페이스로 확장하고 `step` 진행 모델을 도입.

---

### D23. codex·gemini·aipro 1급 실행 (json-event-stream 파서 포팅)
- **결정**: 증분 1의 plain 폴백(`-p "<prompt>"`)이 codex에선 `--profile`과 충돌하고 aipro에선
  모델 미전달로 실패하는 문제를 해결하기 위해, Open Design `defs/{codex,gemini}.ts` +
  `json-event-stream.ts`를 포팅해 세 에이전트를 1급 지원한다. codex=`exec [resume] --json`
  +프롬프트 stdin +`--model` +Windows 샌드박스, gemini/aipro=`--output-format stream-json --yolo`
  +프롬프트 stdin +`--model` +env `GEMINI_CLI_TRUST_WORKSPACE=true`. `StreamFormat`에 `CodexJson`/
  `GeminiJson`, `RunSpec`에 `prompt_format`/`env` 추가. 파서는 `run.rs`의 `parse_codex_event_line`/
  `parse_gemini_event_line`(단위 테스트).
- **근거**: 두 CLI 모두 `stream-json`/`--json` 출력이라 raw로 흘리면 JSON 노이즈가 되고, codex의
  세션 이어가기(D25)에는 스트림의 `thread_id` 캡처가 필요하다. plain으로는 두 문제를 못 푼다.
- **세부**: aipro는 모델 미지정/`default`면 사내 모델 `glm-5.1`을 강제로 `--model`에 넣어
  "Model not found"를 방지한다. opencode·antigravity는 이번 범위 밖(계속 `Plain`).
- **재검토 조건**: aipro가 `--output-format stream-json`을 지원하지 않으면 aipro만 텍스트 모드로 조정.

### D24. Windows 취소는 프로세스 트리 종료(`taskkill /T /F`)
- **결정**: `cancel_run`이 자식만 `kill`하지 않고, 자식 pid로 `taskkill /PID <pid> /T /F`를 실행해
  프로세스 트리 전체를 종료한다(실패 대비 `child.kill()` 병행).
- **근거**: 에이전트는 npm `.cmd` shim → `cmd.exe` → node 손자 구조라, 직접 자식(cmd.exe)만 kill하면
  실제 에이전트(node)가 살아남아 "정지"가 동작하지 않는다. Open Design도 Windows에선 트리 종료를
  하지 않아(직접 kill만) 동일 누수가 있으며, 이 앱에서 보강한 편차다.

### D25. Claude 세션은 capture가 아니라 client-mint (D20 갱신)
- **결정**: Claude 세션 id를 스트림에서 캡처하지 않고, 프론트가 대화 시작 시 `crypto.randomUUID()`로
  **미리 mint**해 첫 턴 `--session-id`, 이후 `--resume`으로 넘긴다.
- **근거**: 정지(취소) 요구(#4) 때문. capture 방식은 취소가 `system/init` 전에 일어나면 세션 id를
  못 얻어 이어가기가 실패한다. mint면 취소 시점과 무관하게 항상 재개 가능. `uuid` 크레이트는
  여전히 불필요(웹뷰 `crypto.randomUUID`). codex는 여전히 capture(`thread_id`), gemini/aipro는
  세션 미지원이라 transcript 재전송.

---

### D26. 대화 영속화 = 파일 기반, 프로젝트=작업 폴더 (`projects.rs`) — **D32로 대체됨**
> ⚠️ 프로젝트=작업 폴더(결정적 id) 부분은 **D32로 대체**되었다. 파일 기반 영속화·데이터 모델·새 의존성 0
> 원칙은 유지된다. 아래는 최초 결정의 기록이다.

- **결정**: 인메모리 대화를 디스크에 저장한다. 루트 `~/.operation-wizard/projects/<projectId>/`,
  대화는 `sessions/<sessionId>/session.json`. **프로젝트 = 작업 폴더(workdir)**이며 `projectId`는
  workdir 경로에서 **결정적으로**(정규화 후 `<basename>-<fnv1a8>`) 생성한다. 홈 루트는 앱이 이미
  쓰는 `std::env::var("USERPROFILE")` 패턴을 재사용(Windows). 첫 질문 시 폴더가 생성되고 매 턴
  `end`마다 재저장된다. 데이터 모델은 `Project`/`SessionMeta`/`StoredSession`(serde `camelCase`).
- **근거**: "프로젝트별 세션 기록 열람/이어가기" 요구를 자연스럽게 만족(workdir 스코프). 결정적 id는
  조회/매핑 파일을 없애 `ensure_project`를 idempotent하게 한다. **새 Cargo 의존성 0** — id는 프론트
  `crypto.randomUUID()`, 타임스탬프는 `SystemTime`, 메시지는 `serde_json::Value`로 보관해 백엔드가
  프론트 `ChatMessage` 형태에 결합되지 않는다. 핵심 fn은 `root: &Path`(=`settings.rs` 스타일)로 두어
  temp root 단위 테스트.
- **대안 기각**: "홈 진입마다 새 프로젝트" → 같은 폴더의 기록이 파편화됨. "projectId 클라이언트 mint +
  매핑 파일" → 조회 로직·상태가 늘어 결정적 id보다 복잡.
- **재검토 조건**: 프로젝트가 여러 폴더/원격을 아우르거나 전문 검색·대량 기록이 필요해지면 SQLite +
  프로젝트 목록 화면으로 승격.

### D27. "새 세션"은 ChatPanel 리마운트로 초기화
- **결정**: 새 세션 버튼은 `WorkspaceView`의 remount 키(`sessionNonce`)를 올려 `ChatPanel`을 통째로
  리마운트한다(워크스페이스·작업 폴더는 유지). 이로써 `messages`/`agentId`/`sessionIdRef`/
  `resumeRef`/`seededRef`/`persistIdRef` 등 모든 state·ref가 한 번에 초기화되고, `started=false`가
  되어 **에이전트 select가 다시 활성화**된다. 기록 열기도 같은 경로로 `initialSession`을 주입해 remount.
- **근거**: 대화 상태가 전부 `ChatPanel` 지역 state라 키 remount가 가장 단순·확실한 리셋. 수동으로
  각 ref를 되돌리는 것보다 누락 위험이 없다.

### D28. codex TLS 오류(BadSignature)는 앱 밖 환경 이슈 — 안내+새 세션 복구만
- **결정**: codex의 `Reconnecting… (invalid peer certificate: BadSignature)`는 **codex CLI 자체**의
  rustls TLS 검증 실패(사내 TLS 검사 프록시의 재서명 인증서를 불신)로 결론. 앱은 해당 문자열을
  `RunEvent::Error`로 중계할 뿐이며(자체 재시도 없음), (a) `errorHint`로 한글 안내를 덧붙이고 (b)
  **새 세션**으로 회복하도록 한다. **환경변수/CA 번들 주입은 이번 범위에서 도입하지 않는다**(사용자 선택).
- **근거**: 근본 원인이 앱 로직이 아니라 OS/네트워크 신뢰 저장소라 앱 코드로 "고칠" 수 없다. 새 세션은
  codex의 `resume` 루프(깨진 thread로 계속 실패)에서 벗어나게 해 실질적 회복 경로가 된다.
- **재검토 조건**: 사내에서 codex가 존중하는 CA/proxy 주입 방식이 확인되면, 에이전트 실행 env에
  `SSL_CERT_FILE`/`HTTPS_PROXY` 등을 설정으로 주입하는 기능을 추가한다.

### D29. 홈 최근목록은 전역 프로젝트 단위 (열기 = 작업 폴더 전환 + 마지막 세션) — **D33로 개정됨**
> ⚠️ "열기 = 전역 workdir 전환(`setWorkdir`)" 부분은 **D33로 개정**되었다(열기는 그 프로젝트의 id +
> 저장된 workdir로 워크스페이스에 진입하며 전역 workdir을 바꾸지 않는다). 프로젝트 단위 최근목록·
> `ProjectSummary` 롤업은 유지된다.

- **결정**: 홈 "최근 작업"을 세션 단위(`listSessions(workdir)`)에서 **프로젝트 단위**(`list_projects`,
  모든 작업 폴더 전역)로 바꾼다. 각 항목은 프로젝트 제목 + 활동 롤업(`ProjectSummary`)을 보여주고,
  클릭하면 앱 작업 폴더를 그 프로젝트로 **전환**(`setWorkdir`)한 뒤 **가장 마지막 세션**을 연다.
  ChatPanel 헤더의 **기록** 팝오버는 그대로 *현재 프로젝트의 세션 목록*(프로젝트 내 세션 전환)으로 둔다.
- **근거**: 프로젝트=작업 폴더 모델(D26)에서 "여러 프로젝트를 오가며 재개"하려면 홈이 폴더 경계를 넘는
  진입점이어야 한다. 세션 단위 목록은 현재 폴더 1개로 스코프되어 폴더 전환 수단이 없었다. 이어쓰기·캔버스가
  올바른 폴더에서 동작하도록 열기 시 workdir 전환은 필수. `ProjectSummary`는 기존 `list_sessions_at`
  롤업으로 계산해 새 상태/의존성이 없다.
- **홈↔워크스페이스 역할 분담**: 홈=프로젝트 간 이동(폴더 전환), ChatPanel 기록=프로젝트 내 세션 이동.

### D30. 요구사항 명확화는 전용 도구 채널 없이 프롬프트 주입 + 텍스트 블록 파싱
- **결정**: `plan` 카테고리 **첫 턴**에 프롬프트 앞에 `CLARIFY_INSTRUCTION`을 붙여(사용자에겐 안 보임)
  에이전트가 ` ```clarify ` 펜스 JSON 블록으로 명확화 질문을 먼저 내도록 유도하고, 스트림 `end` 시 그
  블록을 **클라이언트에서 파싱**(`parseClarify`, `src/lib/clarify.ts`)한다. 파싱 성공 시 블록을 채팅에서
  제거(`CLARIFY_NOTE`로 대체)하고 질문을 캔버스 폼으로 올린다. **파싱 실패/취소 시 일반 chat으로 폴백**.
- **근거**: 로컬 CLI 에이전트 스트림에는 우리가 제어하는 커스텀 tool-call 채널이 없다. 모든 에이전트
  (claude/codex/gemini/aipro)가 낼 수 있는 유일한 이식 가능 프로토콜은 **텍스트 내 약속된 블록**이다.
  2단 파서(clarify 우선 → 임의 펜스 JSON)와 관대한 질문 검증 + 완전 폴백으로 형식 미준수에 견딘다.
- **대안 기각**: 백엔드 tool 채널/전용 커맨드 — 로컬 CLI 스트림에 표준 도구 프로토콜이 없어 이식 불가.
- **재검토 조건**: 특정 에이전트가 구조화 출력을 안정 지원하면 그 경로로 승격.

### D31. 명확화 질문은 캔버스 폼 + WorkspaceView로 상태 리프트, pending은 v1 transient
- **결정**: 질문은 우측 **캔버스 '요구사항' 탭**에 폼(단일/복수/자유입력)으로 렌더한다. 대화(생성)↔캔버스
  (렌더/답변) 매개 상태(`clarify`/`canvasTab`/`answerSubmission`/`streaming`)는 `WorkspaceView`가 소유
  한다. 폼 제출 → `formatClarifyAnswers`로 만든 답변을 `ChatPanel.send`가 다음 턴으로 전송(세션형 resume /
  세션리스 transcript). 주입은 **첫 plan 턴에만**(`clarifyArmedRef`), 답변 턴엔 재주입 없음. **대기(미답변)
  질문은 v1에서 transient**(영속화 안 함) — 저장 세션 재오픈 시 안내문만 보이고 재주입 없음.
- **근거**: 사용자 요구가 "캔버스에서 질문". 상태를 부모로 올리면 두 패널이 형제인 채로 왕복 가능. transient는
  스키마/백엔드 변경 0으로 최소 정확. (후속: `ChatMessage.clarify?` 임베드로 자동 영속·복원 가능 — 범위 밖.)

---

### D32. 프로젝트를 workdir에서 분리 — projectId는 클라이언트 mint (D26 대체)
- **결정**: 프로젝트 id를 workdir에서 **결정적으로 파생하지 않고**, 프론트가 `crypto.randomUUID()`로
  **mint**한다. 새 대화/카테고리 시작 = 새 프로젝트(새 id). 백엔드 커맨드는 모두 **projectId-keyed**로
  전환: `ensure_project(projectId, workdir, title, category)`, `save_session(projectId, session)`,
  `list_sessions(projectId)`, `load_session(projectId, sessionId)`. `ensure_project`가 workdir를
  **resolve**한다 — 외부 폴더가 주어지면 그대로, 없으면 프로젝트 전용 `projects/<id>/workspace/`
  하위폴더(생성 후 그 경로 반환). 프론트는 반환된 `workdir`을 실행 cwd + 캔버스 루트로 쓴다.
  `save_session`은 더 이상 manifest를 만들지 않는다(첫 저장 전 `ensure_project` 선행이 불변식).
  외부 폴더는 **홈에서 프로젝트별로 지정**한다(R1; 초기엔 전역 `settings.workdir`이었으나 D33 R1에서
  폐기·삭제). `Project`에 `category` 추가(`#[serde(default)]`로 구 manifest 하위호환).
- **근거**: 사용자 요구 — "어떤 질문이든 같은 프로젝트가 열리는" 문제(D26의 workdir=프로젝트 결정성)를
  해소하려면 프로젝트를 폴더에서 분리해야 한다. 폴더명=projectId라서 **기존 결정적-id 폴더도 그대로 열림**
  (마이그레이션 불필요). 기본 cwd를 프로젝트 자체 폴더로 두면 사용자가 폴더를 고르지 않아도 격리된
  작업 공간이 생긴다. **새 Cargo 의존성 0**(id=프론트 mint). `workspace/` 하위폴더는 `project.json`/
  `sessions/`를 캔버스 트리에서 분리한다(`files.rs::list_dir`의 스킵 목록이 이들을 못 거르므로).
- **대안 기각**: flat `projects/<id>/`(캔버스에 영속화 노이즈 노출); `settings.workdir` 재정의 없이
  프로젝트마다 폴더 강제 선택(사용자 마찰).
- **재검토 조건**: 한 프로젝트가 여러 폴더/원격을 아우르면 workdir 목록 모델로 승격.

### D33. 프로젝트 열기 = 워크스페이스 진입(그 프로젝트 id+workdir), 전역 workdir 미변경 (D29 개정)
> ⚠️ **R1 개정**: 전역 `settings.workdir`(상단바 picker)을 **폐기**했다(`set_workdir`/`Settings.workdir`
> 삭제). 폴더 선택은 **홈 프롬프트 아래의 작업 폴더 지정 버튼**으로 프로젝트별·transient(미지정=자동)로
> 이동했고, 상단바의 작업 폴더 표시도 제거했다. 아래 결정의 나머지(프로젝트별 workdir·지연 생성·빈
> 프로젝트 숨김)는 유지된다.

- **결정**: 홈 최근목록에서 프로젝트를 열 때 그 프로젝트의 **id + 저장된 workdir**를 워크스페이스로 넘겨
  진입한다(`HomeArea`가 `activeProject{id,workdir}` 소유, `WorkspaceView`가 `resolvedWorkdir` 소유해 D27
  remount에도 유지). 새 프로젝트는 홈 진입 시 새 id를 mint하고, 홈에서 폴더를 지정하면 그 폴더를, 아니면
  첫 send에서 `ensure_project`로 **지연 생성**(프로젝트 전용 `workspace/`)한다. 빈 프로젝트(세션 0)는 홈
  목록에서 숨긴다.
- **근거**: D32에서 workdir이 프로젝트별 값이 되었으므로 전역 workdir 전환/표시는 부적절. 지연 생성은
  D26의 "첫 질문 시 폴더 생성" 불변식을 유지해 워크스페이스 입·퇴장만으로 빈 폴더가 생기지 않게 한다.
  폴더 선택을 홈으로 옮겨 "새 대화 = 새 프로젝트(기본 자동 폴더, 선택 시 지정 폴더)" 모델을 명확히 했다.
- **ensure 규약(R1)**: 프레시 채팅은 첫 send에서 **항상 1회 `ensure_project` 호출**(지정 폴더가 미리
  알려져 있어도) — 매니페스트를 반드시 기록해 최근목록 누락을 막는다(`ChatPanel.ensuredRef`).
- **표시기**: 활성 프로젝트 폴더는 캔버스 툴바 폴더 칩이 표시한다(상단바는 폴더를 표시하지 않음).

### D34. 카테고리 워크플로우 = 클라이언트 단계 오케스트레이터 (D30/D31 일반화)
- **결정**: clarify 1회성 기계(D30/D31)를 **단계 오케스트레이터**(`src/lib/workflow.ts`)로 일반화한다.
  `Step{id,kind,instruction,progress?,file?}` + `WORKFLOWS: Record<Category, Step[]>`. `plan`은 대표
  플로우 `[clarify → search → document(docs/plan.md) → chat]`로 완전 구현, 나머지 카테고리는 턴1에만
  방향을 잡는 유도 서문(`chat` 단계 1개)으로 기반만 마련(단계 추가는 배열 확장). `ChatPanel`은 단계 커서
  (`stepIndexRef`/`stepArmedRef`/`inflightStepRef`)로 매 턴 지시문을 주입하고 `end`에서 `step.kind`로
  분기한다 — **상호작용 단계(clarify)는 확인 지점에서 멈추고**(사용자 폼 답변이 다음 단계를 발사),
  **생성형 단계(search/document)는 자동 진행**(nonce+effect로 다음 턴 자동 발사, stale 클로저 회피).
  document 단계는 에이전트가 **실제 파일**을 쓰게 하고 캔버스에서 연다(기존 `onOpenFile`+파일 뷰어 재사용).
  search 단계는 스트리밍되는 `toolUse`/`toolResult`(에이전트의 Grep/Read)를 그대로 활용. 단계 커서는
  **transient**(D31 확장; 로드 세션은 끝으로 시작해 일반 대화). **백엔드 도구 채널은 여전히 없음(D30 재확인)**.
- **근거**: 로컬 CLI 스트림에 표준 도구 프로토콜이 없어 클라이언트 오케스트레이션이 유일한 이식 경로.
  파일 뷰어·도구 이벤트를 재사용해 신규 백엔드 0, 신규 캔버스 렌더 최소(트리 새로고침 nonce 1개).
- **안전장치**: 단조 증가 커서 + 1회성 arm + `succeeded`에만 자동발사 + 종단 `chat` + 상한으로 무한
  진행 방지. 취소/실패는 자동진행 중단→일반 대화. 파싱 실패는 일반 대화 폴백(대화 안 깨짐).
- **한계**: 세션리스(gemini/aipro)는 transcript 재전송이라 지시문 lossy·크기 증가; plain(opencode/
  antigravity)은 도구 스트림/파일쓰기 보장이 약해 degrade. plan은 claude/codex/gemini에서 가장 풍부.
- **재검토 조건**: 카테고리별 고유 단계가 늘면 `Step.kind`/캔버스 렌더를 확장(예: 소스 목록 전용 탭).

### D35. 요구사항 폼은 accent 카드 그리드 (options는 string[] 유지)
- **결정**: `RequirementsForm`의 single/multi를 세로 버튼 행에서 **반응형 카드 그리드**(2열, hover lift,
  선택 시 accent 테두리+틴트+체크 배지)로 재디자인한다. `text`/`required`/스트리밍 비활성/제출 흐름은
  유지. `ClarifyQuestion.options`는 **`string[]` 그대로**(옵션별 설명 미지원) 두어 프로토콜 무변경.
- **근거**: 사용자 요구 — 콤보박스형보다 카드 버튼. 앱 전역 선택색(accent) 관례를 따르고 미사용 blue
  토큰은 도입하지 않아 토큰 변경 0. 옵션 설명 지원은 clarify 프로토콜(스키마·파서·포맷)을 건드려 별도
  증분으로 연기.
- **재검토 조건**: 카드에 부제/설명이 필요해지면 `options`를 `(string | {value,label,description})[]`로
  확장하고 `coerceQuestions`·`CLARIFY_INSTRUCTION`·`formatClarifyAnswers`를 함께 갱신.

---

### D36. 카테고리 시작 = 고정 선택지 우선 + 프롬프트 자동채움 (D30/D34 확장)
- **결정**: 카테고리 진입 시 **프롬프트가 아니라 고정 선택지(옵션) 폼을 먼저** 보여준다. 옵션은
  카테고리별 정적 카탈로그(`src/lib/options.ts`의 `CATEGORY_OPTIONS: Record<Category, ClarifyQuestion[]>`)로
  앱에 정의하고, 진입 즉시(에이전트 대기 없이) 캔버스 '요구사항' 폼으로 렌더한다. 홈 프롬프트로 시작했으면
  숨김 **프리필 턴**을 격리 실행해(세션 id/resume 미사용, 영속화·스텝커서 불변) `prefillInstruction`으로
  에이전트가 요청에서 **확신 가능한 항목만** ` ```prefill ` JSON으로 채우게 하고, `parsePrefill`로 검증해
  폼을 미리 채운다(미확인 항목만 사용자에게). 폼 제출이 **첫 작업 턴**을 발사(스킬+워크플로우 1단계+답변+
  원요청 주입). plan의 기존 `clarify`(에이전트 생성 질문) 단계는 이 옵션 프리플로우로 대체 →
  `WORKFLOWS.plan = [search, document, chat]`.
- **근거**: 사용자 요구 — "선택지부터 결정하고 시작, 프롬프트에서 아는 값은 자동 채움". 고정 카탈로그는
  즉시성·결정성이 높고(open-design `od.inputs` 대응), 기존 `ClarifyQuestion`/`RequirementsForm`/캔버스 탭을
  그대로 재사용해 신규 렌더 0. 프리필은 clarify와 대칭인 fenced-block 파서라 **새 백엔드/IPC 0**.
- **대안 기각**: 에이전트가 매번 질문 생성(동적 clarify) — 첫 폼까지 왕복 발생, 결정성 낮음. 클라이언트
  휴리스틱 자동채움 — 한국어 자유문 매핑이 불안정.
- **재검토 조건**: 옵션에 부제/설명·조건부 표시가 필요하면 `ClarifyQuestion` 스키마와 프리필 프로토콜을 확장.

### D37. 시스템 스킬 = 앱 번들 + 카테고리별 매핑 (open-design SKILL.md 최소 이식)
- **결정**: 이 시스템이 제공하는 "스킬"(CLI 자체 스킬과 무관)을 **앱에 번들**(`src/lib/skills.ts`의
  `SKILLS: Record<string, Skill{id,name,body}>`)하고 **카테고리별로 매핑**(`CATEGORY_SKILL` + `skillFor`)한다.
  `body`(마크다운 지시문)는 `ChatPanel.send()`가 **첫 작업 턴에 1회** 프롬프트 앞에 주입한다(스텝 지시문 위).
  open-design은 `composeSystemPrompt`가 `SKILL.md` 본문을 `## Active skill`로 시스템 프롬프트에 주입하지만,
  이 앱은 시스템 프롬프트가 없으므로 **wire 프롬프트 주입**으로 최소 이식한다.
- **근거**: 사용자 선택(앱 번들·카테고리 매핑). 현재 요구는 카테고리별 페르소나/방법 고정이라 정적 상수가
  가장 단순하고 배포·검증이 쉽다. 기존 지시문 주입 패턴과 동일해 신규 인프라 0. 디스크 로더(런타임 SKILL.md)는
  재빌드 없는 추가가 필요해질 때로 연기.
- **한계**: 세션형(claude/codex)은 스킬이 세션에 유지되나 세션리스(gemini/aipro)는 첫 턴에만 보여 lossy.
- **재검토 조건**: 재빌드 없이 스킬을 추가/수정해야 하면 open-design `skills.ts`식 디스크 로더
  (`~/.operation-wizard/skills/*/SKILL.md`)로 승격하고 `SKILLS`를 동적 로딩으로 전환.

### D38. Settings 뷰 폐지 → Agents 카드에 경로 설정 통합
- **결정**: NavRail의 별도 **Settings 뷰를 제거**(`View = "home" | "agents"`)하고, 커스텀 바이너리 경로
  설정(입력/Browse/Save & detect/Clear/`*_BIN` 안내)을 **Agents 카드 하단의 접이식 섹션**으로 통합한다.
  `AgentCard`가 `settings`+`onSave`를 받아 표시(탐지)와 편집(경로)을 한 카드에서 처리한다. 백엔드
  `set_agent_bin`/`get_settings`와 `App.handleSave`/`detectOne`는 무변경. `SettingsView.tsx` 삭제,
  `DIAGNOSTIC_HINT`의 "Set a custom path in Settings" 문구를 카드 기준으로 수정.
- **근거**: 사용자 지적 — 채팅 패널의 에이전트 선택과 Settings의 에이전트 목록이 중복으로 보인다. 실제
  진짜 중복은 Agents(탐지 표시)와 Settings(경로 편집)이며, 둘 다 6개 에이전트를 카드/행으로 나열한다. 한
  카드에서 상태 확인 + 경로 override를 하면 화면 중복이 사라지고 진단 힌트("경로를 설정하세요")도 같은 곳을
  가리킨다. 채팅 셀렉터(실행 선택)는 `agents`+`detected`만 소비하므로 무영향.
- **재검토 조건**: 에이전트별 설정 항목이 늘면 카드 내 설정 섹션을 별도 패널/모달로 승격.

---

### D39. 워크플로우 단계·스킬 사용자 설정화 = settings.json 확장 + 필드별 커맨드 (D37 개정)
- **결정**: 카테고리별 워크플로우 단계와 스킬 레지스트리를 **사용자가 설정 화면(Flows 뷰)에서 등록**할 수
  있게 하고, `settings.json`에 영속화한다. `Settings`에 `skills: Option<Vec<SkillDef>>`(전체 교체형 레지스트리)와
  `workflows: HashMap<category, Vec<StepDef>>`(카테고리별 override)를 추가하고, 커맨드 `set_skills`/`set_workflow`를
  `set_agent_bin` 패턴(검증→load→mutate→save→전체 `Settings` 반환)으로 추가한다. `None`/`null` 전달 = 해당
  override 삭제(기본값 복원). **코드 내 기본값(`DEFAULT_SKILLS`/`DEFAULT_WORKFLOWS`) = 폴백 = 편집 가능한 샘플
  콘텐츠**로, settings에 값이 없으면 항상 기본값이 적용되고 Flows 뷰는 기본값을 초기 편집값으로 보여준다.
- **세부**: `StepDef.kind`는 Rust에서 **enum이 아니라 String**이다 — `settings::load`가 파싱 실패 시
  `unwrap_or_default()`로 전체 파일을 버리므로, 알 수 없는 kind 1건이 설정 전체를 지우는 사고를 막는다.
  저장 시 `validate_steps`(≥1개·id 유일·kind 유효·**마지막 단계는 `chat`**)/`validate_skills`(id 유일·이름 필수)로
  검증하고, 런타임은 `coerceSteps`(프론트)로 한 번 더 방어(잘못된 항목 드롭 + 종단 chat 자동 보강). 카테고리
  id 상수 `settings::CATEGORIES`는 프론트 `workspace.ts`의 `Category` 유니온과 동기화한다. dangling
  `skillIds`(참조된 스킬이 삭제됨)는 저장 시 허용하고 런타임이 무시한다(편집기는 경고 표시).
- **근거**: 사용자 요구 — "설정 화면에서 단계·스킬 등록". D37의 재검토 조건(재빌드 없는 스킬 추가/수정)은
  디스크 SKILL.md 로더가 아니라 **설정 화면**으로 해소한다 — 편집 표면이 하나면 충분하고, settings.json은
  기존 로드/저장·마이그레이션 인프라를 그대로 쓴다(새 Cargo 의존성 0).
- **대안 기각**: `~/.operation-wizard/skills/*/SKILL.md` 디스크 로더 — 설정 화면과 파일 편집 두 표면이 중복.
  별도 flows.json — 설정 루트가 둘로 갈라짐(현 데이터 규모에서 이득 없음).
- **재검토 조건**: 스킬/단계가 수십 개로 늘거나 팀 공유가 필요하면 별도 파일·내보내기/가져오기로 승격.

### D40. 스킬 주입: 카테고리당 1회 → 단계별 skillIds (D34/D37 갱신)
- **결정**: `CATEGORY_SKILL`/`skillFor`(카테고리당 스킬 1개, 첫 작업 턴 1회 주입)를 **폐지**하고, 스킬을
  `StepDef.skillIds`로 **단계에 귀속**시킨다. 단계가 armed된 턴에 그 단계의 스킬 body들을 지시문 위에 주입한다.
  guide/query/change는 단일 `chat` 단계에 기존 카테고리 스킬을 붙여 동작 동등성을 유지한다. **세션형**(claude/
  codex)은 같은 스킬을 대화당 1회만 주입(`injectedSkillIdsRef` dedupe; 전송 실패 시 되감기), **세션리스**(gemini/
  aipro)는 transcript 재전송으로 자연 재노출되므로 dedupe하지 않는다. `Step.progress` 하드코딩 문자열은
  삭제하고 `StepDef.name`에서 진행 노트를 **파생**한다(`progressLabel` = "N/M단계 · <이름> 중…").
- **근거**: 사용자 요구 — "각 단계에 스킬들을 등록해 해당 단계에서 주입". 단계 수가 사용자 정의로 가변이
  되면서 고정 progress 문자열·카테고리 단일 스킬 모델이 모두 파탄 — 단계 귀속 + 파생이 유일하게 일관적.
- **샘플(기본값)**: plan 카테고리를 6단계로 개편 — 소스코드 분석(document, `docs/source-analysis.md`,
  mermaid 다이어그램 스킬) → 컨플루언스 탐색(search, 접근 수단 없으면 로컬 docs/ 폴백 스킬) → 계획 생성
  (document, `docs/plan.md`) → 변경영향분석서 생성(document, `docs/impact-analysis.md`) → 테스트 계획서 생성
  (document, `docs/test-plan.md`) → 마무리 대화(chat). 스킬 8개(신규: source-analysis/confluence-search/
  impact-analysis/test-plan).
- **한계**: 세션리스는 transcript 크기 증가. 5개 생성형 단계 연쇄는 자동 진행되므로 중간 개입은 정지(취소)
  → 일반 대화 폴백 경로를 쓴다(D34 안전장치 그대로).

### D41. 요구사항 폼 pending 중 채팅 차단 + '요구사항' 탭 조건부 렌더
- **결정**: 카테고리 옵션 폼이 사용자 답변을 기다리는 동안(`clarify` 비어있지 않음) **채팅 컴포저를 비활성**
  (textarea/전송 버튼 disabled + 안내 placeholder)하고 `ChatPanel.send()`도 가드한다. 단, **숨김 프리필 턴과
  자동전진 턴(system)은 예외**로 계속 실행되고, **정지(Stop) 버튼은 차단하지 않는다**(프리필 스트리밍 취소
  가능해야 함). 캔버스의 '요구사항' 탭 pill은 **폼이 대기 중일 때만 렌더**하고, 제출/초기화로 `clarify`가
  비워지면 탭 자체가 사라진다(`effectiveTab` 파생으로 pill 없는 requirements 상태가 렌더될 수 없게 방어).
  빈 폼 placeholder 화면은 삭제.
- **근거**: 사용자 요구 — "선택지 입력 중에는 채팅이 막혀야 하고, 완료되면 탭이 없어져야 한다". 폼 제출
  시 `setAnswerSubmission`+`setClarify(null)`이 같은 커밋에 배치되어 첫 작업 턴은 차단에 걸리지 않는다.
  로드 세션은 부트를 건너뛰어(`onClarify` 미호출) 게이팅이 없다.

### D42. 캔버스 마크다운+mermaid 미리보기 (react-markdown + remark-gfm + mermaid)
- **결정**: `FileViewer`에 `.md` **미리보기/소스 토글**을 추가한다(기본 미리보기). 렌더는
  `react-markdown`+`remark-gfm`(표), ` ```mermaid ` 펜스는 `mermaid`로 실제 다이어그램 렌더
  (`src/components/Markdown.tsx`). mermaid는 **dynamic import**로 코드 스플릿(~1.5MB, 첫 다이어그램에서 로드),
  `securityLevel: "strict"` 초기화(스크립트/클릭 비활성, 자체 DOMPurify 소독). 렌더 실패 시 원본 코드블록 +
  실패 노트로 폴백(뷰어 안 깨짐). 타이포는 토큰 기반 `.markdown-body`(global.css)로 직접 정의
  (`@tailwindcss/typography`는 토큰 팔레트와 충돌해 미사용). `FileViewer`에 `refreshNonce`를 전달해 이미 열린
  파일을 후속 단계가 재작성하면 재로드한다.
- **근거**: plan 워크플로우 산출물(분석서·계획서)이 표/다이어그램을 담으므로 텍스트 뷰로는 실효성이 낮다.
  전부 npm 번들(CDN 없음)이라 사내망/오프라인에서 동작하고 CSP는 `null` 유지.
- **한계**: mermaid 지연 청크가 커서 Vite 청크 경고가 남는다(수용). 다크 테마에서 mermaid `neutral` 테마가
  완전히 토큰을 따르진 않는다(후속 조정 가능).

---

### D43. Release CI = GitHub Actions 수동 워크플로우, 설치파일 없이 단독 exe만 배포
- **결정**: `.github/workflows/release.yml`("Release")을 추가한다. `workflow_dispatch` 입력
  `version`(`major`/`minor`/`patch` 택1)으로 수동 트리거하며, `windows-latest`에서
  `npm run tauri build -- --no-bundle`로 **NSIS/MSI 설치파일 번들링을 건너뛰고** 컴파일된
  바이너리(`src-tauri/target/release/operation-wizard.exe`)를 그대로 릴리즈 자산으로 쓴다. 버전은
  최신 `v*.*.*` git 태그(없으면 `package.json`)를 기준으로 `.github/scripts/bump-version.mjs`가
  계산해 `package.json`/`src-tauri/tauri.conf.json`/`src-tauri/Cargo.toml`에 반영(저장소에 커밋하지
  않음 — 태그가 버전의 단일 진실 소스)하고, `softprops/action-gh-release`가 새 태그의 GitHub Release를
  만들어 exe를 첨부한다. 릴리즈 노트 본문은 `generate_release_notes: true`(GitHub 자동 변경사항 요약)로 채운다.
- **근거**: 요구사항이 "단독 실행파일(exe, 설치파일 아님) 배포"라 `tauri build`의 기본 번들링(설치파일
  생성)이 오히려 방해가 된다 — `--no-bundle`로 컴파일 산출물만 취하는 편이 정확하고 단순하다. 버전을
  파일에 커밋하지 않고 태그만으로 추적하면 매 릴리즈마다 별도 커밋이 쌓이지 않는다. GitHub Actions의
  `windows-latest` 이미지는 VS Build Tools가 사전 설치돼 있어 [06](06-build-and-environment.md)의 로컬
  vcvars 제약이 CI에는 적용되지 않는다(새 Cargo/npm 의존성 0, 순수 CI 워크플로우).
- **대안 기각**: `tauri-apps/tauri-action`(공식 액션) — MSI/NSIS 설치파일 생성에 최적화돼 있어 "단독 exe"
  요구와 어긋나고, 이번 규모에서는 직접 `--no-bundle` 빌드가 더 명확함. 버전을 저장소에 커밋 — 매 릴리즈마다
  불필요한 커밋/머지 충돌 위험.
- **재검토 조건**: 설치파일(MSI/NSIS) 배포도 함께 필요해지면 `--no-bundle`을 제거하고 `tauri-apps/tauri-action`
  또는 번들 산출물 업로드로 전환.

---

### D44. 기반 3단계 = 고정(pinned) StepKind로 워크플로우 앞에 프리펜드
- **결정**: 카테고리 워크플로우 앞에 **반드시 거치는 기반 3단계**를 추가한다 — ① 코드베이스 분석 →
  ② 사내 문서 RAG 검색 → ③ 지식 베이스 반영. 별도 오케스트레이션 기계를 만들지 않고 **새 StepKind**
  (`codebase`/`rag`/`knowledge`, `FOUNDATION_KINDS`)로 모델링해 기존 스텝 커서·자동전진·취소 폴백·Flows
  편집 UI를 전부 재사용한다. `plan`은 항상 강제, guide/query/change는 Flows의 "기반 3단계 사용" 토글로
  opt-in하며 **저장된 워크플로우 배열에 기반 kind 존재 여부가 곧 플래그**다(새 설정 필드 없음).
  `coerceSteps(steps, {foundation})`가 순서(`codebase→rag→knowledge`)를 강제하고 누락 단계를 기본값
  (`DEFAULT_FOUNDATION_STEPS`)으로 보충한다 — **기존에 저장된 plan 워크플로우도 로드 시 자동으로 기반
  단계를 얻는다**(하위호환 겸 필수화). 편집기는 기반 카드를 pinned로 렌더(삭제·이동·kind 변경 불가;
  지시문·스킬·산출물 파일은 편집 가능). `end` 핸들러의 생성형 판정은 `isGenerative(kind)`(chat 외 전부)로
  일반화. rag/knowledge 단계는 에이전트 턴 **전에 클라이언트 preflight**(`stepPreflight`)가 컨텍스트를
  조회해 wire에 첨부하고, 미설정/0건/실패면 **에이전트 턴 없이 건너뛴다**(system 안내 + 커서 전진 + 다음
  생성형 단계로 체인; 미답 프롬프트는 다음 단계로 이월). preflight 중 Stop은 취소 폴백(커서 끝, 일반 대화).
- **근거**: 사용자 요구 — "큰 세 단계를 반드시 지나고 그 이후 기존 단계 진행". 단계 모델 재사용이
  스킬 연결(1단계에 사용자 탐색 스킬), 진행 표시, 안전장치를 공짜로 제공한다.
- **대안 기각**: ChatPanel에 하드코딩 프리스테이지 기계 — arming/inflight/rearm/dedupe를 병렬 복제하게 됨.

### D45. 코드베이스 경로 = workdir와 분리, folder 질문 타입 + extraDirs(--add-dir)
> ⚠️ extraDirs의 에이전트별 매핑은 **D52로 확장**되었다(gemini/aipro도 `--include-directories`로 접근
> 부여; codebase preflight의 경로 한 줄은 생성형 단계 공통 `pathContext` 주입으로 일반화). 아래의
> "나머지는 프롬프트 언급으로 degrade"는 이제 plain 에이전트(opencode/antigravity)에만 해당한다.

- **결정**: 분석 대상 **코드베이스 폴더는 작업 폴더(workdir)와 별개**다(workdir = 산출물/cwd, 코드베이스 =
  읽기 대상). 첫 질문에서 반드시 선택하도록 `ClarifyQuestion`에 **새 타입 `"folder"`**를 추가하고
  (`RequirementsForm`의 `FolderAnswer` = 네이티브 폴더 픽커 + 경로 칩), `optionsFor(category, settings)`가
  기반 단계 활성 카테고리의 폼 맨 앞에 필수 `codebasePath` 질문을 프리펜드한다. **프리필은 folder 질문을
  제외**한다(에이전트는 로컬 경로를 알 수 없음). 폼 제출 시 folder 답변은 wire에서 분리되어 **구조적으로
  전달**된다: `WorkspaceView.codebasePath` 상태 → `ChatPanel` prop → ① codebase 단계 preflight가
  "분석 대상 코드베이스 폴더: <path>"를 주입, ② **매 턴** `RunArgs.extraDirs`로 전달(claude는 `--add-dir`
  반복 인자, codex는 full-access 샌드박스라 불필요, 나머지는 프롬프트 언급으로 degrade), ③ 프로젝트
  매니페스트에 영속(`Project.codebasePath`, `ensure_project` 인자 + `set_project_codebase` 커맨드 —
  폴더 선택이 ensure 이후에 일어나므로 갱신 커맨드가 정상 경로). 스킬 리소스 폴더(`SkillDef.dir`)도 같은
  extraDirs 경로를 탄다: 주입 턴에 본문 뒤 "[스킬 리소스 폴더] …" 라인을 붙이고 이후 턴에도 접근 유지.
  캔버스는 **루트 전환 칩**(작업 폴더 ↔ 코드베이스)으로 코드베이스를 브라우징한다(`list_dir`/`read_file`은
  절대경로라 백엔드 무변경). 재오픈 시 `ProjectSummary.codebasePath`로 복원.
- **근거**: 사용자 확정 — 분석 대상과 산출물 위치의 분리. 요구사항 폼은 이미 필수·채팅차단(D41)·프리필
  게이트라 폴더 강제 선택 지점으로 재사용이 최적.

### D46. RAG 검색 결과 캔버스 탭 = 인메모리 HTML(srcdoc), 파일 아님
- **결정**: rag 단계의 검색 결과는 `CanvasTab`에 **새 탭 `"rag"`("검색 결과")**로 표시한다. 클라이언트가
  `RagHit[]`에서 **이스케이프된 자립형 HTML 문자열**을 결정적으로 생성(`lib/foundation.ts::ragResultHtml`)해
  FileViewer의 HTML 미리보기와 동일하게 **sandbox iframe(`allow-scripts`, same-origin 없음) srcdoc**으로
  렌더한다. 요구사항 탭과 달리 결과가 생긴 뒤 세션 동안 pill이 유지된다(새 세션/기록 열기 시 초기화).
- **근거**: 프론트에 파일쓰기 능력이 없고(D21) 일시적 결과물로 workdir을 오염시키지 않기 위함. 에이전트
  생성 HTML이 아니라 클라이언트 생성이라 이스케이프가 보장된다.

### D47. 단계별 결과형식 output(chat/file/html) + html은 렌더 서브스텝으로 런타임 확장
- **결정**: `StepDef.output?: "chat"|"file"|"html"`(`STEP_OUTPUTS`, 백엔드 save 검증·plain String)을
  추가한다. 미지정 시 kind에서 파생(document→file, 그 외→chat; `stepOutput`). Flows 편집기는
  search/document/codebase kind에 "결과 형태" 셀렉트를 노출한다. `"chat"`은 런타임에 file을 제거(산출물
  없음), `"html"`은 `runtimeWorkflowFor` = `expandOutputSteps(workflowFor(...))`가 원 단계 뒤에 **합성
  렌더 서브스텝**(`{id: <id>-html, kind:"document", file: <file>.html, skillIds:["html-render"]}`)을
  삽입한다 — 내장 `html-render` 스킬이 md 산출물을 외부 리소스 없는 자립형 HTML로 재생성하고, 기존
  자동전진·`onOpenFile`·FileViewer sandbox 미리보기 경로를 그대로 탄다. 설정에는 `output`만 저장되고
  편집기는 합성 단계를 보지 않는다(`workflowFor`는 편집용 미확장, 오케스트레이터만 확장본 사용).
- **근거**: 사용자 요구 — "각 단계 결과를 대화만/파일/HTML 캔버스 중 선택; HTML은 예쁘게 재생성하는
  스킬로". 한 턴에 md+html을 모두 시키면 지시 과부하 — 별도 턴이 품질·단순성 모두 우수.

### D48. 지식 뷰 + reqwest(blocking/native-tls) + Confluence 수집 Channel + RAG 어댑터 스텁
> ⚠️ **부분 대체(D82)**: Confluence **REST 크롤**과 `ingest_page`→RAG sink는 **D82로 대체**되었다
> (공식 MCP 서버로 수집 → 로컬 지식 베이스 artifact). 지식 뷰·`reqwest`(blocking/native-tls) 도입·
> `knowledge.rs`·`IngestEvent`/`IngestRegistry`/Channel 진행·`RagConfig`는 유지된다. 아래는 최초 결정.
- **결정**: NavRail에 4번째 뷰 **"지식"(`knowledge`)**을 추가한다 — ① RAG 검색 설정(endpoint/apiKey/topK +
  연결 테스트), ② Confluence 수집(baseUrl/PAT/루트 페이지 ID 또는 스페이스 키/TLS 예외 + 수집 시작·중지·
  진행 표시), ③ 지식 베이스 CRUD(제목+본문; `%USERPROFILE%\.operation-wizard\knowledge\<id>.json`
  항목당 1파일, `knowledge.rs`). 지식은 **단순 저장 + 프롬프트 주입**(RAG 아님)으로, knowledge 단계
  preflight가 `list_knowledge` 전체를 16KB 상한으로 주입한다(수십~수백 건에 충분; RAG 승격은 재검토 조건).
  **첫 HTTP 의존성 reqwest**를 `default-features=false, features=["blocking","json","native-tls"]`로
  추가한다 — blocking은 앱의 std 워커스레드 관용구와 일치하고(IPC 스레드 호출 금지), native-tls는 Windows
  에서 schannel = OS 인증서 저장소를 신뢰해 사내 TLS 재서명 프록시(D28의 codex BadSignature 부류)를
  회피한다. `ConfluenceConfig.allowInvalidCerts`(기본 off)는 최후 수단 opt-in. **Confluence 수집**
  (`confluence.rs`)은 Server/DC REST(Bearer PAT; Cloud Basic은 v1 범위 밖)로 **반복 BFS**(visited dedupe,
  MAX_PAGES=2000/MAX_DEPTH=10, 호출 사이 취소 플래그) 크롤링하고, 페이지 **원문 그대로**를
  `RagClient::ingest_page`로 전달한다(요약·임베딩은 사용자 RAG 서비스 담당). 진행은 `IngestEvent`
  Channel(run.rs 패턴; `IngestRegistry`로 취소), per-page 실패는 계속 진행. **RAG 어댑터**(`rag.rs`)는
  사용자가 채우는 확장 지점 — `RagClient::{ingest_page, search}`가 예시 스켈레톤 주석과 함께 **TODO(user)
  스텁**으로 제공되며, 미구현 시 한글 안내 Err를 반환해 rag 단계가 "건너뜀"으로 degrade한다(크래시 없음).
  검색은 `rag_search(query, topK?)` 커맨드(spawn_blocking). 크롤/검색 파이프는 trait(`ConfluenceApi`) +
  순수 파서로 네트워크 없이 단위 테스트한다. **비밀값 주의**: PAT/apiKey는 settings.json 평문 저장
  (로컬 단일사용자 v1 수용; Windows Credential Manager 승격은 후속).
- **부수 버그픽스**: `RunEvent`가 `rename_all_fields` 누락으로 struct variant 필드(`toolUseId`/`sessionId`/
  `inputTokens` 등)를 snake_case로 직렬화해 프론트 미러에서 조용히 유실되던 문제(codex 세션 캡처·도구 결과
  에러 플래그·토큰 사용량)를 `rename_all_fields = "camelCase"`로 수정(직렬화 테스트 추가; `IngestEvent`도
  동일 적용).
- **재검토 조건**: 지식이 수천 건으로 늘면 지식도 RAG 임베딩으로 승격(어댑터 재사용); Confluence Cloud
  지원이 필요하면 `ConfluenceConfig`에 authMode(Basic) 추가.

---

### D49. 캔버스 파일 뷰어 탭화 + 채팅/캔버스 드래그 리사이즈
- **결정**: 캔버스 '파일' 탭에서 트리(240px)+FileViewer 나란히 배치를 폐지하고, **'파일' 탭은 목록(트리)
  전용**, 파일을 열면(트리 클릭 또는 워크플로우 document 단계의 `onOpenFile`) **파일별 뷰어 탭이 새로
  열린다**. `CanvasTab`을 `"files" | "requirements" | "rag" | \`file:${path}\``로 확장하고
  `WorkspaceView`가 `openFiles: string[]`(탭 순서=삽입 순서, dedupe)를 소유한다. 파일 탭 pill은 닫기(×)
  버튼을 가지며, 활성 탭을 닫으면 '파일' 탭으로 폴백. `openFiles`에 없는 `file:` 탭은 `effectiveTab`이
  '파일'로 폴백(요구사항/rag 탭과 동일 방어 패턴). 새 세션/기록 열기 시 `openFiles` 초기화.
  또한 **ChatPanel(412px 고정)과 CanvasPanel 사이 경계를 드래그해 폭 조절**할 수 있다 — `chatWidth`
  상태(+`localStorage["ow.chatWidth"]` 기억, 순수 UI 값이라 settings.json 무변경), 리사이저 핸들은
  pointer capture로 드래그하고(clamp 320px~720px·캔버스 최소 360px 보장) 드래그 중 전역 오버레이로
  캔버스 iframe(HTML/rag 미리보기)의 이벤트 삼킴을 차단한다. ChatPanel은 `w-full`로 바꿔 래퍼가 폭을
  소유(ChatPanel API 무변경).
- **근거**: 사용자 요구 — "파일 목록과 문서 내용을 동시에 볼 필요 없이 탭으로 분리, 패널 크기 조정".
  FileViewer는 이미 자립형 컬럼이라 무변경으로 탭 본문에 재사용. 신규 백엔드/IPC 0.
- **재검토 조건**: 열린 탭이 많아져 관리가 필요하면(고정/전체 닫기 등) 탭 오버플로 메뉴를 도입.

### D50. RagConfig = endpoint + secretKey + passKey (apiKey 대체)
> ⚠️ **D65에서 실연동**: 두 키의 실제 전송처가 확정됐다 — `secret_key`→`x-fabrix-client`,
> `pass_key`→`x-openapi-token`(Fabrix rag-chat API). `search`는 더 이상 스텁이 아니며, `RagConfig`에
> `knowledge_asset_id`가 추가되고 지식 뷰 폼도 6필드(+ 라벨을 실제 헤더명으로)로 확장됐다. 아래는 최초 결정.
- **결정**: RAG 연결 설정을 **endpoint URL + secret key + pass key** 3값으로 바꾼다(`RagConfig.api_key`
  삭제 → `secret_key`/`pass_key` 추가, `top_k` 유지). 두 키는 사용자 RAG 서비스 호출 시 **요청 헤더**로
  전달될 값이며, 실제 전송은 여전히 `rag.rs`의 TODO(user) 스텁이 담당한다(스켈레톤 주석을
  `.header("X-Secret-Key", …)`/`.header("X-Pass-Key", …)` 예시로 갱신 — 헤더 이름은 사용자 서비스 계약).
  `set_rag_config`가 두 키를 trim/empty-filter 정규화. 구 settings.json의 `apiKey`는 serde가 unknown
  field로 무시(마이그레이션 불필요, 하위호환 테스트 추가). 지식 뷰 폼은 Endpoint/Secret Key/Pass Key
  (password)/Top K 4필드.
- **근거**: 사용자 확정 — RAG 연결에 필요한 헤더 3종. 어차피 스텁이라 필드 교체 비용이 최소인 지금이 적기.
  평문 저장 주의는 D48과 동일(읽기 전용 키 권장).

### D51. Confluence 수집 관찰성 = 프론트 모듈 싱글턴 스토어 (백엔드 무변경)
- **결정**: 수집을 시작한 뒤 **다른 화면으로 이동해도 진행현황이 유지·재표시**되게 한다. 백엔드 크롤
  워커는 이미 detached 스레드라 뷰 전환과 무관하게 돌지만, 수신 `Channel`과 진행 집계가
  `ConfluenceSection` 지역 state에 있어 언마운트 시 유실되던 것이 문제였다. 해결은 **React 트리 밖 모듈
  싱글턴 스토어**(`src/lib/ingest.ts`): `startIngest`/`stopIngest`가 Channel과
  `{status, ingestId, progress, failures, summary}`를 소유하고, 컴포넌트는 `useIngestState`
  (`useSyncExternalStore`)로 구독만 한다(이중 시작 가드 포함). 지식 뷰의 수집 러너가 이 스토어를 쓰고,
  **NavRail '지식' 아이콘에 수집 중 accent pulse 점**을 표시해 돌아갈 곳을 알린다. 수집 중 Confluence
  설정 저장은 비활성(실행 중 크롤은 시작 시점 설정을 사용). 데몬 수명 = 앱 프로세스 수명(재시작 복원 없음).
- **근거**: 사용자 요구 — "데몬으로 실행해 다른 작업을 이어가고, 진행현황을 보러 돌아올 수 있게".
  워커가 이미 백그라운드라 **백엔드 변경 0**이 가장 작은 해법. App state로 리프트하면 페이지당 이벤트마다
  App 전체가 리렌더되므로 구독형 스토어가 적합.
- **대안 기각**: 백엔드 `IngestRegistry`에 진행 스냅샷 보관 + 상태 조회 커맨드 — 앱 내 웹뷰는 리로드되지
  않아 프론트 스토어로 충분하며 IPC 표면 증가만 남음.
- **재검토 조건**: 수집 이력(지난 실행 로그)이나 앱 재시작 후 복원이 필요하면 백엔드 영속 상태로 승격.

---

### D52. 코드베이스 접근 = gemini/aipro `--include-directories` + 절대경로 컨텍스트 주입 (cwd 불변)
- **결정**: "코드베이스 폴더를 선택해도 분석이 작업 폴더에서 시작되는" 문제를 두 갈래로 해소한다.
  ① **접근 부여**: `gemini_build_args`/`aipro_build_args`가 `ctx.extra_dirs`를 **`--include-directories
  <dir>`**(디렉터리당 1쌍 — 콤마 결합형은 콤마 포함 경로를 오분할)로 매핑한다. gemini CLI의 공식 멀티
  워크스페이스 플래그로, claude `--add-dir`의 대칭이다. 기존에 gemini/aipro는 extraDirs를 무시했고
  워크스페이스 신뢰가 cwd로 한정되어 **코드베이스를 아예 읽지 못해** 작업 폴더를 분석했다(불만의 직접 원인).
  ② **탐색 시작점 강제**: `ChatPanel`이 **모든 생성형 단계의 armed 턴**에 절대경로 컨텍스트(`pathContext`)를
  주입한다 — "작업 폴더(산출물 저장, 절대경로): …" + "분석 대상 코드베이스 폴더(절대경로): …" + 탐색 지시
  (codebase 단계: "모든 소스 탐색을 이 폴더에서 시작, 작업 폴더에서 소스 찾기 금지"). 이를 위해 `send()`에서
  **`ensure_project`(workdir 확정)를 preflight·wire 조립 앞으로 이동**했다(첫 턴에도 절대경로를 알 수 있게).
  `CODEBASE_STEP`/`SOURCE_ANALYSIS_STEP` 지시문과 `codebase-explore`/`source-analysis` 스킬도 같은 취지로
  개정(산출물은 작업 폴더 절대경로 기준). 기존 `stepPreflight`의 codebase 한 줄은 `pathContext`로 흡수.
- **cwd는 바꾸지 않는다(대안 기각)**: codebase 단계 턴에 `RunArgs.cwd = codebasePath`로 스왑하는 안은
  기각 — claude 세션은 cwd 기준으로 저장되어 턴별 cwd 변경이 `--resume`을 깨뜨릴 수 있고, codex 스레드는
  첫 턴 cwd에 고정되며, gemini/aipro는 cwd 밖 쓰기가 막혀 산출물(`docs/*.md`)을 작업 폴더에 저장할 수 없다.
- **리스크**: 사내 aipro가 `--include-directories`를 지원하지 않는 구버전 포크일 가능성. 인자는 extraDirs가
  있을 때(코드베이스/스킬 폴더 지정 시)만 붙으므로 영향 범위가 한정되고, 실패 시 CLI 에러가 채팅에 표면화된다.
- **재검토 조건**: aipro가 플래그를 거부하면 `AgentDef`에 def 레벨 게이트(예: `include_dirs: bool`)를 추가해
  aipro만 프롬프트 언급으로 되돌린다. 기본값 지시문 개정은 D39 전체 교체형 특성상 **이미 워크플로우를 저장한
  사용자에게는 반영되지 않음**(Flows "기본값으로 되돌리기" 필요).

### D53. 타임아웃 정책: claude BASH env 상향 + HTTP 120초, 실행 엔진은 계속 무타임아웃
- **결정**: ① claude `RunSpec.env`에 `BASH_DEFAULT_TIMEOUT_MS=300000`/`BASH_MAX_TIMEOUT_MS=1200000`을
  추가한다 — 문서 생성 단계의 긴 도구 명령(빌드·전수 검색)이 Claude Code 기본 2분 타임아웃으로 중단되어
  "응답이 끊긴/무시된" 것처럼 보이는 문제를 방지. `run.rs`의 env 병합은 **부모 환경에 이미 있는 키를
  덮어쓰지 않도록** 변경(사용자 자체 설정 존중; `GEMINI_CLI_TRUST_WORKSPACE`도 동일 규칙 적용됨).
  ② `rag.rs`/`confluence.rs`의 reqwest 타임아웃을 30초 → **120초**(`HTTP_TIMEOUT` 상수)로 상향 — RAG 검색은
  턴 중간 preflight로 실행되어 느린 사내 백엔드에서 30초 타임아웃이 "단계 건너뜀"으로 나타났다. 수집 취소
  지연은 여전히 요청 1건으로 바운드. ③ **실행 엔진(run.rs)은 계속 타임아웃 없음**(무한 대기 = 의도된 설계;
  정지는 사용자 Stop). 탐지 프로브(3s/15s)는 무변경.
- **재검토 조건**: HTTP 타임아웃을 사용자별로 조정할 필요가 생기면 `RagConfig`/`ConfluenceConfig` 필드로 승격.

### D54. cmd 창 깜빡임(첫 질문 시) = 업스트림 CLI 버그 — 앱 측은 완결, 코드 변경 없음
- **결정**: "질문할 때 cmd 창이 잠깐 뜨는" 증상은 **앱의 spawn 누락이 아니다** — 백엔드의 모든 프로세스
  실행(탐지 프로브·실행 엔진·taskkill)은 단일 팩토리 `exec::command_for`를 거치며 예외 없이
  `CREATE_NO_WINDOW`가 적용됨을 감사로 확인했다. 잔여 깜빡임(주로 첫 질문/카테고리 진입 시)은 **Claude Code
  CLI 자체가 세션 시작 시 셸 스냅샷 등 자기 하위 프로세스를 windowsHide 없이 spawn하는 업스트림 버그**다
  (anthropics/claude-code #14828, #15572, #16880, #61051 — 버전에 따라 수정·회귀 반복). 앱 코드 변경은 없고
  [06-build-and-environment.md](06-build-and-environment.md)에 known-issue + "CLI 최신화" 안내를 기록한다.
- **대안 기각**: hidden desktop(`CreateProcessW` + `lpDesktop`)으로 자손 콘솔까지 숨기는 재작성 — std
  `Command`를 우회하는 대량 unsafe 코드가 필요해 위험 대비 이득이 없음. `DETACHED_PROCESS` — 콘솔 상속이
  끊겨 자손 콘솔 앱이 **새 가시 콘솔을 만들므로 오히려 악화**.
- **재검토 조건**: CLI 최신화 후에도 앱 유발 깜빡임이 재현되면 spawn 지점 감사를 재수행.

### D55. 스트림 이벤트 신뢰성: 동기 메시지 커밋 + nonce 소비 지연 + lossy 라인 리더 + 파서 방어
- **결정**: "응답이 사라지거나(빈 말풍선) 단계 자동 진행이 멈추는" 4가지 원인을 수정한다.
  ① **동기 커밋(`mutateMessages`)**: `ChatPanel`의 `messagesRef`가 passive effect로만 동기화되어, 연달아
  도착하는 이벤트(claude는 TextDelta→Usage→End가 한 번에 옴)에서 `end` 핸들러가 **stale 스냅샷으로 방금
  스트리밍된 응답을 빈 내용으로 덮어쓰고 그대로 영속화**했다. 모든 메시지 변경을 "ref에서 계산 → ref 즉시
  갱신 → setState" 헬퍼로 통일해 ref가 항상 최신이다(StrictMode 안전 — updater 부수효과 없음). `send()`의
  stale `messages` 클로저(transcript/append)도 ref 기반으로 수정.
  ② **nonce 소비 지연**: 폼 제출(`answerSubmission`)/자동 진행(`autoTurn`) 효과가 nonce를 `send()` 호출 전에
  소비했는데 `send()`에는 조용한 early-return(`streaming` 등)이 있어 **프리필과 겹친 제출이 영구 유실**됐다.
  `send()`가 처리 여부를 boolean으로 반환하고, 효과는 성공 시에만 nonce를 소비하며 `streaming`을 deps에
  넣어 재시도한다(이중 발사는 in-flight ref로 차단). 반환 규약: **부수효과 없이 차단된 경우만 false**
  (시도 후 실패는 에러 표시로 종결 = true — 무한 재시도 루프 방지).
  ③ **lossy 라인 리더(`stream_lines`)**: `BufRead::lines()`가 비 UTF-8 1바이트에 에러를 내면 리더가 스트림
  전체를 포기해 이후 응답이 전부 유실되고도 `End{succeeded}`가 나갔다. `read_until`+`from_utf8_lossy`로
  교체(stderr 드레인도 lossy).
  ④ **파서 방어**: gemini/aipro `message.content`가 파트 배열이면 통째로 드롭되던 것을 평탄화해 수용하고,
  claude `message.content`가 bare string인 경우도 수용(`stringify_tool_content`가 배열 내 bare string 지원).
- **근거**: 사용자 보고 두 증상(응답 소실·자동 진행 멈춤)의 코드 원인을 모두 제거. 신규 의존성 0.

---

### D56. 기동 안정성 패키지: 부트 진단 + 설정 백업 + poison 내성 + ErrorBoundary
- **결정**: 기동(부팅)·초기 로드 실패가 **무음으로 사라지거나 무음으로 망가지는** 4가지 경로를 보강한다.
  **신규 Cargo/npm 의존성 0.**
  ① **부트 진단(lib.rs)**: 유일한 부팅 패닉 지점이던 `.run(ctx).expect(...)`를 **`build()` + `app.run()`
  분리**로 바꾼다 — WebView2 런타임 부재/손상 등 웹뷰 생성 실패가 release(`windows_subsystem="windows"`,
  콘솔 없음)에서 "더블클릭해도 아무 일 없음"으로 증발하던 것을, 타입 있는 `Err`로 받아 처리한다.
  `run()` 첫 문장에서 **panic hook**을 설치해 모든 패닉을 로그로 남기고(기존 훅 위임으로 dev stderr 유지),
  실패는 `%USERPROFILE%\.operation-wizard\startup-error.log`(홈 루트 관례 재사용 — config_dir은 AppHandle
  없이는 해석 불가; fallback `HOME`→temp)로 기록한 뒤 **PowerShell 메시지박스**(`Add-Type
  PresentationFramework; MessageBox`)로 한글 안내(원인=WebView2 + 로그 경로)를 띄운다.
  `exec::command_for` 경유라 콘솔 깜빡임 없음. 다이얼로그는 `BOOT_PHASE`(AtomicBool,
  `RunEvent::Ready`에서 해제) 동안만 — 부팅 후 워커 패닉은 로그만(다이얼로그 스팸 방지). 타임스탬프는
  zero-dep civil-from-days 변환. debug 빌드 한정 `OW_SIMULATE_BOOT_FAILURE` env로 실패 경로를 리허설한다.
  ② **설정 파손 백업(settings.rs)**: `load()`가 파손 JSON을 `unwrap_or_default()`로 무음 초기화 →
  다음 save가 사용자 설정(에이전트 경로/스킬/워크플로우/RAG/Confluence) 전체를 지우던 것을,
  기본값 폴백 전에 원본을 **`settings.json.corrupt`로 보존**(keep-first: 이미 있으면 덮지 않음)한다.
  ③ **RunRegistry poison 내성(run.rs)**: `.lock().unwrap()` 5곳을
  `.unwrap_or_else(PoisonError::into_inner)`로 교체 — 워커 패닉 1회로 mutex가 영구 poison되어
  Stop(cancel_run)과 실행 북키핑이 전부 패닉하던 것을 복구한다.
  ④ **프론트 격리(신규 `ErrorBoundary`)**: 렌더 예외 1건이 React root 전체를 unmount(영구 백지 화면)하던
  것을, **root 바운더리(main.tsx) + 뷰 단위 keyed 바운더리(App.tsx, `key={view}`)**로 격리한다(한 뷰
  크래시에도 셸 생존, 뷰 이동=리마운트 복구; 폴백 = 다시 시도/앱 새로고침). 부트 로드
  (`getSettings`/`listAgents`)는 `.catch(() => {})` 무음 삼킴 → **`Promise.allSettled` + 실패 배너**
  (fixed 오버레이, 다시 시도=부트 effect 재실행)로 교체. `WorkspaceView`의 localStorage 접근 try/catch,
  `AgentCard`의 `settings?.agents?.[..]` 방어 체이닝.
- **근거**: 사내 잠긴 Windows 환경에서 가장 현실적인 부팅 실패(WebView2 부재)가 진단 불가능했고,
  설정 파손은 조용한 전체 데이터 손실이었다. 모두 표준 라이브러리/기존 패턴만으로 해결된다.
- **대안 기각(부팅 실패 안내)**: mshta VBScript — 사내 AppLocker/WDAC가 차단하는 대표 LOLBin.
  msg.exe — SKU 편차. `tauri-plugin-dialog` — 실패 시점에 AppHandle이 없음. `rfd`/`windows-sys` 직접
  의존 — 신규 의존성(단, `windows-sys`는 tauri 전이 의존이라 비용이 거의 0 — 에스컬레이션 경로로 명시).
- **대안 기각(poison 처리)**: IngestRegistry식 `Err("poisoned")` 반환 — mutex는 한 번 poison되면 계속
  poison이라 이후 모든 Stop/북키핑이 영구 실패하고, `wait()`·child kill 사이트는 Err 채널 자체가 없다.
  보호 데이터(HashMap/Child)에 패닉 중단으로 깨질 invariant가 없어 `into_inner` 복구가 안전하다.
- **대안 기각(백업 정책)**: 덮어쓰기 — `load()`는 커맨드마다 호출되어 "파손→save가 기본값 기록→재파손"
  시 사용자 원본이 기본값으로 클로버됨. 타임스탬프 다중 백업 — 파일 누적 대비 이득 없음(단순성 우선).
- **재검토 조건**: PowerShell이 차단된 환경 보고가 나오면 `windows-sys` `MessageBoxW`로 승격.
  백업 다중 세대가 필요해지면 타임스탬프 백업으로 확장.

---

### D57. 사용성 개선 패키지 1차 — 진행 스테퍼·재시도·이탈 확인·미탐지 온보딩·채팅 마크다운
- **배경**: 사용성 검토 보고서(`docs/usability-review.md`)의 P0 5건 + P1 6건 + P2 소형 2건을
  한 증분으로 구현. **신규 Cargo/npm 의존성 0, 백엔드 무변경**(전부 프론트).
- **결정 요점**:
  1. **워크플로우 진행 스테퍼**: `ChatPanel`이 runtime workflow 각 단계의 상태
     (`pending/active/done/skipped/halted`)를 state로 유지하고, 헤더 아래 **`WorkflowStepper`**
     (신규 컴포넌트: 세그먼트 바 + 접이식 체크리스트)로 상시 표시한다. 요구사항 폼 대기 중에도
     전체 단계가 미리 보여 "제출 시 무엇이 진행될지"의 미리보기를 겸한다. 로드 세션·단일 chat
     워크플로우(생성형 단계 없음)는 표시하지 않는다. 상태는 transient(영속화 안 함 — D34 관례).
  2. **채팅 응답 마크다운 렌더**: `AssistantMessage` 본문을 턴 완료 후 `MarkdownView`(D42의 캔버스
     렌더러 재사용, mermaid 포함)로 렌더한다. **스트리밍 중에는 평문 유지** — 부분 펜스/표가 델타마다
     재배치되는 깜빡임 방지. 코드블록 hover **복사 버튼**(mermaid 다이어그램은 숨김)과 응답 전체 복사
     버튼을 추가(`lib/clipboard.ts`: Clipboard API + `execCommand` 폴백).
  3. **같은 세션 재시도**: 실패 턴의 에러 박스에 1차 액션 **'다시 시도'** — 마지막 실전송 턴
     (text/opts/stepIndex)을 ref로 보관했다가, 실패 메시지 쌍(+뒤따른 중단 노트)을 제거하고 워크플로우
     커서를 복원한 뒤 같은 세션(resume)으로 재전송한다. '새 세션으로 다시 시도'는 2차 액션으로 유지.
     첫 턴 `ensure_project` 실패는 재시도 대상 밖(새 세션 경로).
  4. **이탈 확인 + 정지 피드백 통일**: 스트리밍 중 홈 이동/새 세션/기록 열기/NavRail 뷰 전환이 실행을
     조용히 죽이던 것을 `@tauri-apps/plugin-dialog`의 `ask` 확인으로 게이트한다(busy 상태는
     WorkspaceView→HomeArea→App으로 리프트; ChatPanel 언마운트 취소 동작 자체는 유지). 모든 정지·중단
     경로가 동일한 시스템 노트를 남긴다 — 취소: "작업을 중지했습니다 — 이후에는 일반 대화로
     진행됩니다.", 워크플로우 중 실패: "오류로 단계 진행을 중단했습니다…", 일반 대화 취소: "응답 생성을
     중지했습니다."
  5. **에이전트 미탐지 온보딩**: 탐지 완료 후 가용 에이전트가 0이면 홈 히어로 아래 경고 배너 + "Agents
     에서 설정" 버튼. 컴포저에서 미탐지 에이전트를 선택하면 경고 라인 + Agents 이동 링크. 드롭다운은
     "(탐지 중…)"과 "(미탐지)"를 구분 표기. 탐지 진행 중에는 홈에 "로컬 에이전트 탐지 중…" 표시.
  6. **preflight 가시화**: rag/knowledge preflight fetch 동안 "사내 문서 검색 중…"/"지식 베이스 확인
     중…" 일시 노트를 표시하고 완료 시 제거한다(120초 타임아웃 동안의 무표시 해소 — D53 연계).
  7. **파괴적 동작 확인 + Flows dirty 가드**: 지식 항목 삭제(영속본만; 미저장 초안은 즉시)·Flows
     단계/스킬 "기본값으로"·AgentCard Clear(저장값 있을 때)에 `ask` 확인. Flows 단계 편집기는
     "저장되지 않은 변경" 배지를 표시하고, dirty 상태의 카테고리 탭 전환(키 리마운트 = 무음 폐기)을
     확인으로 게이트한다.
  8. **무음 실패 표면화**: 최근 프로젝트 열기 실패(홈에 배너, 빈 새 채팅 폴백 제거), 세션 열기 실패
     (ChatPanel 헤더 아래 배너), 폴더 픽커 실패(홈·요구사항 폼 인라인).
  9. **스크롤**: 사용자가 하단 근처(bottom-pinned)일 때만 자동 스크롤하고, 벗어나면 "최신으로" 플로팅
     버튼을 띄운다(스트리밍 중 과거 읽기 보호).
  10. **표기 개선**: assistant 메시지 헤더에 실행 에이전트명(예: Claude Code)을 표시. RAG 미구현 스텁의
     개발자용 오류를 사용자용 문구로 치환(`foundation.ts::ragUserError` — 연결 테스트·rag 단계 건너뜀
     노트 공통).
- **대안 기각**: `window.confirm` — wry/Tauri 웹뷰는 JS 네이티브 다이얼로그를 지원하지 않음 → 기존
  의존성 `plugin-dialog`의 `ask` 사용(capability `dialog:default`에 이미 포함, 신규 권한 0). 스트리밍
  중 실시간 마크다운 렌더 — 부분 마크다운 재파싱/재배치 비용과 깜빡임이 커서 완료 시 전환이 우수.
  스테퍼 상태의 WorkspaceView 리프트 — ChatPanel 지역 state로 충분(리마운트=리셋이 D27 관례와 일치).
- **한계/재검토**: 언어 한글 통일(P2-12)·다크모드 토글·Escape 닫기·전체 재탐지 등 잔여 P2는 후속
  (보고서 로드맵 참조). 자동 연쇄의 턴 사이 짧은 비스트리밍 순간에는 이탈 확인이 걸리지 않는다(수용).

---

### D58. 캔버스 산출물 허브 · 문서 목차 · 다이어그램 갤러리 (프론트 전용)
- **결정**: 캔버스에 워크플로우 산출물 중심의 두 고정 탭과 뷰어 목차를 추가한다 — ① **'산출물' 탭**
  (`ArtifactsPanel`): 런타임 워크플로우의 `file` 있는 단계들을 좌측 산출물 목록(단계 상태 칩) + 우측
  미리보기(`FileViewer` 재사용)로 집계해, "구획별 여러 계획" 문서를 파일 탭 사냥 없이 한 곳에서 오간다.
  ② **'다이어그램' 탭**(`DiagramGallery`): 산출물 md 문서의 ` ```mermaid ` 펜스를 추출해 카드 갤러리
  (+확대 모달)로 렌더한다. ③ **md 미리보기 목차**(`FileViewer`): 렌더된 DOM에서 h1~h3을 추출한
  드롭다운으로 섹션 점프. **신규 Tauri 커맨드/의존성 0**(전부 프론트).
- **세부 결정**:
  - **산출물 데이터 = 파생 + 미러 + 프로브**: 목록은 `WorkspaceView`가 `artifactsFor(category, settings)`
    (`lib/artifacts.ts` — `runtimeWorkflowFor`의 file 있는 단계, 합성 `-html` 서브스텝 포함)로 파생한다
    (세션당 고정 — ChatPanel의 WF 고정과 동일 의미). 라이브 상태는 ChatPanel 소유의 `stepProgress`(D57)를
    새 `onStepProgress` 콜백으로 **미러만** 올리고(`onStreamingChange` 패턴), 파일 존재는 부모 폴더
    `list_dir` 프로브로 판정한다(`refreshNonce`로 재실행). 로드 세션(stepProgress 없음)은
    생성됨/미생성 존재 기반 칩으로 degrade하되 미리보기는 그대로 동작한다.
  - **산출물 라우팅(D49 개정)**: 워크플로우가 생성한 파일(`ChatPanel`의 `onOpenFile`)은 파일 탭을 만들지
    않고 산출물 탭에서 해당 항목을 선택한다(경로 비교는 `normalizePathKey` — Windows 대소문자/구분자
    정규화). 트리 클릭과 비산출물 파일은 기존 D49 파일 탭을 유지한다.
  - **목차 = DOM 쿼리**: 헤딩 id/슬러그를 만들지 않고 미리보기 커밋 후 DOM에서 추출, 클릭 시 재쿼리해
    엘리먼트 인덱스로 `scrollIntoView`. 한글 슬러그·중복 헤딩·원문/렌더 불일치 문제가 원천적으로 없고
    코드펜스 안 헤딩은 자동 제외된다.
  - **다이어그램 소스 = 산출물 md만**: 탭이 열릴 때 lazy 스캔(`read_file` + 펜스 추적 라인 스캐너
    `extractMermaidBlocks` — 다른 펜스 안에 중첩된 mermaid는 제외), 동일 코드는 dedupe("외 N곳" 칩).
    렌더는 마크다운 미리보기의 `MermaidDiagram`을 export해 재사용(다이어그램별 실패 폴백 포함).
  - **탭 수명**: 두 pill은 `workdir 확정 && 산출물 ≥1`일 때만 렌더(기본 chat-only인 guide/query/change는
    미표시), `effectiveTab` 가드는 rag 탭과 동형. 새 세션/기록 열기 시 `artifactSel`/`stepProgress` 초기화.
- **근거**: plan 워크플로우가 문서 4~5개를 만들면서 개별 파일 탭이 쌓여 문서 간 이동이 사용성 병목이었다.
  기존 인프라(파일 뷰어·mermaid 렌더러·stepProgress·refreshNonce)를 재사용하면 신규 백엔드 없이 집계
  뷰가 성립한다. [01](01-overview.md)의 "범위 밖"이던 아티팩트 집계 뷰의 부분 구현.
- **대안 기각**: `stepProgress`의 WorkspaceView 전체 리프트 — D57의 기각 유지(미러 콜백으로 충분).
  헤딩 anchor/슬러그 방식 목차 — 한글 슬러그·충돌 처리·ReactMarkdown 커스텀 renderer 비용 대비 이득 없음.
  workdir `docs/*.md` 전체 디스크 스캔 — 워크플로우 정의가 명확한 계약(사용자 편집 파일까지 끌어오면
  노이즈). 신규 Tauri 커맨드 — `list_dir`/`read_file`로 충분.
- **한계/재검토**: 과거 세션이 현 워크플로우 정의에 없는 산출물 파일을 남긴 경우 허브 목록에는 안 보인다
  (파일 트리/탭으로 열람 가능). `read_file` 2MiB 상한을 넘는 산출물은 다이어그램 스캔에서 조용히 스킵된다.
  다이어그램 스캔은 탭 오픈 시점 기준(백그라운드 감시 없음 — 재스캔 버튼/`refreshNonce`로 갱신). 산출물
  단계가 수십 개로 늘면 허브 목록 가상화/그룹화를 검토.

---

### D59. 워크플로우 산출물의 지식 저장 = 파일 복사 + 요약 본문 + 주입 시 경로 인덱스/extraDirs
- **배경**: 지식 베이스는 텍스트 전용(`title`+`body`)이고 주입 시 항목당 4,000자/전체 16KB로 잘려
  (D48), 완료된 작업의 산출물(md 문서·HTML 렌더 등)을 본문에 통째로 저장하면 이후 작업에는 앞부분만
  전달됐다. 사용자 요구 — "작업이 완료되면 산출물을 지식으로 저장해 이후 작업에서 참고".
- **결정(저장 구조)**: **파일 보관 + 요약 주입**. `KnowledgeEntry`에 옵셔널 필드 추가 —
  `kind`("note"/"artifact", plain string — settings.rs validate-on-save 관례), `files`(복사된 파일명
  목록), `sourceProjectId`/`sourceCategory`/`sourceTitle`(출처 표시용). 전부 `#[serde(default)]`라 구
  JSON 하위호환. 산출물 파일은 새 커맨드 **`save_knowledge_files(entry, sources)`**가
  `~/.operation-wizard/knowledge/artifacts/<entryId>/`로 **백엔드에서 복사**(프론트는 파일쓰기 불가 —
  D21; `read_file` 2MiB 상한도 미적용, 파일당 10MiB 가드만). 복사는 **staged swap**(`<id>.tmp`에 복사
  후 교체) — 재저장 중간 실패가 기존 파일 세트를 파괴하지 않고, 엔트리 JSON은 복사 완료 후에만
  기록된다. basename 충돌은 `-2`/`-3` 접미사(케이스 무시). 삭제는 기존 `delete_knowledge`가 폴더까지
  제거(멱등 유지). **`get_knowledge_root`** 커맨드가 절대경로를 제공한다(주입 인덱스·extraDirs용 —
  `list_knowledge` 응답에 절대경로를 넣으면 save 라운드트립 때 파생 데이터가 영속되는 문제로 기각).
- **결정(활용)**: knowledge preflight가 artifact 엔트리를 **요약(body, 4000자 클립) + 첨부 문서
  절대경로 인덱스 + "필요하면 원문을 직접 읽으라"는 안내**로 주입하고(`formatKnowledgeContext`에
  `artifactsRoot` 인자 추가), `knowledge/artifacts` **루트 1개**를 `knowledgeDirsRef`로 extraDirs에
  등록해(claude `--add-dir`, gemini/aipro `--include-directories` — D52와 동일 경로) 대화 내내
  에이전트가 원문 전체를 읽을 수 있게 한다. 원문은 절대 인라인하지 않으므로 16KB 상한과 무관.
- **결정(저장 트리거·UX)**: ① **완료 시 제안** — 종단 chat 도달 지점 2곳(end 핸들러의 자동전진 정지
  + preflight 스킵 체인)에서 `file`을 실제 생성한 단계가 있으면 세션당 1회, ChatPanel 컴포저 위
  dismissible 배너로 제안(캔버스 탭은 자동 전환하지 않음 — D58 산출물 라우팅과 충돌 방지).
  ② **수동 저장** — 산출물 탭 행 hover 액션(존재하는 산출물만). 두 경로 모두 **'지식 저장' 조건부
  캔버스 탭**(`KnowledgeSavePanel`, '요구사항' 탭 선례)을 연다 — 산출물 체크박스(존재 프로브는
  `useArtifactExistence` 공용 훅으로 추출, 미생성은 disabled)+제목+요약 편집+저장. `entryId`는
  세션당 1회 mint되어 저장 후에도 유지 → **같은 세션 재저장은 같은 엔트리 upsert**(새 세션=새 엔트리,
  v1 수용). 로드 세션(완료 배너 없음)은 수동 경로로 완전 동작.
- **결정(요약 생성)**: 패널 오픈 시 **격리 요약 턴**(`generateKnowledgeSummary` — 프리필 패턴:
  `runAgent` 직접 호출, 세션 id/resume 없음, ChatPanel 상태 불변)이 산출물 문서를 직접 읽고
  ` ```summary ` 펜스로 요약을 생성해 편집 가능한 textarea로 스트리밍한다(`fencedBlocks`를
  clarify.ts에서 export해 재사용; 펜스 미준수 시 평문 폴백 — plain 에이전트의 raw stdout도 수용).
  실패/취소 시 직접 작성 폴백 — **저장은 요약에 블록되지 않는다**. in-session resume 기각: 로드
  세션에서 stale하고, 세션리스는 transcript 재전송으로 더 비싸며, `streaming` 슬롯과 얽힘.
- **대안 기각**: 산출물 원문을 body로 저장(16KB/4000자 잘림 — 문제의 원인 그대로) / 엔트리별
  `--add-dir`(엔트리 수에 비례한 CLI 인자 폭증) / `files`를 폴더 나열로 파생(list마다 IPC+디렉터리
  읽기, 순서 비결정) / RAG ingest(rag.rs가 사용자 스텁 + Confluence 페이지 형태에 고정).
- **한계**: artifacts 폴더의 파일을 사용자가 수동 삭제하면 주입 인덱스는 남고 에이전트 읽기만
  실패한다(graceful, v1은 존재 프로브 없음). 새 세션에서 같은 프로젝트를 재저장하면 엔트리가
  중복된다(출처 표시로 식별 가능).
- **재검토 조건**: artifact 엔트리가 수십 건으로 늘면 종류별 주입 상한·선택 UI를 도입; 지식의 RAG
  승격(D48 재검토 조건)이 이뤄지면 산출물 원문도 RAG ingest로 승격.

---

### D60. 사용성 개선 패키지 2차 — 캔버스 레이아웃 수정·홈 재정비·제목 변경·장기 작업 가시화
- **배경**: 실사용 피드백 9건(캔버스 스크롤/목차/버튼 레이아웃 깨짐, 다이어그램 확대 부족, 폴더 칩
  표기, 프로젝트 제목 고정, 홈 첫인상, 홈 모델 선택 부재, 장기 턴 행 여부 분간 불가)을 한 증분으로
  구현. **신규 Cargo/npm 의존성 0, 백엔드 추가는 커맨드 1개(`set_project_title`).**
- **결정 요점**:
  1. **캔버스 flex 높이 제약 수정**: `FileViewer` 루트와 `ArtifactsPanel` 미리보기 래퍼에 `min-h-0`을
     추가한다 — flex 컬럼 아이템의 암묵적 `min-height:auto` 때문에 뷰어가 내용 높이만큼 자라
     본문 `overflow-auto`가 발동하지 않았고(스크롤바 없음), 목차 팝오버도 과성장한 루트를 기준으로
     `max-h-[60%]`가 계산되어 화면 밖으로 이어졌다. 팝오버 상한은 px 상한과 병행
     (`min(420px, calc(100%-48px))`).
  2. **파일바 shrink 규칙**: 경로 span만 `min-w-0 flex-1 truncate`로 줄고, 목차 버튼·미리보기/소스
     토글은 `shrink-0` + `whitespace-nowrap`으로 크기를 고정한다(좁은 산출물 탭에서 버튼이
     압축·밀려나던 문제).
  3. **다이어그램 확대 모달 재설계**: 콘텐츠 크기에 맞던 팝업을 **창 전체를 덮는 고정 크기 모달**
     (헤더 바 + 스크롤 본문)로 바꾸고 **확대/축소 컨트롤**(−/현재 %/+, % 클릭 = 원래 크기,
     0.25×~4×)을 단다. 배율은 CSS `transform`이 아니라 **`zoom`**으로 적용 — WebView2(Chromium)
     전용 앱이므로 사용 가능하고, 레이아웃에 반영되어 스크롤 컨테이너가 자연스럽게 따라온다.
  4. **파일 탭 폴더 칩 = 전체 경로**: basename("workspace") 대신 실제 절대경로를 표시한다.
     좌측 말줄임(`dir="rtl"` + LRM 마크로 꼬리 유지)이고 툴팁이 전체 경로를 보여준다.
  5. **프로젝트 제목 변경**: 새 커맨드 **`set_project_title(projectId, title)`**(매니페스트 read →
     mutate → rewrite, `set_project_codebase` 패턴; 빈 제목 거부, 100자 상한). 홈 최근 목록 행의
     hover 연필 버튼 → 인라인 입력(Enter 저장/Escape 취소)으로 노출한다. `ensure_project`는
     idempotent라 제목을 덮어쓰지 않으므로(재사용) 별도 갱신 커맨드가 정상 경로다.
  6. **홈 히어로 재정비**: "무엇을 도와드릴까요?" 채팅 인사 대신 **운영 도구 프레이밍** — 배지
     "Samsung SDS · Operation Wizard", 제목 "운영 작업 마법사", 부제에 진행 절차(요구사항 확인 →
     코드베이스 분석 → 사내 지식 반영 → 산출물 생성)와 기록 보장을 명시한다.
  7. **홈 컴포저 에이전트·모델 선택**: ChatPanel 컴포저와 동일한 셀렉트 2개를 홈 컴포저에 추가한다.
     선택값은 `onStart(category, prompt, workdir?, agentId?, model?)` → `HomeArea` →
     `WorkspaceView` → `ChatPanel`의 `initialAgentId`/`initialModel` prop으로 전달되어 초기 선택이
     된다(첫 턴 전까지 변경 가능). 명시 선택은 `agentPinnedRef`로 고정되어 탐지 결과 도착 시의
     자동 기본값 동기화가 덮어쓰지 않는다(컴포저에서의 수동 선택도 동일하게 고정).
  8. **장기 턴 라이브니스 표시**: 스트리밍 중 `AssistantMessage` 하단에 1초 틱 상태줄을 상시
     표시한다 — 스피너 + "작업 진행 중 · N분 N초 경과", 스트림이 15초 이상 조용하면 "마지막 응답
     N초 전"을 덧붙이고, 90초 이상이면 "오래 걸리는 작업일 수 있음 / 멈춘 것 같으면 중지 후 재시도"
     경고를 띄운다. 활동 시각은 content/thinking/events 변화로 갱신(컴포넌트 로컬, 영속화 없음).
     기존 "생각하는 중…" 표시는 이 상태줄로 대체.
- **대안 기각**: 다이어그램 배율에 `transform: scale` — 레이아웃 크기가 안 변해 스크롤 범위가
  틀어짐(`zoom`이 정확). 폴더 칩 중간 말줄임 JS 계산 — 가용 폭 대응이 조잡, CSS rtl 트릭 + 툴팁으로
  충분. 라이브니스를 ChatPanel 상태로 리프트 — 메시지 컴포넌트 로컬 타이머로 충분(D57 스테퍼 기각
  사유와 동일).
- **한계/재검토**: rtl 말줄임은 경로 끝의 중립 문자가 드물게 재배열될 수 있다(툴팁이 정본).
  라이브니스는 "스트림 이벤트 수신" 기준이라 CLI가 이벤트 없이 내부 작업하는 구간은 idle로
  보인다(수용 — 그래서 경고 문구가 중지/재시도를 안내). 홈에서 고른 에이전트/모델은 새 세션
  리마운트에도 초기값으로 유지된다(첫 턴 전 컴포저에서 재선택 가능).

---

### D61. 데이터 조회(query) 카테고리 다단계 기본 플로우 (Data Query Assistant 디자인 반영)
- **배경**: claude.ai/design의 "Data Query Assistant" 프로토타입(요구사항 명확화 → 정보 탐색 →
  참고 SQL → 테이블 정보 → 완료)을 데이터 조회 카테고리에 반영한다. 사용자 목표는 디자인 복제가 아니라
  **조회 단계 정비 + 도움이 되는 산출물(테이블 ERD·참고 SQL) 제공**이다.
- **핵심 판단**: 이 디자인은 기존 아키텍처에 거의 1:1로 매핑되어 **새 컴포넌트/StepKind/백엔드/의존성이
  전혀 없다** — plan과 동일한 클라이언트 오케스트레이션(D34/D36/D44) 위에서 **콘텐츠 카탈로그 정비**로
  구현한다. 매핑: STEP1 요구사항 명확화 = 옵션 프리플로우(D36), STEP2 정보 탐색 = **기반 3단계 그 자체**
  (codebase=참조 SQL 탐색 / rag=산출 기준 문서 / knowledge=용어→테이블 매핑, D44), STEP3 참고 SQL =
  `document` 단계, STEP4 테이블 정보/상관관계 = `document` + mermaid `erDiagram`(D42 미리보기 · D58
  다이어그램 갤러리), STEP5 완료·메모리 저장 = 종단 chat + 완료 배너 → 지식 저장 탭(D59, 기구현).
- **결정 요점**:
  1. **`DEFAULT_WORKFLOWS.query` 다단계화(`lib/workflow.ts`)**: 기존 `[chat]` 1개를
     `[query-codebase(codebase) → rag-search(rag) → knowledge(knowledge) → table-info(document,
     docs/table-info.md) → sql-draft(document, docs/query-sql.md) → chat]`로 교체. 기반 3단계는
     query 맞춤 지시문(참조 SQL 탐색·조회 기준 정리·용어 매핑)으로 배열에 직접 포함한다 —
     `coerceSteps`가 kind별로 stored 항목을 기본 트리오보다 우선하므로 카테고리별 맞춤 지시문이 성립
     (plan과 동일 패턴).
  2. **codebase 단계 `output:"file"` 명시**: `stepOutput`은 document가 아닌 kind(codebase 포함)의
     기본 output을 `"chat"`으로 파생하고 `expandOutputSteps`가 그 file을 스트립한다. 참조 SQL 후보
     목록(`docs/query-references.md`)을 **산출물로 보존**하려면 codebase 단계에 `output:"file"`을
     명시해야 한다. → 산출물 3종(참조목록 + ERD + 참고 SQL)이 산출물 허브·다이어그램 갤러리에 노출.
  3. **`foundationEnabled` 폴백 보정(`lib/workflow.ts`)**: 기존엔 `settings.workflows[category]`
     (저장된 override)만 보아, query처럼 **기본 워크플로우가 트리오를 갖는** 카테고리가 override 없으면
     false → `optionsFor`가 필수 `codebasePath` folder 질문(D45)을 프리펜드하지 않아 codebase 단계가
     경로 없이 실행됐다. `?? DEFAULT_WORKFLOWS[category]`로 폴백해 기본값의 트리오도 인식한다.
     Flows 토글 off 저장 시엔 stored에 기반 kind가 없어 false → 해제가 유지된다.
  4. **스킬 3종 추가(`lib/skills.ts`)**: `reference-sql-explore`(코드베이스에서 대상 테이블 참조
     SQL/DAO/매퍼 탐색, 지어내기 금지), `table-erd`(근거 기반 mermaid `erDiagram`·마스터 정보 표·관련
     프로그램 표·동일 집계 기존 화면 안내, 불확실은 "미확인"), `sql-draft`(읽기 전용 SELECT만, 머리
     주석 출처·기준, ERD 확인 컬럼만, 검토 포인트 필수, "참고용 초안" 경고). 기존 `query-safe`는 유지해
     sql-draft·종단 chat에 부착.
  5. **옵션 정비(`lib/options.ts`)**: `CATEGORY_OPTIONS.query`에 `knownTables`(text — 이미 아는
     테이블·컬럼·프로그램, 프리필 대상) 추가, `target` 문구를 조회 대상·기간·필터 중심으로 다듬음.
     codebasePath folder 질문은 (3)의 보정으로 자동 프리펜드.
- **백엔드 무변경**: `STEP_KINDS`/`STEP_OUTPUTS`/`CATEGORIES`(settings.rs) 모두 기존 값으로 충분,
  `validate_steps` 통과(마지막 chat + 기반 트리오 선두 순서). serde/타입 미러 변경 0. **신규 의존성 0.**
- **하위호환**: query 워크플로우를 이미 저장한 사용자는 stored override가 그대로 우선(전체 교체형 — D39).
  개편된 기본값은 Flows의 "기본값으로 되돌리기"로만 반영된다. guide/change 카테고리는 무변경.
- **한계/재검토**: 세션리스(gemini/aipro)·plain(opencode/antigravity)의 degrade는 D34/D40과 동일.
  실제 조회 실행·결과 표시는 범위 밖(참고용 SQL 산출까지). ERD·SQL 품질은 코드베이스에 참조 SQL이
  있는지에 크게 좌우된다(없으면 스킬이 "미확인"으로 표기).

---

### D62. 데이터 변경·권한(change) 카테고리 다단계 기본 플로우 + DC Manager 신청양식(HTML) + 본문 HTML 복사
- **배경**: `change`(데이터 변경·권한)는 단일 `chat` 단계뿐이었다(가이드 플로우 없음). 사용자 요구 —
  이 카테고리를 `query`(D61)처럼 다단계로 만들되 **초반에 변경 종류**(데이터 수정 / 테이블 생성 /
  테이블 권한 부여 / 스키마 변경)를 고르게 하고, **그 종류에 따라 결과 양식이 달라지게**, 최종적으로
  **운영서버 반영용 DC Manager 신청양식**을 **HTML로 생성해 본문을 서식째 복사**할 수 있게 한다.
- **핵심 판단**: query가 그대로 템플릿이라 **새 컴포넌트/StepKind/백엔드/의존성이 없다** — 콘텐츠 카탈로그
  정비(options/skills/workflow) + FileViewer/clipboard 1건뿐. 매핑: 초반 종류 선택 = 옵션 프리플로우(D36),
  코드베이스·ERD 파악 = 기반 3단계 + `table-erd`(query와 거의 동일, 사용자 명시), **차이 지점 = DC Manager
  신청양식 단계**.
- **결정 요점**:
  1. **종류별 결과 분기 = 워크플로우 분기가 아니라 스킬 분기.** 단계 배열은 카테고리당 고정이다(조건
     분기 미지원, D30/D34). `changeType` 답변은 폼 제출 시 wire에 주입되어 이후 턴에 계속 보이므로,
     **`dc-manager-form` 스킬이 4종 템플릿을 담고 주입된 종류에 맞는 폼을 생성**한다. 초반 탐색/ERD 단계는
     종류 무관하게 동일.
  2. **`DEFAULT_WORKFLOWS.change` 다단계화(`lib/workflow.ts`)**: `[change-codebase(codebase, output:file,
     docs/change-references.md) → rag-search(rag) → knowledge(knowledge) → change-table-info(document,
     docs/change-table-info.md — mermaid erDiagram) → dc-manager(document, docs/dc-manager-form.html) →
     chat]`. 기반 3단계는 change 맞춤 지시문(`CHANGE_*_STEP`)으로 배열에 직접 포함(coerceSteps가 kind별로
     stored를 기본 트리오보다 우선 — plan/query 패턴). codebase는 `output:"file"` 명시(기본 "chat"이면
     파일 스트립 — D61 패턴). `foundationEnabled`가 `?? DEFAULT_WORKFLOWS`로 트리오를 인식해 change도
     기반 3단계·`codebasePath` folder 질문(D45)이 자동 활성.
  3. **DC Manager 폼은 `document` 단계가 `.html`을 직접 저술**(`file: docs/dc-manager-form.html`, output은
     document 기본값 `"file"`로 파생). **`output:"html"`을 쓰지 않는다** — 그건 범용 `html-render` 스킬로
     "직전 md를 예쁘게 재렌더"하는 용도라 섹션 카드 chrome가 붙어 붙여넣기용 폼에 부적합. 전용
     `dc-manager-form` 스킬이 **인라인 style 속성 + 시맨틱 표** 중심의 자립형 HTML을 직접 만들어, `<body>`만
     복사해도 서식이 살아 붙게 한다(`<style>`/CSS 클래스는 리치 텍스트 붙여넣기 시 사라지므로 금지).
  4. **스킬 2종 추가(`lib/skills.ts`)**: `change-impact-explore`(대상 객체 DDL·참조/수정 지점 C·R·U·D·
     기존 변경 스크립트 탐색; 지어내기 금지), `dc-manager-form`(공통 신청정보 + 종류별 섹션 — DML:
     사전 건수 SELECT/실행 SQL/롤백 SQL, DDL 생성: CREATE·롤백 DROP, 권한: GRANT·REVOKE 방안, ALTER:
     영향 프로그램·마이그레이션·롤백 DDL; "신청 초안·재검증" 경고). 기존 `table-erd`/`change-safe` 재사용.
  5. **옵션 정비(`lib/options.ts`)**: `changeType`를 4종으로 교체(권한은 **부여만** — 사용자 확정), 프리필용
     `knownObjects`(이미 아는 대상 테이블·컬럼·객체) text 추가. `codebasePath` folder 질문은 `optionsFor`가
     기반 활성 시 자동 프리펜드.
  6. **본문 HTML 복사(`FileViewer`/`clipboard.ts`)**: FileViewer HTML 미리보기 파일바에 **"본문 복사"**
     버튼(`isHtml && content!==null`, `shrink-0`, Copy→Check 1500ms). 샌드박스 iframe은 `allow-scripts`뿐이라
     읽을 수 없으므로, 부모가 가진 원본 `content`를 `DOMParser`로 파싱해 `body.innerHTML`을 추출한다.
     `clipboard.ts`에 **`copyHtml(html, plain?)`** 추가 — `ClipboardItem`로 `text/html`+`text/plain` 동시
     기록, 실패 시 **hidden contenteditable 선택 + `execCommand("copy")`** 폴백(WebView2/Chromium). 기존
     `copyText`는 무변경. **모든 `.html` 미리보기에 공통** 제공(기존 `output:"html"` 산출물에도 이득).
- **백엔드 무변경**: `change`는 유효 카테고리, `html`은 유효 output, `validate_steps`는 카테고리별/기반순서
  제약이 없다(마지막 `chat`만 요구). `STEP_KINDS`에 codebase/rag/knowledge가 이미 있어 **새 kind도 없음**.
  serde/타입 미러 변경 0. **신규 의존성 0.**
- **대안 기각**: `output:"html"` + 범용 `html-render`(폼 대신 스타일된 문서 chrome, 붙여넣기 부적합) /
  종류별 워크플로우 분기(조건 분기 미지원 — 아키텍처 위배) / 종류별 스킬 4개(중복; 1개 스킬 4-템플릿 분기가
  단순) / iframe 안에서 복사(opaque origin이라 clipboard 접근 불가 → 부모 문자열 파싱).
- **하위호환**: change 워크플로우를 이미 저장한 사용자는 stored override가 우선(전체 교체형 — D39). 개편된
  기본값은 Flows "기본값으로 되돌리기"로만 반영. guide 카테고리는 무변경.
- **한계/재검토**: 세션리스(gemini/aipro)·plain(opencode/antigravity) degrade는 D34/D40과 동일. 실제 변경
  실행·승인 연동은 범위 밖(신청양식 산출까지). 붙여넣기 서식 보존은 인라인 스타일 사용에 의존(에이전트가
  `<style>`을 쓰면 body-복사 시 유실 — 스킬이 인라인을 강제하나 완전 보장은 아님). 종류가 늘면 스킬
  템플릿을 확장.

---

### D63. 운영 가이드(guide) 카테고리 다단계 기본 플로우 + RAG/Confluence 시각화 + 부분 foundation
- **배경**: `guide`(운영 가이드 생성)는 단일 `chat` 단계뿐이라(가이드 플로우·RAG·산출물 없음) 사내 문서
  활용도가 없었다. 사용자 요구 — 이 카테고리의 **강점으로 RAG/Confluence 정보를 시각적으로 보기 좋게**
  제공하고, **사용자가 어떤 절차로 업무를 수행하면 되는지 단계별 가이드**를 산출한다.
- **핵심 판단**: `query`(D61)/`change`(D62)가 그대로 템플릿이라 **새 컴포넌트/StepKind/백엔드/의존성이
  없다** — 콘텐츠 카탈로그(options/skills/workflow) 정비 + foundation 부분화 소폭 완화뿐. RAG 시각화는
  **기존 rag 단계 preflight → '검색 결과' 캔버스 탭**(sandbox iframe 카드 UI, D46)이 이미 제공하므로,
  guide에 rag 단계를 넣으면 자동으로 얻는다. 프로세스 가이드는 개정된 `guide-author` 스킬이 담당.
- **결정(분석 범위 — 코드베이스 제외)**: 사용자 확정으로 guide는 **Confluence·지식 중심**이다 — 기반
  3단계 중 `codebase`를 빼고 **`rag`+`knowledge`만** 사용한다. 이를 위해 foundation의 all-or-nothing
  불변식(D44)을 **부분 foundation**으로 완화한다:
  - `CATEGORY_FOUNDATION: Record<Category, readonly string[]>`(`workflow.ts`) — plan/query/change=완전
    트리오, **guide=`["rag","knowledge"]`**. `mandatoryFoundation(category, settings)`가 foundation이
    on일 때 이 종류들을 반환.
  - `coerceSteps(steps, {foundationKinds?})` — 지정 종류를 **canonical 순서로 pin**(누락은 defaults에서
    보충). 미지정이면 **present한 foundation 종류만** canonical **부분수열**로 pin(강제 채움 없음).
    기존 `foundation:boolean`은 ≡완전 트리오로 하위호환.
  - `optionsFor`(`options.ts`) — 필수 코드베이스 폴더 질문(D45)을 `foundationEnabled` 대신 **resolved
    워크플로우에 `codebase` 단계가 있을 때만** 프리펜드. guide는 codebase가 없어 **폴더 선택 강제 없음**.
  - `stepsError`(`FlowSettingsView`) canonical 검사를 **부분수열 허용**으로 완화(FOUNDATION_KINDS 인덱스가
    strictly increasing이면 통과 — `[rag,knowledge]` 허용, `[rag,codebase]` 거부). `toggleFoundation`은
    **카테고리 default의 foundation 단계**를 프리펜드(guide=rag+knowledge 맞춤 지시문), 토글 라벨도
    `CATEGORY_FOUNDATION`으로 동적화.
- **결정(산출물 — HTML)**: `guide-doc`(document) 단계가 `docs/operation-guide.md`를 저술하고
  **`output:"html"`**(D47)로 `expandOutputSteps`가 뒤에 `html-render` 합성 서브스텝을 붙여
  `docs/operation-guide.html`을 자동 생성한다. 마크다운 원본(산출물 허브·다이어그램 갤러리·목차)과
  보기 좋은 HTML(FileViewer 미리보기 + "본문 복사")을 모두 확보. 전용 렌더 스킬은 만들지 않는다.
- **결정(스킬/옵션)**: `guide-author` 스킬을 개정 — RAG/지식 근거·출처 인용, 전제→단계→검증→롤백 구조,
  프로세스 mermaid flowchart, 참고 문서(Confluence) 섹션, 지어내기 금지. guide 옵션에 `referenceDocs`
  (참고 Confluence 공간/키워드, text) 추가(프리필 대상). rag/knowledge 단계 지시문은 guide 맞춤
  (`GUIDE_RAG_STEP`/`GUIDE_KNOWLEDGE_STEP`/`GUIDE_DOC_STEP`).
- **기본 `guide` 플로우** = `[사내 문서 RAG 검색(rag) → 지식 베이스 반영(knowledge) → 운영 가이드
  작성(document, docs/operation-guide.md, output:html) → 마무리 대화(chat)]`.
- **백엔드 무변경**: `settings.rs::validate_steps`는 foundation 순서를 검증하지 않으므로(마지막 chat만)
  rag+knowledge-only 배열도 통과. `STEP_KINDS`/`STEP_OUTPUTS`/`CATEGORIES` 기존 값으로 충분. **신규 의존성 0.**
- **대안 기각**: 완전 기반 3단계 포함(코드베이스 폴더 강제 — 사용자가 명시 거부, 강점이 Confluence이므로
  마찰) / 전용 guide-HTML 저술 스킬(Q2가 md→html 자동 = html-render 재사용 선택 — 한 턴 부담↓, md 원본
  확보) / 별도 in-memory HTML 탭 신설(기존 rag 탭·산출물 HTML로 충분).
- **하위호환**: guide 워크플로우를 이미 저장한 사용자는 stored override 우선(전체 교체형 D39); 개편 기본값은
  Flows "기본값으로 되돌리기"로 반영. plan/query/change 동작 불변.
- **한계/재검토**: 세션리스(gemini/aipro)·plain(opencode/antigravity) degrade는 D34/D40과 동일. RAG 미설정·
  0건이면 rag 단계는 안내와 함께 건너뛴다(D44) — 그때 시각화 탭은 뜨지 않는다. guide의 실제 다단계
  운영 자동화(작업 실행 연동)는 범위 밖(가이드 문서 산출까지).

---

### D64. Fabrix = 첫 원격 HTTP API 에이전트 (레지스트리 kind 분기 + fabrix.rs + settings 저장)
> ⚠️ "첫/유일한 원격 에이전트"는 **D71에서 aipro가 두 번째 원격 에이전트로 합류**하며 갱신되었다.
> 그때 `detect_agent`/`run_agent`의 단일 `kind==Remote→fabrix` 분기가 **`def.id` match**로 일반화됐다.
> 아래는 최초 결정의 기록이다.

- **배경**: 기존 6개 에이전트는 전부 **로컬 CLI 바이너리**(resolve→probe→spawn→stdout 파싱)다. 새 에이전트
  **Fabrix**는 이 전제를 깨는 **원격 HTTP API**다 — 모델 목록은 `GET {endpoint}/openapi/chat/v1/all-models`,
  채팅은 `POST {endpoint}/openapi/chat/v1/messages` + **SSE 스트리밍**. 인증은 `x-fabrix-client`/
  `x-openapi-token` 요청 헤더. 사용자 요구 — "Agent 연결에 API 방식 1종 추가".
- **핵심 판단**: 프론트 에이전트 목록은 `list_agents` 레지스트리가 100% 구동하고, 실행 스트리밍은
  `RunEvent`+`Channel` 계약으로 이미 추상화돼 있다. 그래서 Fabrix를 **레지스트리에 7번째 def로 추가**하되
  탐지·실행 **두 지점에서만 원격 경로로 분기**하면 프론트 대부분(에이전트 셀렉트·모델 셀렉트·스트리밍 UI)이
  변경 없이 Fabrix를 소비한다. HTTP는 기존 `rag.rs`/`confluence.rs`의 `reqwest` blocking+native-tls(schannel)
  레시피 재사용 — **신규 crate 0**(SSE는 hand-roll: `data:` 라인을 lossy 리더로 읽어 JSON 파싱).
- **결정(레지스트리)**: `AgentDef`에 `kind: AgentKind { Local, Remote }` 추가. 기존 6개 def는 `Local`, Fabrix는
  `Remote`. Fabrix def는 CLI 필드(`bin_candidates`/`models_probe`/`run`)를 비워 두고(빈 슬라이스·`None`) 원격
  경로로 우회한다. `AGENT_DEFS`는 `[AgentDef; 6]`→`[AgentDef; 7]`. `DetectedAgent`의 `source`/`diagnostic`은
  자유형 `String`이라 원격용 값(`source:"remote"`, `diagnostic:"not-configured"|"unreachable"`)을 스키마
  변경 없이 담는다.
- **결정(모듈)**: 새 모듈 **`src-tauri/src/fabrix.rs`** 에 HTTP 로직 격리 — `parse_models_json`(최상위 배열의
  `modelId` + `name` 배열에서 `languageCode=="ko"` content를 label로, ko 없으면 첫 content, 그것도 없으면
  modelId; **합성 `default` 미prepend** — 채팅 API가 실제 modelId를 요구), `detect_fabrix`(설정 없음→
  `not-configured`, 있으면 GET all-models→성공 시 `available`+live models, 실패 시 `unreachable`),
  `parse_fabrix_sse_data`(`event_status=="CHUNK" && content`→`TextDelta`, 실패 status→`Error`, SUCCESS/R20000
  종료 마커→이벤트 없음), `run_fabrix`(POST+SSE 워커 스레드), `probe_fabrix`(연결 테스트 커맨드). 두 파서는
  순수·단위테스트.
- **결정(실행·취소)**: `run_agent`가 최상단에서 `kind==Remote`면 `fabrix::run_fabrix`로 위임. 프로세스가 없으므로
  `RunHandle.child`를 `Option`으로 바꿔 **자식 없는 취소 핸들**을 지원한다(`RunRegistry`에 `next_id`/
  `register_remote`/`unregister` 헬퍼 추가) — 단일 레지스트리·단일 runId 공간을 유지해 `cancel_run`이 두 경로를
  모두 처리(프로세스는 taskkill, HTTP는 플래그만 set → SSE 읽기 루프가 관측해 연결 종료). 종료 규약은 기존
  그대로: 실패면 `Error` 선행 후 단일 `End`.
- **결정(설정·UI)**: 자격증명은 **앱 `settings.json`** 에 `FabrixConfig{endpointUrl, client, openapiToken,
  allowInvalidCerts}`로 저장(`RagConfig`/`ConfluenceConfig`와 동일 루트 — D39 단일 설정 원칙 유지, 새 파일
  IO 없음). 커맨드 `set_fabrix_config`(`set_rag_config` 미러) + `probe_fabrix`. UI는 **Agents 화면의 전용
  `FabrixCard`**(탐지 상태·모델 목록 + endpoint/client/token 3필드 + 연결 테스트; `AgentsView`가 `id=="fabrix"`
  분기, `RagSection` 폼 패턴 재사용). 시크릿은 평문 저장(로컬 단일사용자 — 기존 관례, 읽기 전용 키 권장).
- **결정(채팅 파라미터)**: **`isStream: true`**(토큰 스트리밍 — 파이썬 샘플의 모순된 `False` 대신), `llmConfig`는
  샘플 기본값(max_new_tokens 2024, top_k 14, top_p 0.94, temperature 0.4, repetition_penalty 1.04), `contents`는
  세션리스 관례대로 `[transcript]` 단일 요소, `systemPrompt`는 기본 문구. Fabrix는 세션리스(ChatPanel
  `SESSION_AGENTS`에 미포함) → 매 턴 transcript 재전송. 모델은 실제 modelId 필수라 ChatPanel이 현재 model이
  목록에 없으면 첫 모델로 스냅(CLI 에이전트는 `default`가 목록에 있어 무영향).
- **대안 기각**: 홈폴더 `~/.operation-wizard/fabrix.json` 별도 파일(사용자가 언급했으나 `.operator-wizard`는
  오타로 판단; settings.json이 이미 영속화 제공 — 설정 루트 이원화 회피, D39) / `RunSpec`에 `FabrixSse`
  StreamFormat 추가(전송이 프로세스 전제와 근본적으로 달라 별도 모듈이 깔끔) / 별도 취소 레지스트리
  (`RunHandle.child`를 Option으로 두면 단일 레지스트리로 충분).
- **하위호환**: `FabrixConfig`는 `#[serde(default)]`라 구 settings.json 무변경 로드. 기존 6개 에이전트 동작 불변.
- **한계/재검토**: SSE 취소는 `data:` 라인 사이 플래그 확인이라 서버가 조용하면 다음 바이트까지 지연
  (confluence 취소와 동형; 토큰 스트림은 자주 방출되어 실사용 무리 없음). `contents`의 교대형 배열 완전 활용은
  후속(RunArgs가 단일 prompt 문자열). 사내 프록시 TLS는 `allowInvalidCerts` opt-in으로 대응. opencode/
  antigravity의 1급 파서처럼 Fabrix도 도구 이벤트는 미지원(텍스트 스트림만).

---

### D65. RAG 검색 = Fabrix rag-chat API 실연동 (rag.rs search 구현 + knowledgeAssetId 설정 + /models 연결 테스트)
> ⚠️ **참고(D82)**: `RagClient::ingest_page`는 여전히 스텁이나, **Confluence가 더 이상 이 sink를 타지 않는다**
> (D82에서 Confluence 수집을 공식 MCP → 로컬 지식 베이스로 전환). RAG **검색**(search) 실연동은 그대로 유효.
- **배경**: `rag.rs`의 `RagClient::search`가 `TODO(user)` 스텁이라 rag 기반 단계(D44/D48)가 항상 "미구현 —
  건너뜀"으로 degrade했다. 사용자 요구 — 실제 사내 RAG(**Samsung SDS Fabrix `rag-chat` API**) 호출로 채운다.
  샘플 계약: 모델 목록 `GET {endpoint}/openapi/rag-chat/v1/models`, 채팅 `POST .../v1/messages`
  (body `{modelIds:[MODEL], contents:[query], isStream:false, llmConfig:{}, systemPrompt:"", knowledgeAssetId}`),
  응답(비스트림 단일 JSON) `{content:"요약 답변", references:[…], contentReferences:[{references:[{title,content,link,filename}]}], status:"SUCCESS"}`.
- **핵심 판단**: 인증 헤더가 **Fabrix(D64)와 동일**(`x-fabrix-client`/`x-openapi-token`)이라 D50의 `RagConfig`
  (`endpoint`/`secret_key`/`pass_key`/`top_k`)에 그대로 매핑된다. 채우는 지점은 스텁 하나뿐 — 설정 로드·
  `spawn_blocking`·커맨드 등록·프론트 `RagHit[]` 소비 파이프라인은 이미 배선됨. **신규 crate 0**(reqwest
  blocking+native-tls 재사용, `fabrix.rs` 레시피).
- **결정(헤더 매핑)**: `secret_key`→`x-fabrix-client`, `pass_key`→`x-openapi-token`(있을 때만; `attach_headers`
  헬퍼). serde 필드명은 그대로 두고 **UI 라벨만 실제 헤더명으로 변경**(마이그레이션 0). `x-generative-ai-user-email`
  헤더는 **생략**(사용자 결정 — 3개 필수 파라미터만; API가 요구하면 필드 1개 추가로 확장 — 재검토 참조).
- **결정(모델·자산)**: 모델은 **GLM 5.2 하드코딩**(`019f23a1-…`, `rag.rs` 상수; UI 없음). `knowledgeAssetId`는
  `RagConfig`에 추가한 설정 필드 — 비어 있으면 샘플 자산(`019f5a11-…`)으로 폴백해 즉시 동작(사용자 요구
  "일단은 샘플 id 그대로"). `/models`는 **연결 테스트 전용**(모델 발견).
- **결정(응답 매핑)**: `parse_rag_response`(순수 fn, 단위테스트) — `content`(요약 답변)를 맨 앞 `RagHit`
  (title "RAG 요약 답변")으로 prepend + 출처 청크(`contentReferences[].references[]`, `link` 보유; 비면 top-level
  `references[]` 폴백)를 `RagHit`으로 매핑, 출처만 `top_k` truncate. 기존 `formatRagContext`/`ragResultHtml`이
  `RagHit[]`만 소비하므로 **계약·프론트 렌더 변경 0**(요약 답변은 첫 hit으로 자연 표출). `status`가 FAIL/ERROR면
  `Err`(프론트 → "건너뜀"). `score`는 rankScore가 0~1 유사도가 아니라 `None`으로 둠(0.00 표시 방지).
- **결정(연결 테스트)**: 새 커맨드 **`probe_rag`**(`probe_fabrix` 미러, spawn_blocking) — `/models` GET →
  `crate::fabrix::parse_models_json` 재사용(응답 shape 동일) → "연결됨 (N개 모델)". assetId 없이 자격증명·도달성
  검증. `RagSection` 연결 테스트가 `ragSearch` 더미쿼리 대신 `probeRag`를 호출(assetId 불필요).
- **결정(ingest 유지)**: rag-chat API에는 ingest 엔드포인트가 없다(지식 자산은 Fabrix 플랫폼에서 관리) →
  `ingest_page`는 **TODO 스텁 유지**(Confluence 수집은 별개, 여전히 per-page 실패로 degrade). `RagClient::new`
  시그니처·`IngestPage` 무변경 → `confluence.rs` 영향 0.
- **대안 기각**: 출처만 매핑(요약 답변 버림 — 사용자 요구와 반대) / 모델 드롭다운(현재 GLM 5.2 고정으로 충분) /
  `RagConfig` 키 이름을 `client`/`openapi_token`으로 개명(마이그레이션·프론트 필드 변경 비용, D50 계약 재활용이 최소).
- **하위호환**: `knowledge_asset_id`는 `#[serde(default, skip_serializing_if=Option::is_none)]`라 구 settings.json
  무변경 로드(구 `apiKey`도 여전히 unknown-field로 무시 — D50). 이미 rag를 저장한 사용자는 재저장 시 필드가 추가된다.
- **한계/재검토**: 이메일 헤더 생략으로 API가 거부하면 `RagConfig.userEmail` + 필드 추가로 확장. `top_k`는 서버가
  받지 않고 반환 청크 수 상한으로만 쓰인다(rag-chat에 topK 파라미터 없음). SSE 스트리밍(`isStream:true`) 미사용
  (검색은 단발 JSON으로 충분).

---

### D66. Fabrix/RAG/Confluence 프록시 우회(`.no_proxy()`) + 모델 목록 캐시 우선 영속
- **배경**: 사내망에서 Fabrix 원격 에이전트와 RAG 서비스(및 Confluence)는 **직접 도달 가능한 사내
  엔드포인트**인데, `reqwest`가 기본적으로 환경변수(`HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`)의 프록시를
  자동 상속해 이들 연결이 프록시를 경유하며 실패/오작동할 수 있었다. 또 모델 목록은 매번 라이브로만
  조회되고 설정에 저장되지 않아, 앱 시작마다 네트워크 호출이 필요하고 일시 장애 시 모델 드롭다운이 비었다.
  사용자 요구 — ① 이 연결들은 프록시를 타지 않고 직접 연결, ② 모델 목록을 처음(저장/새로고침/연결
  테스트) API로 조회한 뒤 설정 파일에 저장하고 이후엔 저장본 사용.
- **결정(프록시 우회)**: 세 HTTP 클라이언트 빌더(`fabrix.rs::build_client`, `rag.rs::RagClient::new`,
  `confluence.rs::HttpConfluence::new`)에 **`.no_proxy()`**를 추가해 환경 프록시를 무시하고 직접 연결한다.
  TLS는 기존 native-tls/schannel(OS 인증서 저장소) 그대로라 사내 프록시 CA 신뢰는 유지된다. **신규
  crate 0.** (Confluence는 Fabrix/RAG와 별개 연결이나 동일 사내 직접 도달 전제라 사용자 결정으로 포함.)
- **결정(모델 캐시 우선)**: `FabrixConfig`/`RagConfig`에 `models: Vec<ModelOption>`(`#[serde(default,
  skip_serializing_if="Vec::is_empty")]`, 구 settings.json 하위호환)을 추가하고 `ModelOption`에
  `Deserialize/Debug/PartialEq` 파생을 확장한다(`detect.rs`). **캐시 우선 동작**:
  - `detect_agent`에 **`force: bool` 인자** 추가. 원격 분기는 `fabrix::detect_fabrix(cfg, force)` 호출.
    `!force && 캐시 있음`이면 **네트워크 없이** 캐시 모델을 반환(`models_source="fallback"`); 그 외
    (강제이거나 캐시 없음=최초)에는 라이브 조회. 라이브 성공 시 `detect_agent`가 `fabrix.models`에 저장.
    라이브 실패 & 캐시 있음이면 캐시를 폴백으로 표시.
  - 프론트 `detectOne(id, force)`: **시작 시 탐지는 force 없이(캐시)**, **수동 Refresh·저장 후
    재탐지는 `force=true`**(라이브 조회 → 캐시 갱신).
  - **연결 테스트**(`probe_fabrix`/`probe_rag`)와 **저장 흐름**도 라이브 조회 후 `models`를 저장한다.
    RAG는 에이전트가 아니라 `probe_rag`가 모델 목록(리네임 `fetch_models`)을 조회·저장하고, RagSection이
    저장/연결 테스트 후 `getSettings()`로 재조회해 목록을 표시한다(Fabrix 카드와 동일 UX).
  - `set_fabrix_config`/`set_rag_config`는 `models`를 **백엔드 소유**로 다룬다 — 연결(endpoint+자격증명)이
    동일한 재저장이면 기존 캐시 이월, 바뀌면 비움(다른 서버 목록 방지). 프론트는 `models`를 보내지 않는다.
- **참고**: RAG 실제 검색(`search`)은 고정 모델(GLM 5.2)을 계속 사용한다 — `rag.models`는 정보 표시용
  캐시이며 검색 로직에는 영향 없음(D65 유지).
- **대안 기각**: 프록시 자동 상속 유지(사내 직접 엔드포인트에 부적합) / **라이브 우선**(탐지마다 조회, 저장본은
  폴백으로만) — 사용자가 "처음엔 API, 이후엔 저장" 캐시 우선을 선택 / 별도 모델 캐시 파일(settings.json이 이미
  영속 제공 — 설정 루트 이원화 회피, D39 단일 설정 원칙).
- **하위호환**: `models` 필드는 `#[serde(default)]`라 구 settings.json 무변경 로드(빈 벡터). 기존 동작 불변.
- **한계/재검토**: 캐시 우선이라 시작 시 도달성을 재확인하지 않는다(저장본을 available로 낙관 표시; 실제
  연결 문제는 채팅/연결 테스트에서 드러남). 라이브 확인이 필요하면 Refresh/연결 테스트를 쓴다.

---

### D67. Fabrix(원격 에이전트) 산출물 = 앱이 스트리밍 텍스트를 파일로 저장 (files.rs write_file 신설, D21 개정)
- **배경**: Fabrix는 **파일시스템 접근도 도구도 없이 텍스트만 SSE로 스트리밍**한다(`kind: Remote`, D64 —
  `run_fabrix`가 `RunEvent::TextDelta`만 방출). 로컬 CLI 에이전트는 워크플로우 `document` 단계에서 자기
  파일 쓰기 도구로 `docs/*.md`를 직접 생성하지만, Fabrix는 아무 파일도 쓰지 못한다. 결과: Fabrix로
  워크플로우를 돌리면 `end` 핸들러가 `onOpenFile(joinWorkdirPath(cwd, step.file))`를 호출해도 디스크에
  파일이 없어 산출물 탭(`ArtifactsPanel`)의 존재 프로브(`useArtifactExistence`→`list_dir`)가 못 찾고,
  응답이 **대화 패널 텍스트로만** 남았다. 사용자 요구 — 다른 CLI 모델처럼 워크스페이스에 실제 파일이
  생기고 산출물 탭에서 확인되게 한다.
- **결정(백엔드 write_file — D21 개정)**: `files.rs`에 세 번째 커맨드 **`write_file(path, contents)`**를
  추가한다(부모 디렉터리 `create_dir_all` + `MAX_WRITE=5 MiB` 가드 + `fs::write`). `files.rs`의 read-only
  방침(D21)을 최소로 개정하되, 별도 `fs` 플러그인/capability는 도입하지 않는다(커스텀 커맨드는 core IPC
  — D21 선례 유지). `lib.rs` invoke_handler에 `files::write_file` 등록, 프론트 `api.ts`에 `writeFile` 래퍼.
- **결정(클라이언트 영속화 — 원격 전용 게이트)**: `ChatPanel`의 `end` 생성형-성공 분기에서 `step.file`이
  있고 **에이전트가 원격**(`detected[agentId]?.source === "remote"`)이면, 마지막 assistant 메시지의 누적
  `content`를 문서 본문으로 정제(`extractDocBody` — 응답 전체가 단일 펜스면 언랩, 아니면 trim)해
  `writeFile(abs, body)`로 저장한 **뒤** `onOpenFile(abs)`를 호출한다(순서 중요 — refreshNonce 범프 전에
  파일이 존재해야 존재 프로브가 발견). CLI(비원격) 경로는 **무변경**(자기 파일을 계속 씀 — 앱이 덮어쓰지
  않음, 회귀 위험 0). 자동전진(`setAutoTurn`)은 동기로 별개 발사되어 write와 무관.
- **결정(원격 문서 프롬프트)**: `send()` wire 조립에서 원격 에이전트의 생성형+`file` 단계에 짧은 지시문
  (`remoteDocCtx`: "출력 전체가 문서로 저장됨 — 파일 도구/저장 언급 없이 문서 본문만 출력")을 `pathCtx`
  뒤에 주입한다. 기존 `document` 지시문("파일 쓰기 도구로 저장")이 Fabrix엔 무의미하고 "…에 저장하겠습니다"
  류 잡담을 유발하므로. CLI wire는 게이트로 격리(무변경).
- **결정(토큰 상한)**: `fabrix.rs::chat_body`의 `max_new_tokens` 2024 → **8192**(긴 문서 잘림 방지; 모든
  Fabrix 채팅에 적용).
- **대안 기각**: 백엔드가 `step.file`을 알고 `run_fabrix`에서 직접 쓰기 — 워크플로우 개념을 실행 엔진에
  누출(클라이언트 오케스트레이션 원칙 D30/D34 위배). / 존재-누락 시 전 에이전트 대상 쓰기 — plain
  에이전트에도 도움이 되나 비결정적이고 이전 실행의 stale 파일을 주울 수 있어 원격 전용이 안전(사용자 확정).
- **하위호환/영향**: 신규 Cargo/npm 의존성 0. CLI 에이전트 동작 불변. `write_file`은 프론트가 항상
  `<workdir>/<step.file>`만 넘기므로 로컬 단일사용자 신뢰 모델(D21)과 일치.
- **한계**: Fabrix는 파일·도구가 없어 **코드베이스 실독·RAG 도구 호출을 못 한다** — `codebase` 단계 등의
  산출물이 추정 기반일 수 있다(순수 채팅 API의 본질적 제약; 본 결정 범위 밖). rag/knowledge preflight
  주입은 기존대로 동작.

---

### D68. 워크플로우 생성형 단계의 일시적 타임아웃 자동 재시도 (aipro 백엔드 간헐 지연 흡수)
- **배경**: AI Pro(`aipro`) 실행 시 `Streaming request timeout after ~45s`로 워크플로우가 중단되는 문제를
  조사한 결과, 원인은 **aipro 백엔드(glm-5.1)의 간헐적 first-byte 지연(>45초)** 으로 확정됐다(진단 요지:
  ① 45초는 백엔드가 내려주는 요청 타임아웃 — `getRequestTimeoutMs`의 서버 실험 플래그
  `DEFAULT_REQUEST_TIMEOUT`→undici `headersTimeout`, 클라이언트에서 못 늘림; ② 앱을 배제한 순수 aipro도
  성공/실패를 모두 재현 — 동일 프롬프트가 어떤 때는 7분+ 실패, 10분 뒤 성공; ③ 입력 크기·실행 방식
  (`GEMINI_CLI_TRUST_WORKSPACE`·`--include-directories`·cwd·cmd.exe 래핑)·프록시·프롬프트 내용 모두 A/B로
  배제). 앱이 유독 자주 실패하는 이유는 **가이드 플로우가 한 세션에서 aipro 요청을 여러 번 순차 발사**해,
  요청당 실패확률 p에 대해 N단계 실패확률 ≈ 1−(1−p)^N로 **증폭**되기 때문이다.
- **결정**: 백엔드 45초는 클라이언트에서 못 늘리므로, **워크플로우 생성형 단계의 턴이 일시적 오류로
  실패하면 중단(halt) 전에 같은 단계를 자동으로 최대 `MAX_STEP_RETRIES`(=2)회 재시도**한다(`ChatPanel`의
  `end` 실패 분기). 재시도는 실패한 메시지 쌍을 제거(세션리스 transcript 오염 방지)하고 단계를 re-arm한 뒤
  **기존 자동전진 경로(`setAutoTurn` nonce+effect)** 로 재발사한다(스트림 핸들러에서 `send()`를 직접 호출하지
  않는 D55 규약 유지). 진행 스테퍼/시스템 노트에 "일시적 오류로 재시도 중 (k/2)"를 표시하고, 소진 시
  기존 halt 노트(+"여러 번 재시도했지만 실패")로 폴백한다(수동 '다시 시도' D57은 그대로).
- **선별 재시도(중요)**: 무한/무의미 재시도를 막기 위해 **일시적 오류에만** 적용한다 —
  `isTransientFailure`가 timeout/stream/network 시그니처는 재시도 대상으로, **치명적 오류(Model not found·
  TLS BadSignature/인증서·401/403·Credential 가드레일 — D28 부류)는 제외**한다. 트리거 조건: `status==="failed"`
  (취소 제외) + 생성형 단계(`isGenerative`) + 예산 잔여 + 일시적 시그니처. 오류 텍스트는 `run.rs`가 실패 시
  stderr를 `RunEvent::Error`로 중계하므로 마지막 assistant 메시지 `error`에서 읽는다.
- **근거**: 재시도로 대부분 흡수된다(동일 프롬프트가 재실행에서 성공하는 간헐성). 단조 커서·1회성 arm·
  종단 chat 등 기존 안전장치(D34)와 정합하며, 자동전진 인프라를 재사용해 **신규 백엔드/IPC/의존성 0**
  (순수 프론트, `ChatPanel`만). 세션형(claude/codex)에도 적용되나 그쪽은 다른 백엔드라 이 타임아웃과
  대체로 무관(무해).
- **대안 기각**: 백엔드 타임아웃 상향(클라이언트 불가 — 서버 실험 플래그) / 입력 축소(15만 토큰도 성공 —
  크기 무관, 비효과) / 전 실패 무조건 재시도(치명적 오류를 반복 — 시간 낭비·오해 유발). 근본 해결(백엔드
  first-byte 지연 개선/`DEFAULT_REQUEST_TIMEOUT` 상향)은 사내 aipro 운영팀 몫으로 범위 밖.
- **재검토 조건**: 재시도 2회로 부족한 사용자 보고가 잦으면 `MAX_STEP_RETRIES`를 설정화하거나 지수 백오프
  지연을 추가한다.

---

### D69. 캔버스 폴더 칩 = 좌측 말줄임 축약(고정폭 버튼 보호) + "탐색기에서 열기" 버튼
- **배경**: 워크스페이스(좌 대화 + 우 캔버스)의 캔버스 파일 탭 툴바 우측에는 활성 작업 폴더의 **절대경로
  폴더 칩**(D60)이 있다. 이 칩이 `max-w-[340px]`까지 커지고 flexbox 기본 `flex-shrink:1`이라, 좁은 창에서
  바로 왼쪽의 **새로고침 버튼**(`h-7 w-7`)·루트 전환 세그먼트가 함께 짓눌려 아이콘이 찌그러지거나 밀리는
  문제가 있었다. 사용자 요구 — ① 경로는 길어지면 `…`로 간략히, ② **탐색기로 그 경로를 여는 버튼** 추가.
- **결정**: **프론트 전용**(`CanvasPanel`/`api.ts`/capability 1줄). ① 툴바에서 **폴더 칩만 축소를 허용**하고
  (`min-w-0` + `max-w-[240px]`, 내부 `truncate`) 나머지 컨트롤(루트 전환 세그먼트·새로고침·신규 탐색기
  버튼)에 **`shrink-0`**을 부여해 긴 경로가 버튼을 짓누르지 못하게 한다. 좌측 말줄임(`dir="rtl"` + LRM
  마크, D60)과 전체 경로 툴팁은 유지한다(칩 최대폭만 340→240으로 축소해 "간략" 요구 충족).
  ② 폴더 칩 오른쪽에 **`ExternalLink` 아이콘 버튼**("탐색기에서 열기")을 추가한다 — `treeRoot`(작업 폴더
  ↔ 코드베이스 토글에 따른 현재 트리 루트)를 `openInExplorer`(`api.ts`, `@tauri-apps/plugin-opener`의
  `openPath`)로 OS 탐색기에 **폴더 내용 그대로** 연다. 실패는 조용히 무시(`.catch(() => {})` — 부수 액션).
- **권한**: `openPath`는 `opener:default`에 없어 capability(`default.json`)에 **`opener:allow-open-path`**를
  추가한다. **신규 Cargo/npm 의존성 0**(`tauri-plugin-opener`·`@tauri-apps/plugin-opener`는 이미 사용 중).
- **대안 기각**: `revealItemInDir`(`opener:default`에 이미 포함 — 권한 추가 불필요) — Windows에선 폴더의
  **부모를 열고 그 폴더를 선택**만 하므로 "그 경로(내용)를 띄운다"는 요구와 어긋난다. `openPath`가 폴더
  자체를 열어 더 부합. / 폴더 칩 폭을 반응형 계산(JS) — CSS `min-w-0`/`shrink-0`로 충분.
- **한계/재검토**: `openPath`는 스코프 없이 임의 경로를 열 수 있으나, 프론트가 넘기는 값은 항상 프로젝트
  workdir/코드베이스 경로라 로컬 단일사용자 신뢰 모델(D21)과 일치한다. 스코프 제한이 필요해지면 opener
  scope로 승격.

---

### D70. RAG 검색 결과 관련성 LLM 판단 게이트 + 정리된 '검색 결과' 패널 (D44/D46 확장)
- **배경**: 기반 3단계의 **rag 단계**(D44)는 검색 결과가 1건이라도 있으면 **무조건** (a) 캔버스 '검색 결과'
  탭(D46, `ragResultHtml`)에 표시하고 (b) `formatRagContext`로 에이전트 프롬프트에 주입했다(유일한 게이트는
  "0건 → 건너뜀"). 사내 지식베이스는 이번 작업과 무관한 문서까지 top-K로 돌려주는 경우가 많아, 관련 없는
  정보가 패널을 채우고 프롬프트를 오염시켰다. 사용자 요구 — ① 관련이 적다고 **판단되면** 사용자에게
  보여주지 않는다, ② 관련이 있으면 **HTML로 보기 좋게 정리된 패널**로 보여준다.
- **핵심 판단**: RAG/Confluence는 이 앱에서 **rag 단계 하나로 수렴**한다(Confluence 페이지는 RAG 소스 청크의
  `url`로 표출 — 별도 패널 없음). 관련성 "판단"은 점수가 아니라 **작업 맥락 기반 LLM 판단**이 정확하고,
  이미 있는 격리-턴 관용구(`knowledgeSave.ts::generateKnowledgeSummary` — D59)와 fenced-block 파서(D30 계열)를
  그대로 재사용하면 **신규 백엔드/IPC/의존성 0**(순수 프론트)으로 성립한다.
- **결정(판단·정리)**: 새 모듈 `src/lib/ragRelevance.ts` — `judgeRagRelevance`가 RAG 검색 직후, 캔버스 표시·
  프롬프트 주입 **전에** 격리된 에이전트 턴 1회를 실행한다(세션 id/resume 없음, 자체 `Channel`, `extraDirs:[]`).
  이 턴은 (작업 내용=검색 query + 검색 결과)를 읽고 **관련성 여부 + 주제별 정리 섹션**을 하나의
  ` ```ragrelevance ` fenced JSON(`{relevant, reason, sections:[{heading,points,sources}]}`)으로 낸다.
  `parseRagRelevance`가 검증(`parsePrefill` 스타일)하며, 정리된 뷰 HTML은 `foundation.ts::ragCuratedHtml`
  (기존 `ragResultHtml`과 `<style>`/`escapeHtml` 공유; 섹션 비면 원본 hits로 폴백)이 만든다.
- **결정(게이트)**: `ChatPanel::stepPreflight`의 `rag` 분기에서 —
  - `verdict.relevant === false` → 기존 **skip 경로 재사용**(패널·주입 없이 스킵 노트 + 워크플로우 계속 진행).
  - `verdict.relevant === true` → `onRagResult(query, hits, verdict)`로 **정리된 패널** 표시 + (기존대로)
    `formatRagContext(hits)` **원본 발췌 주입**(게이트는 표시/스킵만 결정; 관련 판정 시 에이전트에는 전체
    발췌를 그대로 줘 정보 충실도 유지).
  - `verdict === null`(파싱/에이전트 실패) → **fail-open**: 원본 패널(`ragResultHtml`) 표시 + 원본 주입
    (오늘 동작 그대로 — 정보를 잘못 숨기지 않음).
- **취소**: 판단 턴의 run id는 `runIdRef`에 없으므로 `ragJudgeCancelRef`를 두고 `stop()`이 이를 호출한다.
  취소 시 `judge.promise`가 null로 resolve되고 `preflightAbortRef`가 set되어 **기존 preflight abort 경로**
  (halt → 일반 대화)를 탄다. `fetchNote`는 "사내 문서 검색·관련성 확인 중…"으로 검색+판단을 포괄.
- **범위**: 대상은 `rag` 단계와 '검색 결과' 탭. **지식 베이스(`knowledge`) 단계는 범위 밖**(사용자 요구는
  "rag와 confluence"). 백엔드(`rag.rs`/`settings.rs`) 무변경 — rankScore 플럼빙 불필요.
- **대안 기각**: **점수 임계값**(RAG `rankScore`를 프론트로 플럼빙해 컷오프) — rankScore 스케일/의미가
  불확실(샘플 0.0001)하고 "작업과의 관련성"을 총체적으로 못 판단 / **요약 답변 휴리스틱**(RAG 요약의
  "관련 없음" 문구 감지) — 문구 의존적이라 취약 / 표시만 게이팅하고 주입은 유지 — 무관 정보가 프롬프트를
  오염 / 판단을 본 턴에 융합 — 패널이 본 턴 전 preflight에 뜨므로 사후 판단으론 숨길 수 없음.
- **하위호환/한계**: 프론트 전용 타입(`RagVerdict`, serde 미러 아님). plain(opencode/antigravity)·세션리스
  (gemini/aipro)가 fenced 형식을 안 지키면 `null` → fail-open(오늘 동작, D34/D40 degrade와 일관). 지연은
  RAG 검색(최대 120s) + 판단 턴 1회이며 Stop으로 취소 가능(`fetchNote` 안내).

---

### D71. AI Pro = 두 번째 원격 에이전트 — 로컬 gemini CLI에서 OpenAI 호환 HTTP로 전환 (D17/D23/D52 갱신)
- **배경**: `aipro`(AI Pro, 사내)는 D17에서 **gemini 호환 로컬 CLI**(`aipro` 바이너리 spawn →
  `--output-format stream-json` 파싱)로 등록됐다. 하지만 이 방식은 (a) `aipro` CLI 설치에 의존하고,
  (b) 사내 백엔드의 간헐적 45초 first-byte 타임아웃 증폭(D68), (c) "Model not found"(D23), (d) 세션 시작
  콘솔 깜빡임(D54)의 원인이었다. 실제 백엔드는 **OpenAI 호환 HTTP 서비스**
  (`https://aipro.sdsdev.co.kr/open/api/v1`)로, opencode CLI는 이 엔드포인트를 `@ai-sdk/openai-compatible`
  프로바이더로 붙인다(`opencode.json` `provider.aipro`). 사용자 요구 — 이 HTTP 방식을 차용해 aipro
  연결을 개선한다.
- **핵심 판단**: 이미 검증된 **Fabrix 원격 패턴**(D64, `kind: Remote` → `fabrix.rs`)이 그대로 템플릿이다.
  프로토콜만 OpenAI로 바꿔 복제하면 **신규 crate 0**(reqwest blocking+native-tls+`.no_proxy()` 재사용,
  SSE hand-roll). 프론트 런타임(ChatPanel)은 원격/세션 동작을 전부 **`source==="remote"` 문자열**과 모델
  목록으로 구동하므로(id 무관), aipro가 `source:"remote"`를 보고하면 **문서 파일 저장(D67)·세션리스
  transcript·모델 스냅이 ChatPanel 변경 0으로 자동 적용**된다.
- **결정(교체)**: `aipro` def를 **그 자리에서** `kind: Local`→`kind: Remote`로 전환(CLI 필드 비움,
  `aipro_build_args` 삭제, `run: None`). id/슬롯 유지(`AGENT_DEFS` 7종 불변). `fabrix`와 달리
  **`fallback_models`(glm-5.1/qwen3.6-27b/gpt-oss-120b)를 유지**해 `detect_aipro`가 `/models` 도달 불가+
  캐시 없음일 때 정적 폴백으로 쓴다(합성 `default` 없음 — 채팅은 실제 id 필요).
- **결정(모듈)**: 새 모듈 **`aipro.rs`**(fabrix.rs 미러) — `detect_aipro`(캐시 우선 D66),
  `fetch_models`(`GET /models`), 순수 파서 `parse_openai_models_json`(`data[].id`)·`parse_openai_sse_data`
  (`choices[0].delta.content`→TextDelta, `usage`→Usage, `error`→Error, `[DONE]` 종료),
  `run_aipro`(`POST /chat/completions` + SSE), `probe_aipro`. 인증 `Authorization: Bearer <apiKey>`,
  `chat_body`는 `{model, messages:[system,user], stream, stream_options.include_usage, temperature, max_tokens:8192}`.
- **결정(디스패치 일반화)**: 원격이 2종이 됐으므로 `detect_agent`(lib.rs)·`run_agent`(run.rs)의 단일
  `kind==Remote→fabrix` 분기를 **`match def.id`**(`fabrix`/`aipro`/`_`→에러)로 일반화. 트레이트/enum
  추상화는 config 타입이 달라(FabrixConfig vs AiProConfig) 2-암 match보다 과함.
- **결정(설정·UI)**: `settings.json`에 **`AiProConfig`**(endpointUrl+apiKey+allowInvalidCerts+models 캐시)
  추가(D64 단일 설정 루트 원칙 유지 — opencode.json/auth.json을 직접 읽지 않음). 커맨드
  `set_aipro_config`/`probe_aipro`. UI는 Agents 화면의 전용 **`AiProCard`**(endpoint는 알려진 상수로
  프리필·편집 가능 + 단일 API 키 password 필드 + 연결 테스트; `AgentsView`가 `id=="aipro"` 분기).
  `DIAGNOSTIC_HINT`는 원격 카드가 공유하므로 **에이전트 중립 문구**로 완화.
- **모델 목록**: 라이브 `GET /models` + 캐시(D66) — 저장/새로고침/연결 테스트에서 조회해 `aipro.models`에
  저장, 이후 캐시 우선. 실패 시 정적 폴백(3종). **⚠️ D73/D74에서 개정**: 모델을 이미 정적으로 알고 있어
  `/models` 의존을 제거(모델 목록=정적 카탈로그, 연결 테스트=최소 `POST /chat/completions`; D73). 실제
  500의 원인은 `/models`가 아니라 **누락된 `User-Agent`**였고 `opencode/<ver>` UA 부착으로 해결됐다(D74).
- **대안 기각**: **CLI 유지 + 별도 API 에이전트 추가**(에이전트 8종, 두 경로 중복 — 사용자가 "교체" 선택) /
  **opencode.json/auth.json 직접 읽기**(설정 루트 이원화·외부 파일 의존 — 사용자가 앱 settings.json 선택) /
  **원격 trait/enum 추상화**(2종에는 과설계) / **CLI 없을 때만 HTTP 폴백**(비결정적, 복잡) /
  **endpoint를 detect/settings에서 자동 시드**(저장 전 available 오탐 — 카드 프리필만).
- **하위호환**: `AiProConfig`는 `#[serde(default)]`라 구 settings.json 무변경 로드(`aipro` 없으면 `None`).
  D17/D23/D52는 aipro의 *과거* CLL 전송을 기술하는 역사 기록으로 유지(D71 역참조). **D68(일시적 타임아웃
  자동 재시도)은 HTTP 전환 후에도 유효**(원격도 first-byte 지연 가능 — 무해).
- **한계/재검토**: 원격이라 **코드베이스 실독·도구 호출 불가**(Fabrix와 동일 — D67 한계; codebase 단계
  산출물은 추정 기반일 수 있음). `.no_proxy()`는 aipro가 사내 직접 도달 전제(프록시 경유 필요 시 aipro
  클라이언트만 해제). `max_tokens`를 거부하는 백엔드가 있으면 `max_completion_tokens`로 조정. 개편된
  기본 연결은 사용자가 Agents 카드에서 저장해야 활성(저장 전 `not-configured`).

---

### D72. 모든 앱 데이터를 `~/.operation-wizard/`로 통일 — settings를 `app_config_dir`에서 이전 + 공유 `ow_home()`
- **배경**: 앱이 디스크에 쓰는 런타임 파일이 **두 루트로 갈라져** 있었다 — `settings.json`(+
  `settings.json.corrupt`)만 Tauri `app.path().app_config_dir()`(Windows `%APPDATA%\com.shi.operationwizard\`)에,
  나머지 전부(`projects/`, `knowledge/`·`knowledge/artifacts/`, `startup-error.log`)는 이미
  `~/.operation-wizard/`(`%USERPROFILE%\.operation-wizard\`)에 있었다. 사용자 요구 — **모든 설정/데이터
  파일을 홈 폴더 `~/.operation-wizard/` 하위에 쓰게** 통일(한 곳에서 찾고 백업).
- **핵심 판단**: `settings::load(config_dir)`/`save(config_dir, s)`는 루트를 **인자로 받고**(스스로 정하지
  않음), 18개 호출부가 전부 `app_config_dir()`를 넘겼다. 반면 `projects.rs`/`knowledge.rs` 커맨드는
  **`app`을 받지 않고** `projects_root()`/`knowledge_root()`로 env에서 루트를 구한다. 이 패턴을 settings에
  적용하면 된다 — 즉 실제로 옮길 파일은 `settings.json` 하나뿐.
- **결정(공유 헬퍼)**: `~/.operation-wizard`를 **한 곳에서** 정의하는 `crate::ow_home() -> Result<PathBuf,
  String>`(USERPROFILE→HOME)를 `lib.rs`에 추가하고, 흩어져 있던 3개 리졸버(`startup_log_path`·
  `projects_root`·`knowledge_root`)를 이 헬퍼로 통합(중복 제거·정의 단일화). `startup_log_path`는 홈
  해석 실패 시 temp 폴백을 유지.
- **결정(설정 저장 루트 전환)**: 18개 `app.path().app_config_dir()` 호출을 `crate::ow_home()?`로 교체.
  설정 용도로만 `app`을 받던 **14개 커맨드에서 `app` 파라미터 제거**(`get_settings`·`set_agent_bin`·
  `set_skills`·`set_workflow`·`set_*_config`·`detect_agent`·`probe_fabrix`·`probe_aipro`·`rag_search`·
  `probe_rag`·`probe_confluence`) — projects/knowledge 커맨드와 동일 형태가 됨(프론트는 `app`을 넘긴 적이
  없어 무영향; `.path()` 전용 `use tauri::Manager`도 정리). `app.state()`가 필요한 넷(`run_agent`·
  `run_fabrix`·`run_aipro`·`start_confluence_ingest`)은 `app` 유지, 그 한 줄만 교체.
- **결정(자동 이전)**: `settings::load`가 새 위치에 파일이 없고 레거시 `%APPDATA%\com.shi.operationwizard\
  settings.json`이 있으면 **1회 비파괴 복사**(`migrate_legacy_settings` — 원본 유지, dest 있으면 no-op).
  env `APPDATA` 조회(`legacy_appdata_settings`)는 `#[cfg(not(test))]`로 게이트해 테스트 격리; 순수 헬퍼는
  명시 경로로 단위 테스트. 사용자 확정(기존 연결·워크플로우 유지).
- **불변**: `settings::load`/`save`/`backup_corrupt` 시그니처는 그대로(기존 테스트·`settings.json.corrupt`
  백업이 새 홈 루트로 자동 따라감). `tauri.conf.json`의 식별자(`com.shi.operationwizard`)는 **미변경**
  (창·기타 Tauri 기능이 사용) — 단지 settings 저장에 `app_config_dir`를 쓰지 않을 뿐. 프론트/타입/의존성 0.
- **대안 기각**: **clean switch**(기존 %APPDATA% 설정 폐기 — 사용자가 자동 이전 선택) / **식별자 변경으로
  app_config_dir 이동**(창·업데이터 등 광범위 영향) / **`tauri-plugin-fs` base dir 설정**(미사용 플러그인
  도입) / **settings.rs가 루트를 스스로 정하도록 `load()`/`save()` 무인자화**(테스트가 루트 주입에
  의존 — projects/knowledge식 "커맨드가 루트 해석" 패턴이 더 일관적).
- **한계**: 마이그레이션은 레거시 식별자를 상수로 하드코딩(1회성·경로 동결). 홈(USERPROFILE/HOME) 미해석
  환경에선 설정 커맨드가 에러(기존 `app_config_dir` 실패와 동일 의미). `<workdir>/<문서>` 산출물은
  caller-supplied라 무관(기본 workdir가 이미 `~/.operation-wizard/projects/<id>/workspace/`).

---

### D73. AI Pro 연결 테스트/탐지의 `/models` 의존 제거 — 최소 채팅 프로브 + 정적 카탈로그 (D71 개정)
> ⚠️ **원인 정정(D74)**: 이 결정은 500의 원인을 "`/models` 엔드포인트가 깨졌다"로 추정했으나, 이후
> 라이브 진단으로 **진짜 원인은 누락된 `User-Agent`**임이 밝혀졌다(게이트웨이가 opencode UA를 요구하고
> 백엔드가 `ua.split("/")`). `/models`도 UA만 있으면 동작한다. 단, **아래 설계 결정(연결 테스트=최소 채팅
> 프로브 + 모델=정적 카탈로그 + cert-skip 제거)은 그대로 유효**하다(모델 3종을 이미 알고 있어 `/models`가
> 불필요하고, 채팅 프로브가 "실제 대화 가능"을 더 정확히 검증). 실제 수정은 **D74** 참조.

- **배경**: D71에서 AI Pro 연결 테스트(`probe_aipro`)와 탐지(`detect_aipro`)는 OpenAI 표준
  **`GET {endpoint}/models`**로 모델 목록을 조회했다. 실사용에서 정상 API 키로도 연결 테스트가
  **HTTP 500**으로 실패했다: `{"detail": "'NoneType' object has no attribute 'split'"}`.
- **원인(당시 추정 — D74에서 정정)**: 서버(Python) 측 크래시(500 + 파이썬 예외 `detail`)이고 우리 인증/키
  문제가 아니다(401/403 아님)는 판단은 맞았다. 다만 "`/models` 고유 결함"이라는 추정은 틀렸다 — 실제로는
  **모든 요청**이 UA 누락으로 500이었다(D74). AI Pro 모델 3종(glm-5.1/gpt-oss-120b/qwen3.6-27b)은 이미
  정적 카탈로그로 알고 있어 라이브 조회가 불필요하다(이 부분은 유효).
- **결정**:
  - **연결 테스트(`probe_aipro`)**: `GET /models` → **최소 비스트림 `POST /chat/completions`**
    (`chat_probe` — model=`glm-5.1`, `messages:[{user:"ping"}]`, `max_tokens:1`, `stream:false`,
    헤더 `Authorization: Bearer`+`Accept: application/json`+`Content-Type`(via `.json()`)). HTTP 2xx면
    `"연결됨 — AI Pro 응답 정상"`. opencode의 검증된 경로와 동일해 "정말 대화가 되는지"를 진짜로 확인한다.
  - **탐지(`detect_aipro`)**: **네트워크 호출 제거** — 설정(endpoint 존재) 시 `available=true` +
    **정적 카탈로그**(`models_source="fallback"`), 미설정 시 `not-configured`. `/models`를 호출하면 카드가
    "unreachable"로 오표시되고, 매 탐지(앱 시작/새로고침)마다 채팅 토큰을 쓸 순 없어 탐지 시 도달성은
    확인하지 않는다(명시적 연결 테스트·실제 대화가 검증). `force` 인자는 미사용(fabrix와 공통 시그니처 유지).
  - **죽은 코드 제거**: `aipro.rs`의 `fetch_models`·`parse_openai_models_json`(+단위 테스트) 삭제.
    `parse_openai_sse_data`·`run_aipro`·`static_fallback_models`는 유지. `AiProConfig.models` 캐시 필드는
    aipro에서 실질 미사용이나 필드는 유지(serde/마이그레이션 무변경).
  - **UI**: AI Pro 카드의 **"인증서 검증 건너뛰기" 체크박스 제거**(사용자 요청). 500은 TLS가 아니라 앱
    오류라 이 옵션과 무관. `allowInvalidCerts`는 항상 `false`로 저장(백엔드 필드는 호환 유지).
- **대안 기각**: **`/models`에 Content-Type/Accept 헤더 추가로 재시도**(원인이 헤더 누락일 때만 해결 —
  불확실, `/models`는 비필수) / **`x-generative-ai-user-email` 헤더 추가**(opencode·RAG 모두 미전송 —
  현 증거상 불필요; 채팅 경로에서도 500이면 그때 후속 추가) / **라이브 모델 목록 유지**(깨진 엔드포인트
  의존).
- **한계/재검토**: 탐지가 configured→available이라 카드가 다소 낙관적으로 "Detected"를 표시(도달성은 연결
  테스트·실제 대화가 진짜 검증 — 저렴한 헬스 엔드포인트 부재로 감수).

---

### D74. AI Pro 500의 진짜 원인 = 누락된 `User-Agent` — 요청에 `opencode/<ver>` UA 부착 (D73 정정)
- **배경**: D73 이후에도 AI Pro 연결 테스트(이제 `POST /chat/completions`)가 정상 키로 **여전히 HTTP 500**
  `{"detail": "'NoneType' object has no attribute 'split'"}`을 냈다. 앱을 배제하고 라이브 엔드포인트에
  직접 curl로 요청을 변주하며 원인을 이분 탐색했다.
- **진단(라이브 curl로 확정)**:
  - 무인증 → 401. Bearer 인증 → 통과(401 아님) 후 **모든 authed 경로**(`/chat/completions`·`/models`·`/`·
    없는 경로, 빈 body 포함)가 동일하게 500 `NoneType.split`. → 라우트/바디가 아니라 **요청 공통 요소**.
  - 이메일 헤더(`x-generative-ai-user-email` 등 3종)·Fabrix 헤더(`x-fabrix-client`/`x-openapi-token`) 추가 →
    변화 없음(500).
  - **`User-Agent`가 결정적**이었다: UA를 붙이면(`curl/8.x`·`reqwest/…`·`node`·`axios`·`aipro-cli`·`Mozilla`)
    게이트웨이가 **406 Not Acceptable**로 차단, UA를 빼면 백엔드에 도달하나 **500**(백엔드가 `ua.split("/")`를
    하는데 UA가 None). **`User-Agent: opencode`/`opencode/<ver>`만** 게이트웨이 allowlist를 통과하고 백엔드
    파싱도 성공해 **HTTP 200 + 정상 completion**(비스트림·스트림 모두 라이브 검증).
  - 근거: AI Pro CLI(`@aipro/aipro-cli`)는 **opencode 기반**(rg 번들·MCP·skills·hooks 등 동일)이라 공식
    클라이언트가 `opencode/<ver>` UA를 보낸다. `reqwest`는 기본 UA를 안 보내서 앱이 500을 맞았다.
- **결정**: `aipro.rs`의 공유 `build_client`에 **`.user_agent("opencode/0.1.0")`**(상수 `OPENCODE_UA`)를 붙여
  **모든 AI Pro 요청**(`chat_probe`·`run_aipro`)에 적용한다. 한 줄로 연결 테스트·실제 대화가 모두 동작.
  범위는 aipro만(Fabrix/RAG는 별도 엔드포인트·헤더 — 무관).
- **부수 개선**: 스트리밍 응답이 `delta.reasoning`(glm-5.1 추론 토큰)을 `delta.content` 앞에 보내므로
  `parse_openai_sse_data`가 `reasoning`(및 `reasoning_content`)를 **`ThinkingDelta`**로 매핑(무응답 구간
  체감 감소). 기존 `content`→`TextDelta`는 그대로.
- **D73과의 관계**: D73의 설계(연결 테스트=최소 채팅 프로브, 모델=정적 카탈로그, cert-skip 제거)는 유지.
  D74는 그 위에 **실제 동작을 가능케 한 근본 수정**이다. `/models`는 UA만 있으면 동작하지만 여전히 미사용
  (모델을 정적으로 알고 있음 — D73).
- **대안 기각**: UA 없이 우회(백엔드가 UA를 split해 500 — 불가) / 실제 브라우저/툴 UA(게이트웨이 406) /
  이메일·Fabrix 헤더(무효, 라이브 확인) / `.user_agent`를 전 원격 모듈에 일괄 적용(Fabrix/RAG는 무관 —
  aipro 스코프 유지).
- **한계/재검토**: UA 값은 `opencode/0.1.0` 상수(라이브 검증). 게이트웨이가 향후 opencode **최소 버전**을
  요구하면 상수만 상향. `reqwest`가 UA를 안 보낸다는 사실에 의존(향후 기본값이 생겨도 `.user_agent`가 우선).

---

### D75. Confluence 수집 403의 원인 = 누락된 `User-Agent` — 브라우저 UA 부착 + 오류 본문 노출 + TLS-skip 체크박스 제거 (D66/D73/D74 연장)
> ⚠️ **대체(D82)**: REST 크롤(`HttpConfluence`)과 `CONFLUENCE_UA`는 **D82로 제거**되었다(Confluence를 공식 MCP
> 서버로 전환). UA가 사내 `sdsdev.co.kr` 게이트웨이 통과의 열쇠라는 교훈은 유효하며, MCP 클라이언트도
> `User-Agent: opencode/*`를 부착한다(D82). TLS 검증 상시 on 방침도 유지. 아래는 최초 결정.
- **배경**: 지식 화면의 Confluence 수집이 정상 PAT로도 `HTTP 403 Forbidden — {baseUrl}/rest/api/content/<id>?expand=body.storage`로 실패했다(대상: 사내 `https://devops.sdsdev.co.kr/confluence`, Server/DC).
- **원인**: `confluence.rs`의 `HttpConfluence::new` reqwest 클라이언트가 `.no_proxy()`(D66)·`.timeout()`·`.danger_accept_invalid_certs()`는 설정하지만 **`.user_agent(...)`가 없었다**. `reqwest`는 기본 UA를 보내지 않고, 사내 게이트웨이/WAF는 **UA 없는 요청을 403으로 차단**한다 — AI Pro가 UA 부착 전까지 실패하던 것과 **같은 클래스**(D74; 두 호스트 모두 `sdsdev.co.kr`). Bearer PAT 인증(`req.bearer_auth`)과 URL은 이미 정상이라 인증 스킴은 원인이 아니다. 형제 모듈 중 `aipro.rs`만 UA를 부착했고 `fabrix.rs`/`rag.rs`/`confluence.rs`는 미부착이었다(confluence만 이번에 부착 — 나머지는 무관해 스코프 유지, D74 관례).
- **결정(수정)**:
  - **브라우저형 UA 상수 `CONFLUENCE_UA`**(Chrome 계열 `Mozilla/5.0 …`)를 추가해 클라이언트 빌더에 `.user_agent(CONFLUENCE_UA)`로 부착한다. **증상이 403**(aipro의 500/406과 다름)이라 표준 Confluence+WAF는 일반 브라우저 UA를 원한다고 판단 — aipro의 커스텀 게이트웨이가 요구한 `opencode/*`가 아니라 브라우저 UA를 선택(사용자 확정). `.no_proxy()`는 유지(프록시 우회 — 사용자 요구 + D66).
  - **오류 본문 노출**: `HttpConfluence::get`이 비2xx 시 응답 본문 스니펫(~300자, 공백 정규화)을 오류 메시지에 덧붙인다 — 잔여 403이 **WAF 차단 HTML**(게이트웨이/UA 문제)인지 **Confluence 권한 오류 JSON**(PAT 열람 권한 부족 — 코드 이슈 아님)인지 수집 진행/오류 UI에서 바로 구분 가능.
  - **방어적 헤더/토큰**: 요청에 `Accept: application/json`을 부착하고, `bearer_auth`에 `token.trim()`을 넘긴다(프론트가 저장 시 이미 trim하지만 다른 저장 경로 방어).
  - **TLS 검증 생략 체크박스 제거**: 지식 뷰 `ConfluenceSection`에서 `allowInvalidCerts` 체크박스·상태를 제거하고 저장 시 항상 `allowInvalidCerts: false`를 보낸다(인증서 검증 상시 on — 사용자 요구 "TLS 생략 불필요"). 백엔드 `ConfluenceConfig.allow_invalid_certs`(`#[serde(default)]`) 필드는 serde 호환을 위해 유지(항상 false) — **AI Pro D73의 cert-skip 제거와 동형**. 크롤과 연결 테스트(`probe_confluence`)는 같은 `HttpConfluence::new`/`get`을 타므로 한 곳 수정으로 둘 다 고쳐진다.
- **대안 기각**: `opencode/0.1.0` UA(aipro 게이트웨이 값 — devops가 같은 게이트웨이면 필요할 수 있으나 403 증상은 표준 WAF 쪽이라 브라우저 UA 우선; 잔여 실패 시 오류 본문으로 확인 후 전환) / UA 미부착·프록시 경유(사내 직접 도달 전제 — D66) / `X-Atlassian-Token: no-check`(변경성 POST용, GET엔 불필요) / TLS-skip 유지(사용자가 불필요로 확정) / UA를 전 원격 모듈 일괄 적용(fabrix/rag는 무관 — confluence 스코프 유지).
- **하위호환/영향**: 신규 Cargo/npm 의존성 0. `ConfluenceConfig` serde·타입 미러 무변경(프론트가 `allowInvalidCerts: false`를 계속 전송). 기존 크롤 파서/`FakeApi` 단위 테스트는 `HttpConfluence`를 우회하므로 무영향.
- **한계/재검토**: UA가 브라우저형이라 만약 devops가 aipro와 **동일 커스텀 게이트웨이**면 406이 날 수 있다(그땐 오류 본문이 드러내며 `opencode/0.1.0`로 상수만 교체). 게이트웨이가 향후 특정 UA를 요구하면 상수만 상향. 403 본문이 Confluence 권한 오류면 PAT 사용자 열람 권한 문제(코드 밖).

---

### D76. 미리보기 HTML/마크다운의 외부 링크를 OS 브라우저로 — 링크 가드 (D42/D46/D69 연장)
- **배경**: 운영 가이드(`guide`)는 결과를 자립형 HTML(`docs/operation-guide.html`)로 만들어 캔버스
  `FileViewer`의 **샌드박스 iframe**(`sandbox="allow-scripts"`)에 미리보기로 띄운다. 이 HTML의 "참고
  문서" 섹션 링크(`<a href="http://…">`)를 클릭하면 **뷰가 그 URL로 이동해 앱이 브라우저가 되고 원래
  화면으로 못 돌아갔다**. 어디에도 링크 클릭을 가로채 OS 브라우저로 넘기는 코드가 없었고, 외부 열기
  수단은 `openPath`(탐색기 — D69)뿐 `openUrl`은 미사용이었다.
- **심각도 분석(두 프레임)**: ① **최상위 프레임의 마크다운 링크**(가장 치명적) — 채팅 응답
  (`AssistantMessage`)과 `.md` 미리보기(`FileViewer`)의 링크는 React DOM의 평범한 `<a href>`라 클릭 시
  **앱 전체가 교체**된다(복구 불가). ② **샌드박스 iframe HTML**(운영 가이드 `.html` + RAG '검색 결과'
  패널 — D46) — `allow-top-navigation`이 없어 최상위는 못 바꾸지만 링크가 **iframe 자신(캔버스 pane)** 을
  외부 사이트로 이동시켜 되돌아갈 수 없다. 사용자 요구는 두 표면 모두 수정.
- **결정(2계층 프론트 링크 가드)**: 새 순수 모듈 `src/lib/linkGuard.ts` — `isExternalUrl`(http/https/
  mailto만), `LINK_GUARD_SCRIPT`(주입 스크립트 문자열), `withLinkGuard(html)`(가장 이른 안전 지점에
  `<script>` 삽입), `OW_OPEN_URL` 상수.
  - **iframe**: parent가 불투명 origin iframe DOM에 리스너를 못 붙이므로, srcdoc에 **캡처단계 클릭/Enter
    가드 스크립트**를 주입한다 — 외부 URL이면 `preventDefault()` 후 `window.parent.postMessage({type:
    'ow-open-url', url}, '*')`. `App.tsx`의 단일 `message` 리스너가 메시지 shape+scheme를 재검증(불투명
    origin이라 `event.origin==="null"` → origin 비교 불가)하고 `openExternal(url)`(= opener `openUrl`)로
    OS 브라우저를 연다. `FileViewer::buildSrcdoc`(완결/조각 두 분기 모두)과 `CanvasPanel`의 RAG iframe
    (`useMemo`)이 `withLinkGuard`를 통과한다. `foundation.ts`(`target="_blank"` 링크)는 무변경 — 가드가
    클릭을 먼저 가로챈다.
  - **최상위 마크다운**: `Markdown.tsx`의 `components`에 `a` 오버라이드 추가 — **모든 클릭 `preventDefault`**
    (최상위 이동은 항상 치명적) 후 외부 URL만 `openExternal`. `AssistantMessage`·`.md` 미리보기 모두
    `MarkdownView` 경유라 한 번에 커버(스트리밍 중 채팅은 평문이라 앵커 없음; `rehype-raw` 미설정).
- **권한**: `openUrl`은 신규 grant 불필요 — `opener:default`가 이미 `allow-open-url`을 포함한다(D69의
  `allow-open-path`는 default에 없어 따로 추가했던 것과 대조). capability 무변경.
- **정책 분리**: iframe은 **최소 간섭**(`#앵커`·상대·`javascript:`는 그대로 둬 문서 내부 동작/토글 보존),
  최상위 마크다운은 **전 클릭 차단**(보존할 in-page 앵커가 없고 — 목차는 index 기반 `jumpTo` — 이동은 곧
  손실). `javascript:`/`data:`/`blob:`은 어느 경로에서도 외부로 열리지 않는다(parent가 http/https/mailto만
  허용).
- **대안 기각**: Rust `on_navigation`(wry 네비게이션 핸들러) — **최상위 프레임만** 잡고 iframe 자기 이동은
  못 잡아 보고된 버그를 못 고친다. 게다가 창이 `tauri.conf.json`로 선언 생성되어 핸들러를 붙이려면
  `WebviewWindowBuilder`로 창 생성을 `setup()`으로 옮기는 큰 리팩터가 필요 — 프론트 가드가 두 프레임을
  모두 커버하므로 채택하지 않고 **향후 CSP-내성 backstop**으로만 남긴다. / `foundation.ts` 링크 수정 —
  가드가 클릭을 가로채므로 불필요. / capability에 `opener:allow-open-url` 추가 — 이미 default에 포함이라 무의미.
- **하위호환/영향**: 신규 Cargo/npm 의존성 0(전부 프론트). `FileViewer` "본문 복사"(D62)는 원본 `content`를
  파싱하므로 주입 `<script>`가 복사물에 섞이지 않는다(무변경).
- **한계/재검토**: 모델 생성 HTML이 `<head>`에 CSP `<meta>`를 넣으면 인라인 가드가 막힐 수 있다(그래서
  최대한 앞에 주입 — 필요 시 Rust backstop으로 승격). iframe 내 상대경로 링크는 앱 자산으로 이동하나
  운영 가이드엔 사실상 없음(최소 간섭으로 미간섭 — 잔여 리스크로 기록).

---

### D77. Confluence 수집 = 내장 WebView 내부 fetch로 WAF 우회 (스파이크; fetch/미리보기 MVP — D48/D65/D67 연장)
> ⚠️ **대체(D82)**: 이 WebView 스파이크(`confluence_open_login`/`confluence_fetch_page`/`WebviewBridge`/
> `INIT_SCRIPT`/`on_navigation` sentinel)는 **D82에서 전면 제거**되었다 — 공식 Confluence **MCP 서버**가
> WAF 게이팅과 무관하게 동작해 브라우저-내-fetch 우회가 불필요해졌다. 아래는 최초 결정의 기록이다.
- **배경**: Confluence REST(`/rest/api/`)가 사내 **WAF에 의해 Apache 계층에서 403**된다(응답이 Apache 기본
  403 HTML). 진단으로 확정: `.no_proxy()`+브라우저 UA(D75)에도 403, **P1(강제 직결)·P2(프록시 경유)·T3(세션
  쿠키) 모두 403** → 사내 아웃바운드 프록시가 아니라 컨플루언스 앞단 서버/WAF가 비브라우저 클라이언트를
  차단. 쿠키+브라우저 헤더 조합의 수동 검증은 사용자 환경에서 어려움. 브라우저 웹 UI는 SSO로 정상 동작.
- **결정(전략)**: 요청을 **앱 내장 WebView(실제 Chromium) 안에서 실행**해 게이팅 종류(UA/헤더/쿠키/TLS
  지문) 무관하게 통과시킨다. **Playwright는 비권장**(수백 MB 의존성·단일 exe 배포(D43)·사내망 다운로드·SSO
  자동화 취약 — 이 앱은 이미 WebView2 내장). **범위: fetch/미리보기만**(저장 보류 — `ingest_page`는 스텁 유지,
  D65/D67).
- **결정(회수 메커니즘 = `on_navigation` 인터셉트)**: Tauri 2.11의 `eval`은 **콜백 없는 fire-and-forget**이라
  반환값을 못 받고, 원격 origin에 IPC를 열면 **remote-scoped ACL이 필요 + 페이지 CSP `connect-src`가
  `ipc.localhost`를 막을 수 있으며 origin-confusion 보안 이슈**(GHSA-7gmj-67g7-phm9)가 있다. 그래서 주입
  스크립트(`INIT_SCRIPT`의 `window.__owFetch`)가 same-origin `fetch()` 후 결과를 **sentinel URL
  (`https://ow-ingest.local/r?reqId=..&d=<JSON>`)로 top-level 네비게이션**해 내보내고, `WebviewWindowBuilder`
  의 **Rust `on_navigation` 클로저가 이를 가로채(취소, `false` 반환) 페이로드를 회수**한다. `on_navigation`은
  top-level 네비게이션을 잡으므로(D76이 링크가드에서 기각한 사유 = **iframe 하위프레임** 미포착은 여기선
  무관 — 전용 창의 top-level 이동) 동작하고, **capability/ACL/CSP-connect가 전혀 필요 없다**. 데이터는
  `Url::query_pairs`로 자동 퍼센트 디코드.
- **결정(구현)**: `confluence.rs`에 `WebviewBridge`(managed state: `counter` + `pending: Mutex<HashMap<reqId,
  Sender>>`) + 커맨드 2개 — **`confluence_open_login`(sync=메인스레드 → 로그인 `WebviewWindow` 생성,
  label `confluence-login`, `WebviewUrl::External(base)`, `initialization_script`+`on_navigation`)**,
  **`confluence_fetch_page(page_id)`(async=메인스레드 밖 → `run_on_main_thread`로 `eval(__owFetch)` 디스패치
  후 `on_navigation`이 채널로 넘긴 결과를 `spawn_blocking`+`recv_timeout(90s)`로 대기)**. `WebviewFetchResult
  {ok,status,title,bodyHtml,raw}` 반환(성공 시 기존 `parse_page` 재사용, 실패 시 `raw`로 WAF 403 HTML 확인).
  순수 파서 `parse_nav_payload` 단위테스트. `lib.rs`에 state·커맨드 등록. 프론트: `api.ts` 래퍼 2개
  + 지식 뷰 `ConfluenceSection`에 "WebView로 가져오기(실험)" UI(로그인 창 열기 + 페이지 ID + 가져오기 +
  미리보기). **신규 Cargo/npm 의존성 0, capability/`tauri.conf.json` 무변경.**
- **게이트/후속(Phase 1)**: 스파이크가 1페이지 성공(200+본문)하면 `WebviewConfluence`(두 번째 `ConfluenceApi`
  impl)로 기존 `crawl` 재사용해 재귀 fetch로 확장. 저장(지식 베이스/ingest)은 별도 결정.
- **대안 기각**: 경량 쿠키 추출(reqwest+쿠키) — WAF가 TLS 지문까지 보면 실패, 검증도 어려움(그래서 게이팅
  무관한 in-WebView 채택) / 원격 IPC(`invoke`) — ACL+CSP+보안 리스크 / `eval`-콜백 — 2.11에 공개 API 없음 /
  네비게이션 대신 title/커스텀스킴 회수 — 더 취약.
- **리스크/한계**: 페이지 CSP에 (드문) `navigate-to`가 있으면 sentinel 네비게이션이 렌더러에서 막혀 회수
  실패(→ 타임아웃으로 표면화, 그때 다른 메커니즘 재검토). sentinel URL 길이 한계(대형 페이지 → 후속 청킹
  필요, 스파이크는 단일 네비게이션). WebView2 쿠키 스토어는 시스템 Edge와 분리 → 앱 창에서 별도 SSO
  로그인·세션 만료 시 재로그인. 원격 에이전트처럼 **저장측(ingest)은 미구현**(보류).

---

### D78. 요구사항 우선 필드 + 첫 작업 턴 '프롬프트 최적화' 내장 스킬 + '프롬프트' 캔버스 탭 (D36/D40/D46 확장)
- **배경**: 카테고리 가이드 플로우는 진입 시 고정 선택지 폼(캔버스 '요구사항' 탭, D36)을 먼저 보여주고 제출이
  첫 작업 턴을 발사한다. 사용자 요구 두 가지 — ① **"뭘 하고 싶은지"가 폼에서 우선**되어야 한다(기존 자유
  텍스트 질문 constraints/topic/target은 각 카탈로그 맨 뒤에 있어, 카테고리 카드로 시작하면 요구사항 자체를
  적을 곳이 없었다), ② 폼 제출 후 답변을 바탕으로 **AI가 프롬프팅 기법을 총동원한 "최적 프롬프트"를 완성해
  캔버스에 보여준다**(사용자가 쓰면서 프롬프팅 개선법을 학습하는 교육 효과). 사용자 명시 구현 방식:
  **프로그램 구조를 바꾸지 말고, 기존 스킬 주입 기능에서 모든 카테고리의 첫 턴에 스킬 하나를 더 얹는 것**.
- **핵심 판단**: 기존 인프라(옵션 프리플로우 D36, 스킬 주입 D40, 인메모리 캔버스 탭 D46, fenced-block 파서
  D30/D59)에 그대로 얹힌다 — **신규 백엔드/IPC/의존성 0**(전부 프론트).
- **결정(요구사항 필드)**: `REQUIREMENT_QUESTION`(id `userRequest`, text, required, `noPrefill`)을 **모든 카테고리
  폼 맨 앞**에 프리펜드(`optionsFor` — codebase 질문보다도 앞). 홈 프롬프트(seed)로 시작하면 그 텍스트로
  **클라이언트가 결정적으로 자동 채움**(`onPrefill({userRequest: seed})`, 수정 가능) — 에이전트 프리필 대상에서
  제외(`ClarifyQuestion.noPrefill` 신설 → `prefillInstruction`/`parsePrefill` 둘 다 스킵; 에이전트가 재서술하면
  손실). 프리필 완료 시 `handlePrefill`이 전체 교체이므로 완료 분기가 **seed를 병합**해 요구사항 값을 지키지
  않게 한다. 요구사항 답변은 **wire에서 빼지 않는다**(`formatClarifyAnswers`가 포함 — 최적 프롬프트 생성의
  핵심 입력). 답변은 `answerSubmission.requirement`로도 실어 ChatPanel이 사용자 버블로 쓰고 seed 덧붙임(‘원래
  요청’)을 생략한다(중복 제거).
- **결정(내장 스킬 = 레지스트리 밖 런타임 주입)**: `PROMPT_OPTIMIZER_SKILL`(`lib/promptCraft.ts`)은 **코드
  하드코딩·런타임 전용** — Flows 편집기/settings.json/백엔드에 노출되지 않는다. D39 전체 교체형 레지스트리라
  `DEFAULT_SKILLS`에 넣으면 override 저장 사용자에게 유실되므로, **레지스트리 밖에서 항상 주입**한다. 주입 시점은
  스텝 arming과 **독립적인 첫 실제 작업 턴**(`promptSkillPendingRef`, 대화당 1회, 프리필 턴 제외, 로드 세션 없음) —
  `skillBodies.unshift`로 wire 최상단에 얹고 `unwindSkills`에 되감기를 동봉해 preflight 중지·**스킵 재귀 send
  체인**(guide의 첫 rag가 스킵돼도 다음 생성형 턴에 재주입)·spawn 실패 세 지점을 자동 커버한다.
- **결정(펜스 계약 + '프롬프트' 탭)**: 에이전트가 응답 맨 앞에 ` ```prompt ` 펜스로 최적 프롬프트를 낸 뒤 **같은
  턴에서 실제 작업을 계속**한다(추가 턴/확인 게이트 없음). `end`에서 `parsePromptBlock`(태그 일치만, **폴백 없음** —
  나머지는 작업 산출물)이 블록을 뽑아 `stripPromptBlock`(→ `PROMPT_NOTE`)으로 채팅에서 제거하고(세션리스
  transcript 재전송 시 블록 재노출·재출력 방지; `mutateMessages` 동기 커밋 D55, `persist`보다 앞) `onPromptResult`로
  캔버스 **'프롬프트' 탭**(rag 탭 동형 — 인메모리 평문, 세션 유지, 도착 시 자동 전환 D46; 복사 버튼)에 표시한다.
  실패/취소 end는 파싱 생략(스트림 실패는 skill dedupe와 동일하게 되감지 않음 — wire가 이미 도달).
- **대안 기각**: 별도 생성 턴/확인 게이트(턴 추가·마찰 — 사용자 기각) / `DEFAULT_SKILLS` 등록(override 사용자
  미적용) / 전체 응답 폴백 파싱(작업 응답 전체를 프롬프트로 오인) / 스텝 기반 주입(guide rag 스킵 시 첫 턴을
  놓침 — 첫 "실제 에이전트 턴" 기준이 정확).
- **하위호환/한계**: 프론트 전용 타입(`noPrefill`은 serde 미러 아님). plain(opencode/antigravity)·원격
  (fabrix/aipro)·세션리스(gemini)가 펜스를 안 지키면 파싱 실패 → '프롬프트' 탭 없음, 작업은 그대로(폴백 없음이
  의도, D34/D40 degrade와 일관). 스트리밍 중에는 펜스 원문이 잠시 보이다 완료 시 `PROMPT_NOTE`로 치환(D57
  "스트리밍 중 평문" 관례). 전 턴 실패 후 D68 자동 재시도 턴은 스킬 dedupe와 같은 의미론이라 프롬프트가
  재주입되지 않을 수 있다(교육 표시라 수용).

---

### D79. Claude fallback 모델 카탈로그 최신화 — 별칭 우선 + 현행 세대 pinned (D12 콘텐츠 정책)
- **배경**: Claude Code 에이전트를 고르면 모델 드롭다운에 구세대(`claude-opus-4-5`/`claude-sonnet-4-5`/
  `claude-haiku-4-5`)만 보였다. Claude는 라이브 모델 조회 경로가 없어(`models_probe: None` — D12,
  MMS 라우트 미포팅) `detect.rs`가 항상 `agents.rs`의 **정적 fallback 카탈로그**를 반환하는데, 그
  카탈로그가 4.5세대로 하드코딩돼 있었기 때문이다(D66 모델 캐시는 원격 에이전트 전용 — claude 무관).
- **결정**: `agents.rs`의 claude `fallback_models`를 **별칭 우선 + 현행 세대 pinned**로 교체한다 —
  `opus`/`sonnet`/`haiku`(Claude Code CLI가 항상 최신 세대로 해석하므로 잘 안 낡음)를 앞에 두고, pinned
  ID를 현행(`claude-opus-4-8`/`claude-sonnet-5`/`claude-haiku-4-5`, 라벨은 친숙한 이름)으로 갱신한다.
  `&'static` 상수라 **재빌드해야 반영**된다. D12의 "claude는 fallback 전용" 판단은 그대로 두고, 이건 그
  카탈로그의 **콘텐츠 정책**이다.
- **근거**: 라이브 조회가 없어 카탈로그가 낡으면 곧바로 UI stale로 드러난다. 별칭을 앞세우면 CLI의 자동
  해석에 기대어 재빌드 없이도 최신을 쓸 수 있고, pinned는 세대 고정 선택지를 제공한다(현행 세대 유지는
  주기적 갱신 대상). 소비처(`AgentCard`/`HomeView`/`ChatPanel`)가 백엔드 배열을 그대로 렌더하므로 def
  배열만 고치면 UI에 즉시 반영된다(프론트/타입/캐시 변경 0).
- **대안 기각**: 별칭만 유지(세대 고정 pin 불가) / 라이브 조회 도입(MMS 프록시 인프라 필요 — D12 범위 밖) /
  전체 세대 명시(목록 과다). 모델 ID는 공식 레퍼런스로 확정한 현행 문자열이며, pinned는 설치 CLI/계정이
  거부할 수 있으나 fallback 표시용 정적 값이고 별칭으로 최신 사용이 보장된다.
- **한계/재검토**: 세대가 넘어가면 pinned를 다시 갱신해야 한다(별칭은 자동 추종). codex/gemini/antigravity
  카탈로그는 이번 범위 밖(요청은 claude 한정).

---

### D80. codex 샌드박스 = workspace-write (danger-full-access에서 하향) + 엔터프라이즈 정책 한계 (D24/D28 연장)
- **배경**: Codex CLI 실행이 `Error: invalid value for allowed_sandbox_modes: [WorkspaceWrite] is not
  in the allowed set must include 'read-only' to allow any PermissionProfile (set by enterprise-managed
  requirements Group requirements ...)`로 실패했다. 조사 결과 두 층위의 원인이다: ① 앱이
  `codex_build_args`(agents.rs)에서 샌드박스를 **`danger-full-access`로 하드코딩**(create `--sandbox`,
  resume `-c sandbox_mode`)해 넘겼고, ② 사용자 머신의 **엔터프라이즈 관리 codex 정책**(cloud "Group
  requirements")이 `allowed_sandbox_modes`를 `["workspace-write"]`로 두되 **`read-only`를 빠뜨려**, 최신
  codex가 이를 `Constrained<PermissionProfile>`로 변환할 때 config 로드 단계에서 하드 에러로 거부한다
  (codex-rs `config/src/config_requirements.rs` — read-only는 모든 PermissionProfile의 floor라 필수).
- **결정**: 앱의 codex 샌드박스를 `danger-full-access` → **`workspace-write`**로 낮춘다(create/resume
  두 분기). 문서 생성은 cwd(워크스페이스 루트) 쓰기만 필요하므로 workspace-write면 충분하고, 이는 사내
  정책의 허용 상한과 호환된다. 아울러 이 부류 오류를 `workspace.ts::errorHint`가 인식해 **한글 안내 +
  대체 에이전트 유도**를 표시하도록 한다(D28 TLS 케이스 관례). 샌드박스 모드의 **설정 노출·자동 폴백은
  미도입**(사용자 선택: 최소 교체 — RunArgs/RunCtx/settings/UI 무변경).
- **정책은 앱이 못 고친다(핵심)**: requirements는 **하드 실링**이라 클라이언트가 `-c`/`--sandbox`로 상한을
  넓히거나 `allowed_sandbox_modes`를 덮어쓸 수 없다. ②의 config-로드 오류는 **어떤 샌드박스 모드로도**
  동일하게 나므로(앱뿐 아니라 수동 `codex exec`도 실패), **표시된 오류의 실제 해소는 IT/관리자가 정책의
  `allowed_sandbox_modes`에 `read-only`를 포함(예: `["read-only","workspace-write"]`)하거나
  `allowed_permission_profiles` 체계로 이관**하는 것이다(D28과 동류의 환경 제약 — 앱은 안내·회복만 제공).
  앱의 workspace-write 전환은 정책 정정 후 정상 동작을 위한 정합성 수정이자, danger-full-access 과요구
  제거다(정책 정정 후에도 danger는 `DangerFullAccess is not in the allowed set`로 거부됨 — codex #18242).
- **대안 기각**: 설정 노출(단일 조직 환경엔 과설계) / 정책 실패 시 자동 폴백(config-로드 단계 하드 에러는
  어떤 모드로도 실패해 무효) / config.toml·env 주입으로 우회(관리 계층이 우선순위가 높아 불가).
- **한계/재검토**: workspace-write는 기본 네트워크 차단(문서 생성엔 무관)이고, 코드베이스 분석(D45)에서
  cwd 밖 코드베이스 폴더 읽기는 대체로 허용되나 Windows codex 샌드박스 동작에 버전차가 있어 E2E 확인
  필요 — 읽기가 막히면 `sandbox_workspace_write` 읽기 범위/`writable_roots` 조정을 후속 검토(범위 밖).

---

### D81. 홈 컴포저 프롬프트 → 카테고리 자동 분류 (WorkspaceView 격리 분류 턴 + plan 폴백)
- **배경**: Home 컴포저에 프롬프트를 입력해 시작하면 `HomeView.send()`가 `start("plan")`으로 카테고리를
  **무조건 `plan`(개발 계획 수립)** 으로 하드코딩해, 어떤 요청이든 개발 계획 워크플로우로 진입했다(카테고리
  카드 클릭만 각 id를 넘김). 사용자 요구 — **처음 프롬프트를 근거로 4개 카테고리(`plan`/`guide`/`query`/
  `change`) 중 가장 어울리는 것으로 자동 라우팅**한다. 카드 클릭은 명시적 선택이라 분류하지 않고, 불명확·
  실패 시 `plan`으로 폴백(기존 동작 유지).
- **핵심 제약**: ① 카테고리는 `ChatPanel` **마운트 시 고정(freeze)** 되는 refs(`WF`/`optionQuestions`/
  `skillMapRef`/`stepProgress` 초기값)를 구동하므로 카테고리를 바꾸려면 그 카테고리로 **마운트**해야 한다.
  ② 분류 격리 턴은 **유효한 `cwd`가 필수**다(`run.rs`가 빈 cwd 거부 + `current_dir`). 자동 프로젝트 workdir는
  `ensureProject`가 만들어야 존재하므로 **Home 화면에서는 cwd가 없다** → 워크스페이스 진입 후 분류한다.
- **결정(Option B — WorkspaceView가 분류, ChatPanel은 최종 카테고리로 1회 마운트)**: `ChatPanel`의 가장
  취약한 세 지점(boot effect/`send()`/`stop()`)을 **건드리지 않는다.** `WorkspaceView`가 진입 시
  (a) `ensureProject`로 cwd 확정(+`setResolvedWorkdir`) → (b) 새 모듈 `lib/categorize.ts`의
  `classifyCategory`(격리 에이전트 턴, `judgeRagRelevance`(D70)/`generateKnowledgeSummary`(D59) 패턴을 그대로
  복제 — 세션 없이 1회 실행, `` ```category `` fenced JSON 파싱, 실패 시 `null`) 실행 → (c) `activeCategory`
  확정 후 `ChatPanel`을 그 카테고리로 **딱 한 번** 마운트한다(리마운트 없음, 고정-refs 불변식을 구성적으로
  충족). 분류 중에는 채팅 컬럼에 "작업 유형 분석 중… + 중지" placeholder를 표시하고, `ChatPanel`은 그동안
  마운트하지 않는다. 실패/취소/`null` → `"plan"` 폴백. **신규 백엔드/의존성 0, 프론트 전용.**
- **분류 신호 배선**: `HomeView.onStart`에 trailing optional `autoCategory?: boolean` 추가 — 컴포저 `send`는
  `start("plan", true)`(잠정 plan + 분류 요청), 카테고리 카드는 `start(id)`(false). `HomeArea`가 `autoCategory`
  state로 캡처해 `WorkspaceView`로 전달. `WorkspaceView.classifying` 초기값 =
  `autoCategory && !initialSession && !!seedPrompt.trim()`(빈 프롬프트·카드·loaded session·recent-no-session은
  분류 안 함). 분류 effect는 `classifyStartedRef` 가드(ChatPanel `bootedRef` 패턴 — cleanup 없음, StrictMode
  이중 실행 no-op). 분류 턴 취소는 `classifyCancelRef`(placeholder 중지 버튼 → `cancel()` → `null` → plan).
- **project.json category는 vestigial**: 분류 전 `ensureProject`가 잠정 `"plan"`을 매니페스트에 1회 기록하고
  (idempotent — 이후 호출은 그대로 반환), 실제 분류 카테고리는 마운트된 `ChatPanel`의 `persist()`가
  **`SessionMeta.category`** 로 저장한다. 프론트에서 `Project.category`/`ProjectSummary.category`를 읽는 코드는
  없고(재열기는 `session.category` 사용) 최근 목록에도 표시하지 않으므로, 잠정 plan이 박혀도 **표시·재열기에
  영향 없다**(별도 `set_project_category` 커맨드는 죽은 데이터라 도입하지 않음).
- **대안 기각**: **Home에서 분류**(cwd·projectId가 없어 프로젝트 라이프사이클을 HomeView로 끌어와야 함 +
  Home UI 블로킹) / **ChatPanel boot에서 분류 후 리마운트**(잠정 카테고리로 마운트했다 버림 — boot/`send`/
  `stop` 세 지점을 건드려 streaming 플래그 누출·`preflightAbortRef` 교차간섭 위험, 리마운트 낭비) /
  **`set_project_category` 백엔드 커맨드로 매니페스트 정정**(vestigial 데이터라 불필요).
- **하위호환/한계**: 프론트 전용 타입(`autoCategory`는 serde 미러 아님). 분류는 **추가 격리 턴 1회**라 진입에
  수 초의 "분석 중" 구간이 생긴다(prefill 턴처럼 숨김·중지 가능; aipro 등 느린 백엔드에선 더 길 수 있음 —
  D68 재시도와 무관, 분류 실패는 plan 폴백). plain(opencode/antigravity)·원격(fabrix/aipro)·세션리스(gemini)가
  펜스를 안 지키면 파싱 실패 → plan 폴백(D34/D40 degrade와 일관). 분류 근거는 초기 프롬프트 텍스트만
  사용한다(옵션 답변 이전 시점).

---

### D82. Confluence 연결 = 공식 MCP(streamable HTTP, JSON-RPC 2.0) 클라이언트 + 로컬 지식 베이스 수집 (D48/D65/D75/D77 대체)
- **배경**: 기존 Confluence 연결(REST 크롤 + WebView 스파이크)은 **실제로 값을 전달한 적이 없다** — REST가
  사내 WAF에 403으로 막히고(D75), WebView 우회(D77)는 실험 단계였으며, 최종 sink `RagClient::ingest_page`가
  **항상 "미구현" 에러를 반환하는 스텁**(rag-chat에 ingest 엔드포인트 없음 — D65)이라 모든 페이지가
  `PageFailed`로 끝났다. 사용자는 사내에서 **공식 제공 Confluence MCP 서버**
  (`https://sdsdev.co.kr/mcp-confluence/mcp`, streamable HTTP + `x-auth` 헤더)로 연결에 성공했고, 이를
  차용해 ① 설정 화면에서 URL(프리필)+x-auth 키를 저장, ② MCP 연결 테스트, ③ 이후 수집을 MCP로 처리하기를 원했다.
- **핵심 판단**: `confluence.rs`의 BFS 엔진(`crawl` + `ConfluenceApi` 트레이트 + `IngestEvent`/`IngestRegistry`)과
  그 `FakeApi` 단위 테스트는 transport와 무관하므로 **그대로 둔다**. 바꾸는 것은 **transport(REST→MCP)** 와
  **sink(RAG 스텁→로컬 지식 베이스)** 둘뿐이다. HTTP는 `fabrix.rs`/`aipro.rs`의 `reqwest` blocking+native-tls
  +`.no_proxy()`(D66) 레시피 재사용 — **신규 crate 0**(JSON-RPC framing·SSE는 `serde_json`으로 hand-roll).
- **결정(MCP 클라이언트 = 신규 `mcp.rs`)**: Confluence를 모르는 범용 **MCP-over-streamable-HTTP JSON-RPC 2.0
  클라이언트**(`McpSession`). handshake = `initialize`(응답 헤더 `Mcp-Session-Id` 캡처, `protocolVersion` 반영)
  → `notifications/initialized` → `tools/list`. 요청 헤더 = `Content-Type: application/json` +
  `Accept: application/json, text/event-stream` + `x-auth` + (post-init) `Mcp-Session-Id`/`MCP-Protocol-Version`.
  응답은 **`application/json`(단일 JSON-RPC)와 `text/event-stream`(SSE) 양쪽 모두** 처리(본문을 통째로 읽어
  content-type 분기 — MCP 도구 응답은 유한 메시지라 증분 리더 불필요; 120초 total timeout이 hung 스트림을
  bound). 세션 전략 = **operation당 1회 handshake**(전역 캐시 없음; mid-op HTTP 404면 1회 재handshake 후 재시도).
  `sdsdev.co.kr` 게이트웨이가 UA를 gate하므로(D74/D75) `User-Agent: opencode/0.1.0` 부착(406/403이면 조정).
  순수 파서(`parse_jsonrpc_body`/`sse_event_to_result`/`tool_result_text`/`parse_tools`/`arg_key_for`) 단위 테스트.
- **결정(수집 = MCP 크롤 → 지식 베이스 artifact)**: `McpConfluence`가 기존 `ConfluenceApi`를 구현
  (`RefCell<McpSession>` — 트레이트 `&self` + `call_tool` `&mut`; getPageById/getChild/searchContent 호출,
  결과 텍스트를 관대 파싱). 도구 이름·인자 키는 `tools/list`의 스키마에서 해석(`arg_key_for`, 폴백 포함) —
  와이어 계약을 하드코딩하지 않음. `start_confluence_ingest`는 **`ConfluenceTarget{rootPageId, searchQuery}`**
  (제거된 config 필드 대체 — 수집 패널에서 per-run 전달)를 받아 `crawl`을 돌리고, 각 페이지를 버퍼에 모아
  **트리 전체를 artifact `KnowledgeEntry` 1개**로 저장한다(`knowledge.rs`의 신규 in-memory writer
  `save_knowledge_docs` — staged-swap; 페이지=파일, body=요약+제목 색인). 페이지당 note는 16KB 주입 상한을
  몇 건 만에 초과하므로 artifact 모델(D59: 요약+파일 색인만 주입, extraDirs로 원문 읽기)이 정답. 취소 시에도
  부분 버퍼를 저장. `IngestEvent`/`Channel`/`IngestRegistry` 진행·취소는 그대로. **RAG 의존 제거**(더 이상
  `settings.rag`/`RagClient` 로드 안 함).
- **결정(연결 테스트)**: `probe_confluence` = `McpSession::connect`(initialize+tools/list) 후 "연결됨 — N개 도구
  (getChild, searchContent, …)". 잘못된 x-auth → HTTP 401/403 + 본문 스니펫.
- **결정(설정·UI)**: `ConfluenceConfig`를 `{ url, auth_key }`로 재구성(구 `baseUrl`/`token`/`rootPageId`/
  `spaceKey`/`allowInvalidCerts`는 serde가 unknown 필드로 무시 — 마이그레이션 없이 로드, `url` 빈값→미설정
  → 사용자 재저장, URL은 프리필). `set_confluence_config`는 url trim + auth_key trim/empty→None. UI는 지식 뷰
  `ConfluenceSection`을 **URL(프리필)+x-auth 2필드 + 연결 테스트 + 수집 대상(루트 페이지 ID/검색어) 패널**로
  재작성; WebView 스파이크·PAT·루트/스페이스·TLS 필드 전부 제거. `rag.rs`의 `confluence.allow_invalid_certs`
  참조 2곳은 `false`로 교체(TLS 상시 on, D75 일관).
- **삭제**: `HttpConfluence`(REST)·`CONFLUENCE_UA`·WebView 스파이크(`confluence_open_login`/
  `confluence_fetch_page`/`WebviewBridge`/`WebviewFetchResult`/`parse_nav_payload`/`INIT_SCRIPT`)와 그 커맨드
  등록·managed state. 프론트 `confluenceOpenLogin`/`confluenceFetchPage`/`WebviewFetchResult`.
- **대안 기각**: **온디맨드 검색 뷰만**(대량 수집 대신 검색 — 사용자가 "지식 베이스로 수집" 선택) /
  **RAG ingest 구현으로 sink 유지**(rag-chat에 ingest 엔드포인트 없음 — 불가) / **페이지당 note 엔트리**
  (16KB 상한 초과·목록 오염) / **REST 크롤 유지 + fallback**(WAF 403이 근본 문제 — 사용자가 전면 제거 선택) /
  **incremental SSE 리더**(도구 응답이 유한 메시지라 전체 읽기가 단순·충분).
- **하위호환/한계**: `ConfluenceConfig`는 `#[serde(default)]`라 구 settings.json 무변경 로드(구 필드 무시).
  `ingest_page` 스텁은 rag.rs에 남지만 Confluence가 더 이상 타깃 안 함(D65 유지). **MCP 도구 I/O 실제 shape은
  런타임에 확정** — 파서가 여러 후보 키를 관대하게 시도하고(id: `id|pageId|contentId`, body:
  `body.storage.value|body|content|value|text`, 목록: `results|children|pages|…`) 비-JSON 텍스트는 본문으로
  폴백하지만, 서버가 예상 밖 shape면 연결 테스트의 도구 목록·실제 수집 결과로 확인·조정이 필요하다.
  getChild는 페이지네이션 없이 전체 자식을 반환한다고 가정(REST식 start 무시).

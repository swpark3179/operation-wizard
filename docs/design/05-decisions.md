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
     "Samsung SDS · Operation Wizard", 제목 "운영 작업 도우미", 부제에 진행 절차(요구사항 확인 →
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

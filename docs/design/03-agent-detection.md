# 03. 핵심 기능 — 로컬 CLI 에이전트 탐지

이 기능은 Open Design 데몬(`apps/daemon/src/runtimes/`)의 동작을 Rust로 포팅한 것이다.
원본 사양: Open Design 저장소 `docs/cli-agent-detection-and-daemon.ko.md`.

구조는 **"런타임 정의(def) + 공통 probe"** 다. 에이전트별 데이터(`AgentDef`)는
`agents.rs`의 레지스트리에 모으고, `detect.rs`의 파이프라인은 그 정의를 받아 동작한다.
(원본 대응: `apps/daemon/src/runtimes/defs/*.ts` → `agents.rs`,
`detection.ts` `probe()` → `detect.rs`.)

## 에이전트 레지스트리 (`agents.rs`)

`AGENT_DEFS`는 7개 에이전트 정의를 **표시 순서대로** 담은 정적 배열이다.
`find(id)`로 조회하고, `all()`로 전체를 얻는다. 로컬 CLI 6종은 Open Design defs에서 1:1 포팅했고,
`fabrix`는 원격 HTTP API 에이전트다(`kind: Remote` — D64, 아래 표 하단 ✧).

| id | name | kind | bin 후보 | env override | 추가 검색 하위경로 | 모델 명령 / 파서 / 타임아웃 | fallback 모델 (default는 코드에서 prepend) |
|----|------|------|----------|--------------|-------------------|------------------------------|---------------------------------------------|
| `opencode` | OpenCode | Local | `opencode-cli`, `opencode` | `OPENCODE_BIN` | `.opencode\bin` | `models` / line / 15s | `anthropic/claude-sonnet-4-5`, `openai/gpt-5`, `google/gemini-2.5-pro` |
| `claude` | Claude Code | Local | `claude`, `openclaude` | `CLAUDE_BIN` | — | (없음 → fallback) | `sonnet`(Sonnet alias), `opus`, `haiku`, `claude-opus-4-5`, `claude-sonnet-4-5`, `claude-haiku-4-5` |
| `codex` | Codex CLI | Local | `codex` | `CODEX_BIN` | — | `debug models` / JSON / 5s | `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`, `gpt-5.3-codex`, `gpt-5.1`, `gpt-5.1-codex-mini`, `gpt-5-codex`, `gpt-5`, `o3`, `o4-mini` |
| `gemini` | Gemini CLI | Local | `gemini` | `GEMINI_BIN` | — | (없음 → fallback) | `gemini-3-pro-preview`, `gemini-3-flash-preview`, `gemini-2.5-pro`, `gemini-2.5-flash`, `gemini-2.5-flash-lite` |
| `antigravity` | Antigravity | Local | `agy` | `ANTIGRAVITY_BIN` ※ | — | (없음 → fallback) | `Gemini 3.1 Pro (High/Low)`, `Gemini 3.5 Flash (High/Medium/Low)`, `Claude Sonnet 4.6 (Thinking)`, `Claude Opus 4.6 (Thinking)`, `GPT-OSS 120B (Medium)` |
| `aipro` | AI Pro | Local | `aipro` | `AIPRO_BIN` ✦ | — | (없음 → fallback) | `glm-5.1`(GLM-5.1), `qwen3.6-27b`(Qwen3.6-27b), `gpt-oss-120b`(Gpt-Oss-120b) |
| `fabrix` | Fabrix | **Remote** ✧ | — | — | — | HTTP GET `all-models` / ko-name 매핑 / — | (없음 — 라이브 전용) |

> ※ `ANTIGRAVITY_BIN`은 Open Design에 없는 항목으로, 일관성을 위해 추가한 의도적
> 편차다([05-decisions.md](05-decisions.md) D15). 모델 id와 label이 다른 경우(claude
> alias, antigravity, aipro)는 `(id, label)` 튜플로 보존한다.
>
> ✦ `aipro`는 사내 도구로, Open Design에서는 `~/.open-design/agents.local.json`의
> `baseAgent: "gemini"` 로컬 프로필이었다. 이 앱에서는 런타임 프로필 로더 대신 **빌트인 def**로
> 추가했다([05-decisions.md](05-decisions.md) D17). gemini 호환이라 모델 나열 명령이 없어
> fallback 전용이며, 프로필의 `env`(`GEMINI_CLI_TRUST_WORKSPACE`)는 실행 전용이라 탐지에는
> 미반영. `AIPRO_BIN`은 `*_BIN` 일관성용 보조 override다.
>
> ✧ `fabrix`는 **첫 원격 HTTP API 에이전트**(`kind: Remote` — [05-decisions.md](05-decisions.md) D64)다.
> CLI 필드(bin/env/probe/run)를 쓰지 않고 **`fabrix.rs`** 의 HTTP 경로로 우회한다: 탐지는
> 설정(`settings.fabrix`) 유무 + `GET {endpoint}/openapi/chat/v1/all-models` 도달성으로 판정하고,
> 모델 목록은 응답 배열의 `modelId`(id) + `name`의 `languageCode=="ko"` content(label)로 매핑한다.
> 실행(채팅)은 `POST .../messages` + SSE 스트리밍([07-workspace-and-runs.md](07-workspace-and-runs.md)).
> fallback 카탈로그가 없어(라이브 전용) 미설정 시 모델 목록은 빈 배열 + `diagnostic: not-configured`.

`AgentDef` 필드: `id`, `name`, `kind: AgentKind {Local, Remote}`(D64), `bin_candidates`,
`env_var: Option`, `extra_search_subdirs`, `version_timeout`, `models_probe: Option<ModelsProbe>`,
`fallback_models: &[(id,label)]`, `run: Option<RunSpec>`(실행 스펙 — 탐지가 아닌 **실행**용, 자세히는
[07-workspace-and-runs.md](07-workspace-and-runs.md)). `kind == Remote`(fabrix)면 CLI 필드는 빈 값이고
탐지·실행이 `fabrix.rs`로 분기한다.
`ModelsProbe`: `args`, `parse: fn(&str)->Option<Vec<ModelOption>>`, `timeout`.

## 파이프라인 (3단계, def 기반)

```
resolve  →  version probe  →  models probe(있으면)  →  DetectedAgent
(경로찾기)  (--version, def.timeout)   (def.args, def.timeout)   (UI로 반환)
```

`detect_agent_blocking(def, custom)`가 **로컬(`kind: Local`)** 에이전트를 탐지한다(version args는
`--version` 하드코딩, 타임아웃·모델 명령·파서·fallback은 def에서). **원격(`kind: Remote`, fabrix)**
에이전트는 이 파이프라인을 건너뛰고 `detect_agent` 커맨드가 `fabrix::detect_fabrix(cfg)`로 분기한다
(resolve/spawn 없이 설정 유무 + HTTP 도달성으로 판정 — D64).

### 1) Resolve — 실행 파일 경로 해석 (`resolve.rs`)

`resolve_agent(def, custom)`. 찾는 바이너리 후보는 `def.bin_candidates` 순서.

**우선순위**
1. **사용자 지정 경로**: 저장된 설정(`agents[id].customBin`), 없으면 `def.env_var`가
   가리키는 환경변수. 절대경로이고 실제 파일이면 source = `custom-path`.
   (사용자 지정 경로가 있으나 무효이면 env로 가지 않고 곧장 검색으로 fall-through.)
2. **검색 경로 스캔**: 아래 디렉터리들을 `PATHEXT` 확장자(`.EXE;.CMD;.BAT` + 확장자 없음)와
   조합해 탐색. 찾으면 source = `path`.

**검색 디렉터리** (순서 보존, 중복 제거) — 모든 에이전트 공유 + def별 추가:
- 프로세스 `PATH` 전체
- `%APPDATA%\npm` (npm 전역 기본 — 가장 중요)
- `NPM_CONFIG_PREFIX` 및 그 `\bin`
- 공통 툴체인 디렉터리: `scoop/shims`, `.bun/bin`, `.cargo/bin`, `.local/bin`,
  `.deno/bin`, `.volta/bin`
- **`def.extra_search_subdirs`** (`%USERPROFILE%` 기준) — 예: opencode의 `~/.opencode/bin`
- fnm node-versions: `%APPDATA%\fnm`, `%LOCALAPPDATA%\fnm`, `FNM_DIR` 하위의
  각 버전 `installation` 디렉터리

> **왜 PATH만으로 부족한가**: GUI/패키징된 앱은 축소된("stripped") PATH로 실행되는
> 경우가 많아, npm/툴체인 설치 위치를 명시적으로 보강해야 한다.
> 이는 Open Design의 `wellKnownUserToolchainBins` 보강과 동일한 의도다.

### 2) Version probe (`exec.rs`, 타임아웃 `def.version_timeout` = 3초)

찾은 실행 파일을 `--version`으로 실행한다.
- spawn 자체 실패 → 권한 거부면 `not-executable`, 그 외 `missing-target`.
- 종료코드 127 → `missing-target`, 126 → `not-executable`.
- **spawn 성공 시점에 `available = true`** (버전 파싱 실패해도 "있긴 함"으로 간주).
- 정상 종료(0)이고 타임아웃 아니면 stdout 첫 줄을 버전으로 채택.

### 3) Models probe (`exec.rs`, `def.models_probe`가 `Some`일 때만)

`def.models_probe.args` 실행 → `def.models_probe.parse`로 파싱.
- **probe 성공 게이트**: spawn 성공 && 타임아웃 아님 && 종료코드 0 && stdout 비어있지 않음.
- 게이트 통과 후 파서가 `Some(models)`를 주면 `models_source = live`로 채택.
- 게이트 실패 / 파서 `None` / `models_probe == None`(claude·gemini·antigravity)이면
  **def의 정적 fallback 카탈로그** 사용(`models_source = fallback`).

## 모델 파서 (`detect.rs`)

두 파서 모두 시그니처 `fn(&str) -> Option<Vec<ModelOption>>` (실패/0개면 `None` → fallback).
둘 다 맨 앞에 합성 `default` 옵션을 prepend한다.

- **`parse_line_separated_models`** (opencode): `parseLineSeparatedModels` 포팅 —
  한 줄당 id 하나, trim, 빈 줄·`#` 주석 제거, 순서 보존 중복 제거. 항상 `Some`.
- **`parse_codex_debug_models`** (codex): `parseCodexDebugModels` 포팅 — `debug models`의
  JSON(`{models:[…]}`) 파싱, `visibility=="hidden"` 스킵, id = `slug`||`id`(trim),
  label = `display_name`||`name`||id, 중복 제거. 파싱 실패/유효 모델 0개면 `None`.
- **claude / gemini / antigravity**: 모델 나열 명령이 없어 파서 없음 → 항상 fallback.
  (claude는 Open Design에서 MMS 라우트 fetch를 쓰지만 자체 프록시 인프라 의존이라
  이 로컬 앱에서는 제외 — [05-decisions.md](05-decisions.md) D12.)

## 실행 안전성 (`exec.rs`) — Windows 특이사항

`run_capture(path, args, timeout)`는 에이전트 무관 공통 함수다.
- **`.cmd`/`.bat` 실행**은 `cmd.exe /d /s /c <path> <args>`로 감싼다.
  - Rust 표준 라이브러리의 *BatBadBut* 완화책은 `.bat/.cmd`에 인자 전달을 거부하므로,
    실제 `.exe`인 `cmd.exe`를 통해 우회한다.
  - `/d`=AutoRun 생략, `/s /c`=나머지를 명령으로 실행.
  - 공백 포함 경로는 따옴표로 감싸지지만, **뒤에 인자가 하나 이상 따라오면**
    cmd.exe의 `/s` "바깥 따옴표 제거" 규칙이 발동하지 않아 경로가 올바르게 파싱된다.
    → 그래서 `run_capture`는 **항상 인자를 1개 이상** 받도록 호출한다.
- **콘솔 깜빡임 방지**: `CREATE_NO_WINDOW` 플래그로 실행(윈도우 외 무시).
- **파이프 데드락 방지**: 큰 `models` 출력(>1MB)이 파이프 버퍼를 채워 멈추지 않도록
  stdout/stderr를 별도 스레드에서 끝까지 읽어낸다.
- 타임아웃 초과 시 자식 프로세스를 kill.

## 결과 모델 (`DetectedAgent`)

백엔드 직렬화는 `camelCase`. 프론트 미러는 `lib/types.ts`.

| 필드 | 의미 |
|------|------|
| `id`, `name` | def의 `id`/`name` (예: `"opencode"`, `"OpenCode"`) |
| `available` | 실행 파일을 spawn할 수 있었는지 |
| `path` | 해석된 실행 파일 경로 (없으면 null) |
| `version` | `--version` 첫 줄 (없으면 null) |
| `source` | `custom-path` / `path` / `not-found` / **`remote`**(fabrix — D64) |
| `models` | `{ id, label }[]` (로컬은 맨 앞 `default`; **fabrix는 실제 modelId만, `default` 없음**) |
| `modelsSource` | `live` / `fallback` |
| `diagnostic` | `not-on-path` / `not-executable` / `missing-target` / **`not-configured` / `unreachable`**(fabrix) / null |

프론트의 `DIAGNOSTIC_HINT` 맵이 진단 코드를 사용자용 안내문으로 변환한다(fabrix의 `not-configured`/
`unreachable` 포함).

## 설정 영구화 (`settings.rs`)

- `Settings { agents: { [agentId]: { customBin } }, skills?, workflows?, confluence?, rag?, fabrix? }` —
  에이전트별 경로 맵 + **워크플로우 단계/스킬 override**(탐지와 무관, [07](07-workspace-and-runs.md)·
  [05](05-decisions.md) D39) + **`fabrix: FabrixConfig?`**(endpointUrl/client/openapiToken/allowInvalidCerts —
  fabrix 원격 에이전트 연결, D64. `set_fabrix_config`로 저장/해제).
- 앱 config 디렉터리의 `settings.json`에 pretty JSON으로 저장.
- 파일 없음/파싱 실패 시 기본값(빈 맵).
- **레거시 마이그레이션**: v0.1의 단일 `opencodeBin` 필드가 있으면 load 시
  `agents.opencode.customBin`으로 흡수하고 다시 저장할 때 제거한다(self-healing).

## 테스트 (cargo test)

- **line 파서 단위 테스트**: trim/주석/중복 제거 + `default` prepend 검증.
- **codex JSON 파서 단위 테스트**: hidden 스킵 / slug·id / display_name·name / 중복 제거 /
  빈·비JSON → `None`.
- **레지스트리 sanity**: 7개 id 유일·비어있지 않음, **로컬** 에이전트는 fallback 비어있지 않음(remote는
  예외), 7개 id 조회됨, `fabrix`는 `kind: Remote`.
- **fabrix 파서 단위 테스트**: `parse_models_json`(ko content 매핑·폴백·비배열/비JSON→Err),
  `parse_fabrix_sse_data`(CHUNK→TextDelta·SUCCESS 마커→무이벤트·실패 status→Error).
- **E2E(Windows)**: 임시 `opencode.cmd` 스텁을 만들고 `detect_agent_blocking(find("opencode"))`로
  탐지 → resolve + cmd.exe 래핑 + version/models 파싱까지 통과하는지 확인.
- 잘못된 custom-path가 검색으로 fall-through 하는지 확인.
- **settings 단위 테스트**: skills/workflows 라운드트립, 구파일 하위호환(+레거시 마이그레이션 유지),
  `validate_steps`/`validate_skills` 규칙, reset(`None`) 의미론.

## 확장 포인트 (새 에이전트 추가)

**로컬 CLI 에이전트**:
1. `agents.rs`의 `AGENT_DEFS`에 `AgentDef` 항목 1개 추가(`kind: Local`, bin 후보/env/타임아웃/fallback).
2. 그 에이전트가 모델을 나열한다면 `models_probe`를 채우고, 출력 형식이 기존과 다르면
   `detect.rs`에 `fn(&str)->Option<Vec<ModelOption>>` 파서를 추가해 연결.
3. 프론트는 자동 반영(레지스트리를 `list_agents`로 받아 카드/설정 행을 렌더).

**원격 HTTP API 에이전트**(fabrix 패턴 — D64): `AGENT_DEFS`에 `kind: Remote` def 추가(CLI 필드는 빈 값) →
`detect_agent`/`run_agent`의 `kind == Remote` 분기에서 해당 HTTP 모듈로 위임 → 연결 설정은 `Settings`에
필드 추가 + `set_*_config` 커맨드 → 프론트는 `AgentsView`가 id로 분기해 전용 설정 카드를 렌더.

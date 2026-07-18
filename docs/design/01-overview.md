# 01. 프로젝트 개요

## 무엇인가

**Operation Wizard**는 Windows 데스크톱 앱이다.
로컬 머신에 설치된 **CLI 코딩 에이전트를 탐지·관리**하고, 나아가 **대화 패널 + 캔버스 패널
워크스페이스에서 실제로 실행**해 프로그램 개발 업무를 진행하는 것을 목적으로 한다.

- 제품명: `Operation Wizard`
- 식별자: `com.shi.operationwizard`
- 제작: Samsung SDS
- 현재 버전: `0.1.0` (초기 단계)
- 대상 플랫폼: **Windows** (Windows 11 우선, WebView2 내장 가정)

## 목표

- 사용자가 직접 PATH/설치 경로를 신경 쓰지 않아도, 앱이 로컬 에이전트의
  **설치 여부·경로·버전·사용 가능한 모델**을 자동으로 찾아 보여준다.
- 자동 탐지가 실패하는 환경(사내망, 비표준 설치)에서는 **사용자 지정 경로**로
  보완할 수 있게 한다.
- 차분하고 정돈된 제품 UI(Open Design 계열)를 제공한다.

## 현재 범위 (v0.1)

- **핵심 기능: 코딩 에이전트 탐지·관리(다중).**
  - 대상 에이전트: **OpenCode, Claude Code, Codex, Gemini CLI, Antigravity**(로컬 CLI 5종) +
    **Fabrix**(D64)·**AI Pro**(D71) (원격 HTTP API 2종). 총 7종.
    AI Pro는 사내 도구로 **OpenAI 호환 HTTP API**다(과거엔 gemini 호환 로컬 CLI였으나 D71에서 원격으로 전환).
  - **로컬 CLI**: 실행 파일 해석(resolve) → `--version` 프로브 → 모델 목록화. **Fabrix**: 설정
    (endpoint+토큰) 유무 + HTTP 도달성으로 탐지, 모델 목록은 `all-models` API를 호출해 한글 모델명으로 매핑.
    **AI Pro**: 설정(endpoint+API 키) 유무로 탐지(네트워크 없음), 모델 목록은 **정적 카탈로그**
    (glm-5.1/gpt-oss-120b/qwen3.6-27b) — 모델을 이미 알고 있어 `/models` 미조회(D73). 도달성은
    **연결 테스트**(최소 `POST /chat/completions`)로 검증. 모든 AI Pro 요청은 `User-Agent: opencode/<ver>`가
    **필수**다(사내 게이트웨이 allowlist + 백엔드가 UA를 split — 누락 시 HTTP 500/406, D74).
  - 결과를 에이전트별 카드 UI로 표시(로컬=상태/경로/버전/모델; Fabrix=상태/모델 + endpoint·client·token
    입력 폼; AI Pro=상태/모델 + endpoint·API 키 입력 폼 + 연결 테스트). 로컬은 사용자 지정 경로,
    Fabrix·AI Pro는 연결 정보를 영구 저장(settings.json).
- 탐지 로직은 **"런타임 정의(def) + 공통 probe"** 구조다(`AgentDef.kind: Local | Remote`). 새 로컬 에이전트 =
  레지스트리에 정의 1개 추가; 원격 에이전트 = def(`kind: Remote`) + 탐지·실행 분기(자세히는
  [03-agent-detection.md](03-agent-detection.md)).
- **에이전트 실행(run) 워크스페이스 (증분 1):** HOME 런처(프롬프트 + 업무 카테고리) →
  좌 대화 패널 + 우 캔버스 패널. 대화는 실제 에이전트를 실행해 응답을 스트리밍한다
  (**Claude Code·Codex·Gemini** 1급 구조화 스트림, opencode/antigravity는 plain 폴백,
  **Fabrix·AI Pro는 HTTP POST + SSE 스트림** — Fabrix D64/AI Pro D71. 두 원격 에이전트는 파일을 못 쓰므로
  문서 단계 산출물은 앱이 스트리밍 텍스트를 워크스페이스 파일로 저장해 다른 에이전트처럼 산출물 탭에 표시한다 — D67).
  정지(프로세스 트리 종료)와 세션 이어가기를 지원한다. 캔버스는 작업 폴더 파일 뷰어
  (트리 + 코드/HTML 미리보기). 실행 엔진은 Open Design 데몬의 run/stream을 Tauri `Channel`로
  포팅한 것(자세히는 [07-workspace-and-runs.md](07-workspace-and-runs.md)).
- **대화 영속화(증분 2):** 대화를 파일로 저장한다.
  `~/.operation-wizard/projects/<projectId>/{workspace,sessions/<sessionId>}`에 기록하며, 첫 질문 시
  폴더가 생성되고 프로젝트별 세션 기록을 **열람·이어가기**할 수 있고 **새 세션**(에이전트 재선택 가능)을
  만들 수 있다. codex의 사내 TLS 인증서 오류는 안내를 개선하고 새 세션으로 복구한다.
- **프로젝트 격리(증분 4):** **프로젝트 ≠ 작업 폴더**로 바꿨다. `projectId`는 프론트가 mint하고, 홈에서
  **새 채팅/카테고리로 시작하면 매번 새 프로젝트**가 된다(최근 목록 클릭은 그 프로젝트의 마지막 세션을
  이어감). 작업 폴더(실행 cwd)는 프로젝트별 값으로, 홈의 **작업 폴더 지정 버튼**으로 폴더를 고르면 그 폴더를,
  고르지 않으면 프로젝트 전용 `workspace/` 폴더를 자동 생성해 쓴다(상단바의 폴더 표시는 없음 —
  [07](07-workspace-and-runs.md), [05](05-decisions.md) D32/D33).
- **카테고리 가이드 플로우(증분 3→6):** 카테고리마다 **고정 선택지 우선 시작 + 단계 오케스트레이터(단계별
  스킬 주입)**를 클라이언트에서 구동한다(`lib/options.ts`·`lib/skills.ts`·`lib/workflow.ts`). **홈에서
  카테고리 카드를 고르면 그 카테고리로, 프롬프트를 입력해 시작하면 진입 후 격리 분류 턴이 프롬프트에 가장
  맞는 카테고리를 자동 선택한다(`lib/categorize.ts`, 실패 시 개발 계획 수립 폴백 — D81).** 카테고리
  진입 시 프롬프트가 아니라 **카테고리별 고정 선택지 폼을 먼저** 보여주고(홈 프롬프트로 시작했으면 아는 값을
  숨김 프리필 턴으로 자동 채움; **폼 대기 중 채팅 차단, 제출 시 '요구사항' 탭 소멸**), 폼 제출이 첫 작업
  턴을 발사한다. 폼 맨 앞에는 **요구사항 필드(무엇을 하고 싶은지)가 필수·우선**으로 놓이고(홈 seed로 자동
  채움), 첫 작업 턴에는 **내장 '프롬프트 최적화' 스킬**이 실려 에이전트가 최적 프롬프트를 먼저 내면 앱이
  이를 캔버스 **'프롬프트' 탭**에 표시한다(프롬프팅 학습용 — D78). 프롬프트 주입 + 텍스트 블록/도구 이벤트
  해석 방식이며 실패 시 일반 chat 폴백(자세히는
  [07-workspace-and-runs.md](07-workspace-and-runs.md)·[08-guided-flows-and-skills.md](08-guided-flows-and-skills.md)).
- **설정형 워크플로우·스킬(증분 6):** 단계·스킬의 **정의는 사용자 설정**이다 — **Flows 설정 화면**에서
  카테고리별 단계(이름/종류/지시문/산출물 파일/결과 형태/스킬 연결)와 스킬 레지스트리를 편집해
  `settings.json`에 저장한다(`set_skills`/`set_workflow`). 코드 기본값이 폴백=샘플이며, 생성형 단계는
  자동 진행하고 산출물은 캔버스 **마크다운+mermaid 미리보기**로 표시된다(D39~D42).
- **기반 3단계 + 지식 인프라(증분 7):** 워크플로우 앞에 **반드시 거치는 기반 3단계**를 도입했다 —
  ① **코드베이스 분석**(첫 질문에서 코드베이스 폴더 필수 선택; workdir와 별개, claude `--add-dir` +
  gemini `--include-directories`로 접근 부여(D52), 사용자 탐색 스킬 연결 가능. 원격 에이전트(Fabrix·AI Pro)는
  파일 접근이 없어 코드베이스 실독 불가 — D67 한계) → ② **사내 문서 RAG 검색**(실행 시점에는
  `rag_search`가 **Fabrix rag-chat API**를 호출해(D65) 요약 답변+출처 청크를 캔버스 '검색 결과' 탭(HTML)으로
  보여주고 에이전트에 주입; 지식 화면에서 **공식 Confluence MCP 서버**(url+x-auth)에 연결해 페이지 트리를
  수집하면 로컬 지식 베이스 artifact로 저장돼 knowledge 단계가 참고한다 — D82) → ③ **지식 베이스
  반영**(지식 화면에서 제목+본문 CRUD, 단순 저장 후 프롬프트 주입). 이후 기존 설정 단계들이 이어진다.
  `plan`·`query`(D61)·`change`(D62)는 기본값에 완전 기반 3단계 포함, `guide`(D63)는 **코드베이스를 뺀 부분
  foundation(rag+knowledge)** 기본 포함(사내 문서 시각화가 강점 — 코드베이스 폴더 선택 강제 없음).
  스킬은 **리소스 폴더**(claude skill 스타일
  참고 파일/스크립트)를 가질 수 있고, 단계는 **결과 형태**(대화만/파일/HTML 캔버스 — HTML은 내장
  `html-render` 스킬 턴으로 재생성)를 고른다(D44~D48).
- **산출물 지식 저장(증분 8):** 워크플로우가 완료되면(컴포저 위 배너 제안) 또는 산출물 탭에서 수동으로,
  산출물 문서들을 **지식 베이스의 '산출물' 항목**으로 저장한다 — 파일은 `knowledge/artifacts/<id>/`로
  복사되고 격리 에이전트 턴이 생성한 요약(편집 가능)이 body가 된다. 이후 작업의 knowledge 단계는
  **요약 + 문서 절대경로 인덱스**를 주입하고 extraDirs로 원문 읽기 접근을 부여해, 16KB 주입 상한과
  무관하게 과거 산출물 전체를 참고할 수 있다(D59).

## 범위 밖 (현재 미포함)

- 디자인의 아티팩트별 전용 캔버스 탭(저장소 분석/영향도/변경 가이드 등) — **집계 뷰는 캔버스 '산출물'
  허브 + '다이어그램' 갤러리 탭으로 구현됨(D58)**; 아티팩트별 전용 뷰/오케스트레이션은 후속.
  (요구사항 명확화·소스 조사·계획서 생성은 `plan`, 참조 SQL·테이블 ERD·참고 SQL 산출은 `query`(D61),
  변경 종류별 DC Manager 신청양식(HTML) 생성은 `change`(D62), 사내 문서 시각화 + 운영 가이드(HTML)
  산출은 `guide`(D63)에 한해 카테고리 워크플로우로 구현됨. guide에 따른 실제 운영 작업 자동화는 후속.)
- 대화는 파일(JSON)로 영속화된다([07](07-workspace-and-runs.md)). SQLite/전문 검색/여러 프로젝트
  목록 화면은 후속 증분. opencode/antigravity의 1급 실행 파서(현재 plain; claude·codex·gemini은
  1급 지원, Fabrix·AI Pro는 원격 HTTP+SSE).
- RAG 검색(`search`)은 **Fabrix rag-chat API 실연동됨**(D65). **Confluence 수집은 공식 MCP 서버로 전환됨**
  (D82) — 페이지 트리를 가져와 로컬 지식 베이스 artifact로 저장하고 knowledge 단계가 참고한다(구 REST 크롤·
  WebView 스파이크·RAG `ingest_page` sink는 폐기; `ingest_page`는 호출부 없는 스텁으로만 남음).
- 인증/로그인 상태 프로브(Open Design의 claude/codex `authProbe`는 후속 작업).
- Claude의 라이브 모델 목록(Open Design의 MMS 라우트 fetch는 자체 프록시 인프라
  의존이라 제외 — 정적 fallback만 사용).
- macOS/Linux 지원(해석 로직은 Windows 툴체인 경로에 특화됨).
- 자동 업데이트, 텔레메트리.

## 핵심 혈통(Heritage)

탐지 로직은 **Open Design의 데몬 동작**(`apps/daemon/src/runtimes/`)을
**Rust로 재구현**한 것이다. 동작 사양의 원본은 Open Design 저장소의
`docs/cli-agent-detection-and-daemon.ko.md`다.
즉, "이미 검증된 TypeScript 데몬 로직을 Tauri 백엔드(Rust)에 포팅"한 것이
이 프로젝트 핵심 기능의 본질이다. (자세한 매핑은 [03-agent-detection.md](03-agent-detection.md))

## 용어

| 용어 | 의미 |
|------|------|
| Agent | 로컬 CLI 코딩 에이전트 (현재는 OpenCode) |
| Resolve | 실행 파일의 실제 경로를 찾는 단계 |
| Probe | 찾은 실행 파일을 짧게 실행해 버전/모델을 캡처하는 단계 |
| Source | 실행 파일을 찾은 출처: `custom-path` / `path` / `not-found` |
| Diagnostic | 탐지 실패 원인 분류: `not-on-path` / `not-executable` / `missing-target` |
| Models source | 모델 목록의 출처: `live`(CLI 실측) / `fallback`(정적 카탈로그) |
| Shim | `.cmd`/`.bat` 래퍼 실행 파일 (npm 전역 설치 등에서 생성) |

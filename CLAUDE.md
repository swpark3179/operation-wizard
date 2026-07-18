# CLAUDE.md — Operation Wizard

이 파일은 매 세션 자동으로 컨텍스트에 로드된다. 아래 설계문서들도 `@` 임포트로
**모든 작업/질문 시 항상 함께 로드**된다. 별도 언급이 없어도 이 설계문서를 기준으로
판단·답변·작업한다.

## 한 줄 요약

Operation Wizard = Windows 데스크톱 앱(Tauri v2 + React/TS + Tailwind v4).
코딩 에이전트를 탐지·관리하고(로컬 CLI: OpenCode·Claude Code·Codex·Gemini·Antigravity +
원격 HTTP API: Fabrix(D64)·AI Pro(사내, OpenAI 호환, D71)),
**대화 패널 + 캔버스 패널 워크스페이스에서 실제로 실행**한다(Claude Code 1급, 나머지 plain 폴백, Fabrix·AI Pro는 HTTP+SSE).

## 설계문서 (Single Source of Truth)

@docs/design/README.md
@docs/design/01-overview.md
@docs/design/02-architecture.md
@docs/design/03-agent-detection.md
@docs/design/04-ui-and-design-system.md
@docs/design/05-decisions.md
@docs/design/06-build-and-environment.md
@docs/design/07-workspace-and-runs.md
@docs/design/08-guided-flows-and-skills.md

## 작업 규칙 (필수)

1. **설계를 바꾸는 변경은 코드와 함께 `docs/design/`의 해당 문서를 같은 작업에서 갱신한다.**
   - 새 기능/모듈/Tauri 커맨드 추가, 데이터 모델(`DetectedAgent`/`Settings`) 변경,
     기술 선택 변경, 빌드 절차 변경, 디자인 토큰/UI 구조 변경 등.
   - 변경에 이유가 있으면 `docs/design/05-decisions.md`에 결정 항목으로 추가한다.
   - 문서가 길어지면 분할하고, `README.md` 인덱스와 위 임포트 목록을 함께 갱신한다.
2. 설계문서는 **컨셉 수준**을 유지한다. 라인 단위 디테일은 코드/주석에 둔다.
3. 백엔드 serde 구조체(`detect.rs`/`settings.rs`/`run.rs`/`files.rs`)와 프론트 `src/lib/types.ts`는
   항상 동기화한다(직렬화는 `camelCase`).

## 빌드 주의 (요약 — 상세는 06번 문서)

- **Rust/Tauri 빌드는 MSVC 환경에서만.** Git Bash 금지(`link.exe` 충돌), 순수 PowerShell도 링커 못 찾음.
  → "Developer PowerShell for VS 2022" 또는 `vcvars64.bat` 초기화 후 실행.
- 프론트 전용 명령(`npm install`/`npm run dev`/`build`)은 아무 셸에서나 가능.

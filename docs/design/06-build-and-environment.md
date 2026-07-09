# 06. 빌드 & 환경 제약

> 이 문서는 **개발 환경에서 자주 막히는 지점**을 설계 차원에서 못 박아 둔 것이다.
> 실행 커맨드 자체는 루트 `README.md`에도 있다.

## 명령 구분

| 작업 | 셸 요구 |
|------|---------|
| 프론트만 (`npm install`, `npm run dev`, `npm run build`) | **아무 셸이나** 가능 |
| Rust/Tauri (`cargo ...`, `npm run tauri dev|build`) | **MSVC 환경 필수** (아래) |

## Rust/Tauri 빌드 — MSVC 환경 필수 (Windows)

이 머신에서 `cargo`/`tauri` 빌드는 셸 선택이 결과를 좌우한다.

- ❌ **Git Bash(=Bash 도구)로 빌드 금지**: `/usr/bin/link.exe`(coreutils의 `link`)가
  MSVC `link.exe`를 가려 `linking with link.exe failed` 발생.
- ❌ **순수 PowerShell도 실패**: PATH에 `link.exe`가 없어 rustc가 링커를 못 찾음
  (vcvars 자동 로드 안 함).
- ✅ **해결**: 먼저 VS 환경을 초기화한다.
  - "Developer PowerShell for VS 2022" 사용, 또는
  - `cmd /c "call \"<VS설치경로>\VC\Auxiliary\Build\vcvars64.bat\" && cargo ..."`
  - `<VS설치경로>`는 `vswhere -latest -property installationPath`로 확인.
  - vcvars64 이후 `link.exe`/`cl.exe`는 `...\VC\Tools\MSVC\<ver>\bin\Hostx64\x64`로 해석됨.

> VS Community 설치 위치(이 머신, 2026-07 기준 — VS 18로 업그레이드되며 경로가 바뀜):
> `C:\Program Files\Microsoft Visual Studio\18\Community`
> (과거 `...\2022\Community` 경로는 더 이상 없음. 항상 `vswhere -latest`로 확인할 것.)

## Windows SDK

- 한때 **SDK 부재**로 `LIB`가 비어 `LNK1181: cannot open input file kernel32.lib` 발생.
- **Windows 11 SDK 10.0.26100 설치**(2026-06-29)로 해결.
  라이브러리: `C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\{um,ucrt}\x64`.

## 사내망(Samsung SDS) 제약

- **VS Installer 다운로드 실패**: 투명 프록시 뒤에서 `Content-Length is missing from
  response header`(구식 .NET WebClient가 chunked 응답 처리 불가).
- 사내 프록시: `http://70.10.15.10:8080` (npm은 이 프록시 사용; 시스템 WinHTTP/WinINET은 direct).
- ✅ **winget은 동작**(최신 HTTP 스택, `--proxy` 지원). MS 구성요소 설치에 사용.
  예: `winget install --id Microsoft.WindowsSDK.10.0.26100 -e --silent` (관리자 권한/UAC 승인).

## 사전 요구사항 요약

- Node 24+ / npm
- Rust(`rustup`, 타깃 `x86_64-pc-windows-msvc`)
- Visual Studio 2022 + "C++를 사용한 데스크톱 개발" 워크로드 (MSVC `link.exe`/`cl.exe` + SDK)
- WebView2 런타임 (Windows 11 내장)
- `reqwest`(D48)는 `native-tls`로 빌드되어 Windows에서 **schannel**을 쓴다 — OpenSSL 설치 불필요,
  OS 인증서 저장소(사내 프록시 CA 포함)를 그대로 신뢰한다.

## 테스트 실행

```powershell
cargo test --manifest-path src-tauri\Cargo.toml
```

(이 역시 MSVC 환경에서 실행해야 한다. Linux에서는 `libgtk-3-dev`/`libwebkit2gtk-4.1-dev` 설치 시
`cargo test`가 동작한다 — 파서/설정 등 단위 테스트는 플랫폼 무관.)

## Known issue: 첫 질문 시 콘솔 창 깜빡임 (업스트림 CLI 버그)

질문(특히 세션의 첫 턴)에서 cmd/콘솔 창이 잠깐 나타났다 사라질 수 있다. **이 앱의 프로세스 실행은
원인이 아니다** — 모든 spawn(탐지 프로브·실행 엔진·`taskkill`)은 `exec::command_for` 단일 팩토리를
거치며 `CREATE_NO_WINDOW`가 예외 없이 적용된다(D54에서 감사 완료). 깜빡임은 **Claude Code CLI가
세션 시작 시 자기 하위 프로세스(셸 스냅샷 등)를 windowsHide 없이 spawn하는 업스트림 버그**다
(anthropics/claude-code [#14828](https://github.com/anthropics/claude-code/issues/14828) ·
[#15572](https://github.com/anthropics/claude-code/issues/15572) ·
[#16880](https://github.com/anthropics/claude-code/issues/16880) ·
[#61051](https://github.com/anthropics/claude-code/issues/61051) — 버전에 따라 수정·회귀 반복).

- **조치**: Claude Code CLI를 최신 버전으로 업데이트한다(`npm i -g @anthropic-ai/claude-code` 또는
  네이티브 인스톨러). gemini/aipro도 유사 증상이 있으면 각 CLI 업데이트.
- 앱 측 코드 변경은 없다([05](05-decisions.md) D54 — hidden desktop 재작성은 기각).

## CI 릴리즈 워크플로우 (GitHub Actions)

`.github/workflows/release.yml`("Release")은 **Windows 단독 실행파일(exe, 설치파일 아님)**을
빌드해 GitHub Release로 배포하는 수동(`workflow_dispatch`) 워크플로우다.

- **입력**: `version` — `major`/`minor`/`patch` 중 택1(드롭다운).
- **버전 결정**: 최신 `v*.*.*` 태그(없으면 `package.json`의 버전)를 기준으로
  `.github/scripts/bump-version.mjs`가 계산하고, `package.json`/`src-tauri/tauri.conf.json`/
  `src-tauri/Cargo.toml`의 버전 필드를 갱신한다(커밋은 하지 않음 — 태그가 버전의 단일 진실 소스).
- **빌드**: `windows-latest` 러너에서 `npm run tauri build -- --no-bundle`로 **설치파일(NSIS/MSI)
  번들링을 건너뛰고** 컴파일된 바이너리(`src-tauri/target/release/operation-wizard.exe`)만 사용한다.
  GitHub Actions의 `windows-latest` 이미지는 VS Build Tools가 이미 설치·레지스트리에 등록돼 있어
  rustc/link.exe가 자동 인식하므로, 로컬 개발 환경과 달리 vcvars 초기화가 **불필요**하다.
- **배포**: `softprops/action-gh-release`로 새 태그(`vX.Y.Z`)의 GitHub Release를 생성하고
  exe를 첨부한다. 릴리즈 노트는 `generate_release_notes: true`(GitHub 자동 변경사항 요약)로
  본문에 채운다.
- 새 Cargo/npm 의존성 없음(순수 CI 워크플로우).

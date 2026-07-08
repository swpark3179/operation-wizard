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

## 테스트 실행

```powershell
cargo test --manifest-path src-tauri\Cargo.toml
```

(이 역시 MSVC 환경에서 실행해야 한다.)

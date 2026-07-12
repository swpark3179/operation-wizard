# Operation Wizard

A Windows desktop app (Tauri v2 + React + Tailwind) with an Open-Design-style UI.
Its core feature **detects local CLI coding agents** — currently OpenCode,
Claude Code, Codex, Gemini CLI, Antigravity and AI Pro (an in-house,
Gemini-compatible tool). For each it resolves the
executable (PATH + `PATHEXT` + well-known toolchain dirs, or a custom path),
probes `--version`, and lists models (a live `models`/`debug models` probe where
the CLI supports it, otherwise a static fallback catalog).

The detection logic re-implements Open Design's daemon behaviour
(`apps/daemon/src/runtimes/`) in Rust as a "runtime definition (def) + common
probe" registry — see `docs/cli-agent-detection-and-daemon.ko.md` in the
open-design repo, and `docs/design/` here.

## Architecture

- **Rust backend** (`src-tauri/src/`)
  - `agents.rs` — the agent registry: one `AgentDef` per agent (bin candidates,
    `*_BIN` env override, model-listing probe + parser, fallback catalog), ported
    1:1 from Open Design's `runtimes/defs/*.ts`.
  - `resolve.rs` — find an agent's executable: custom path / its `*_BIN` env, then
    PATH + `%APPDATA%\npm`, shared toolchain dirs + def-specific extras (e.g.
    `~/.opencode/bin`), fnm node-versions, scanned with `PATHEXT` (`.EXE;.CMD;.BAT`).
  - `exec.rs` — run a probe with a timeout. `.cmd`/`.bat` shims are launched via
    `cmd.exe /d /s /c` (Open Design's `buildCmdShimInvocation`), with
    `CREATE_NO_WINDOW` so no console flashes.
  - `detect.rs` — the def-driven version + models pipeline → `DetectedAgent`, plus
    the model parsers (line-separated and codex JSON).
  - `settings.rs` — persists per-agent custom paths to `settings.json` in the app config dir.
  - `lib.rs` — Tauri commands: `list_agents`, `detect_agent`, `get_settings`, `set_agent_bin`.
- **React frontend** (`src/`) — Open-Design palette/tokens (`styles/tokens.css`),
  Tailwind v4 `@theme` mapping, a nav-rail + top-bar shell, an Agents view with one
  card per agent (detection status **plus** an inline custom-path editor), and a
  Home → Workspace flow (chat panel + canvas) with category-guided flows and
  bundled skills.

> 프로젝트 전반을 처음부터 이해하려면 [`docs/PROJECT_GUIDE.md`](docs/PROJECT_GUIDE.md)를 참고하세요
> (개념·구조·동작·확장을 한 문서에 정리). 개념 수준의 단일 출처는 [`docs/design/`](docs/design/README.md)입니다.
>
> To enable and use the RAG search + Confluence ingestion features, see
> [`docs/rag-confluence-guide.md`](docs/rag-confluence-guide.md) (fill in the RAG adapter,
> configure Confluence crawling, and troubleshooting — in Korean).

## Prerequisites

- Node 24+ and npm
- Rust (`rustup`, MSVC target `x86_64-pc-windows-msvc`)
- **Visual Studio 2022 with the “Desktop development with C++” workload**
  (provides the MSVC `link.exe` / `cl.exe` and Windows SDK)
- WebView2 runtime (built into Windows 11)

## Running

> **Important — build the Rust side from a VS Developer shell.**
> Plain PowerShell does not expose the MSVC linker, and **Git Bash must not be used**
> for the Rust build: its `/usr/bin/link.exe` (coreutils `link`) shadows MSVC's
> `link.exe` and the build fails. Use **“Developer PowerShell for VS 2022”** (Start
> menu), or initialize the environment first:
>
> ```powershell
> cmd /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && powershell'
> ```

```powershell
npm install

# dev (opens the app window with hot reload)
npm run tauri dev

# production build → installer under src-tauri/target/release/bundle/
npm run tauri build
```

Frontend-only commands (`npm run dev` / `npm run build`) work from any shell;
only the Rust/Tauri steps need the MSVC environment.

## Tests

```powershell
cargo test --manifest-path src-tauri\Cargo.toml
```

Covers model parsing and an end-to-end detection of a stub `opencode.cmd` via a
custom path (resolve → `cmd.exe` wrapping → version/models parsing).

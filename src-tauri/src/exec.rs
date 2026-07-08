//! Run a CLI probe with a timeout, no console flash, and `.cmd`/`.bat`-safe
//! invocation. Mirrors Open Design's `createCommandInvocation` /
//! `buildCmdShimInvocation` (`packages/platform/src/index.ts`).

use std::ffi::OsStr;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

pub struct CaptureResult {
    /// Process exit code (`None` if killed/timed out or signalled).
    pub status_code: Option<i32>,
    pub timed_out: bool,
    /// Set when the process could not even be spawned.
    pub spawn_error: Option<std::io::Error>,
    pub stdout: String,
    /// Captured to drain the stderr pipe (avoids deadlock); kept for diagnostics.
    #[allow(dead_code)]
    pub stderr: String,
}

impl CaptureResult {
    fn spawn_err(e: std::io::Error) -> Self {
        CaptureResult {
            status_code: None,
            timed_out: false,
            spawn_error: Some(e),
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

/// Apply CREATE_NO_WINDOW so probing a `.cmd` shim via cmd.exe never flashes a
/// console window. No-op off Windows.
fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Build a `Command` for `<path> <args...>`, wrapping `.cmd`/`.bat` shims via
/// cmd.exe and applying CREATE_NO_WINDOW. Shared by the one-shot probe
/// (`run_capture`) and the streaming run engine (`run.rs`). Stdio is left to the
/// caller. Callers always pass at least one argument (see the `/s /c` note).
pub fn command_for<S: AsRef<OsStr>>(path: &str, args: &[S]) -> Command {
    let lower = path.to_ascii_lowercase();
    let is_shim = lower.ends_with(".cmd") || lower.ends_with(".bat");

    let mut cmd = if is_shim {
        // Launch cmd.exe (a real .exe) so we sidestep Rust's BatBadBut
        // mitigation, which refuses to pass args to a .bat/.cmd *program*.
        // `/d` skips AutoRun, `/s /c` runs the rest as the command.
        //
        // Rust quotes the spaced path arg ("C:\Users\First Last\..\opencode.cmd").
        // After `/c` the line is `"<path>" --version`: it starts with a quote but
        // does NOT end with one (a trailing arg follows), so cmd.exe's /s
        // "strip outer quotes" rule does not fire and the quoted path is parsed
        // correctly. This is why callers always pass >=1 arg.
        let mut c = Command::new("cmd.exe");
        c.arg("/d").arg("/s").arg("/c").arg(path);
        for a in args {
            c.arg(a);
        }
        c
    } else {
        let mut c = Command::new(path);
        for a in args {
            c.arg(a);
        }
        c
    };

    no_window(&mut cmd);
    cmd
}

/// Run `<path> <args...>`, capturing stdout/stderr, killing the child if it
/// exceeds `timeout`. Callers always pass at least one argument (see the note
/// on the `cmd.exe /s /c` quote-stripping rule in `command_for`).
pub fn run_capture(path: &str, args: &[&str], timeout: Duration) -> CaptureResult {
    let mut cmd = command_for(path, args);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(ch) => ch,
        Err(e) => return CaptureResult::spawn_err(e),
    };

    // Drain stdout/stderr on threads so a large `opencode models` dump (>1MB)
    // can't fill the pipe buffer and deadlock before we wait.
    let mut out = child.stdout.take().expect("piped stdout");
    let mut err = child.stderr.take().expect("piped stderr");
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });

    let (status_code, timed_out) = match child.wait_timeout(timeout) {
        Ok(Some(status)) => (status.code(), false),
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            (None, true)
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            (None, false)
        }
    };

    let stdout = String::from_utf8_lossy(&out_handle.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_handle.join().unwrap_or_default()).into_owned();

    CaptureResult { status_code, timed_out, spawn_error: None, stdout, stderr }
}

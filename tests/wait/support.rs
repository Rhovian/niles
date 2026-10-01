pub(crate) use crate::common::*;
pub(crate) use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

/// Starts a wait and returns the child. Callers that need the wait to already be polling before
/// they perturb the worker use [`settle`] — with the waiter-registration file gone there is no
/// artifact to synchronise on, and the wait has nothing to race against anyway.
pub(crate) fn spawn_wait(workspace: &Path, args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_niles"))
        .arg("wait")
        .args(args)
        .current_dir(workspace)
        .env("NILES_HOME", niles_home(workspace))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

pub(crate) fn run_wait(workspace: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_niles"))
        .arg("wait")
        .args(args)
        .current_dir(workspace)
        .env("NILES_HOME", niles_home(workspace))
        .output()
        .unwrap()
}

pub(crate) fn settle() {
    thread::sleep(Duration::from_millis(300));
}

pub(crate) fn worker_with_status(workspace: &Path, id: &str, status: &[u8]) -> std::path::PathBuf {
    let worker_dir = workspace.join(".niles/worker").join(id);
    fs::create_dir_all(&worker_dir).unwrap();
    fs::write(worker_dir.join("status.log"), status).unwrap();
    worker_dir
}

pub(crate) fn cursor(worker_dir: &Path) -> String {
    fs::read_to_string(worker_dir.join("status.cursor")).unwrap()
}

pub(crate) fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub(crate) fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Writes a worker directory complete enough for task-label selection to find it.
pub(crate) fn write_task_worker(workspace: &Path, id: &str, task_label: &str, status: &[u8]) {
    let worker_dir = worker_with_status(workspace, id, status);
    fs::write(
        worker_dir.join("meta.json"),
        format!(
            r#"{{
  "niles_schema": 2,
  "id": "{id}",
  "agent": "codex",
  "project": "{}",
  "window": "niles:niles-{id}",
  "brief": "{}",
  "launch": "{}",
  "task_label": "{task_label}"
}}
"#,
            workspace.display(),
            worker_dir.join("brief.md").display(),
            worker_dir.join("launch.sh").display(),
        ),
    )
    .unwrap();
}

/// A private tmux server on its own socket, torn down when the test ends.
///
/// These tests assert on tmux window state, and a stub that answers window queries is a second
/// implementation of tmux that has to stay correct as niles's use of it grows — the previous stub
/// claimed to create a window and then reported none existed, which hid a real bug. A per-test
/// socket keeps parallel tests from seeing each other's sessions, and keeps them out of the
/// developer's own tmux.
pub(crate) struct TmuxServer {
    pub(crate) socket: std::path::PathBuf,
    pub(crate) session: String,
}

impl TmuxServer {
    pub(crate) fn start(workspace: &Path, session: &str) -> Self {
        // Not under the workspace: a unix socket path is capped near 104 bytes on macOS, and the
        // temp workspace names are long enough on their own to blow it.
        //
        // Named from a counter, not a timestamp. `SystemTime` is microsecond-resolution here, so
        // two tests starting in the same microsecond got the same socket: the second joined the
        // first's server, and the first's `Drop` then killed it mid-test.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let socket =
            std::path::PathBuf::from(format!("/tmp/nt-{}-{unique}.sock", std::process::id()));
        let server = Self {
            socket,
            session: session.to_owned(),
        };
        // The session runs a long-lived command rather than a shell. A shell that exits takes
        // the last window with it, which destroys the session — and a worker whose whole tmux
        // server has gone is a different state from one whose window has, with different
        // answers from every query after it.
        server.run(&[
            "new-session",
            "-d",
            "-s",
            session,
            "-c",
            &workspace.display().to_string(),
            "sleep 600",
        ]);
        server
    }

    /// The value niles reads from `$TMUX` to find this server.
    pub(crate) fn tmux_env(&self) -> String {
        format!("{},0,0", self.socket.display())
    }

    pub(crate) fn new_window(&self, name: &str) {
        self.new_window_running(name, None);
    }

    /// `command` runs in the window instead of the default shell, for tests that need a pane
    /// which reacts to keystrokes in a particular way.
    pub(crate) fn new_window_running(&self, name: &str, command: Option<&str>) {
        // Explicit index: tmux's auto-indexing collides when base-index != 0 and more than one
        // window is created in a session (it keeps re-choosing the same index). Names are what
        // `wait` matches on, so the index is arbitrary as long as it is unique.
        static NEXT: AtomicU64 = AtomicU64::new(10);
        let index = NEXT.fetch_add(1, Ordering::Relaxed);
        let target = format!("{}:{}", self.session, index);
        let mut args = vec!["new-window", "-d", "-t", &target, "-n", name];
        if let Some(command) = command {
            args.push(command);
        }
        self.run(&args);
    }

    /// Whatever tmux says about this server right now, for failure messages.
    pub(crate) fn diagnostics(&self) -> String {
        let sessions = Command::new("tmux")
            .args(["-S", &self.socket.display().to_string(), "list-sessions"])
            .output()
            .unwrap();
        format!(
            "socket={} exists={} sessions={:?}/{:?} windows={:?}",
            self.socket.display(),
            self.socket.exists(),
            String::from_utf8_lossy(&sessions.stdout),
            String::from_utf8_lossy(&sessions.stderr),
            self.windows()
        )
    }

    pub(crate) fn windows(&self) -> String {
        let output = Command::new("tmux")
            .args(["-S", &self.socket.display().to_string()])
            .args([
                "list-windows",
                "-t",
                &format!("={}", self.session),
                "-F",
                "#{window_name}",
            ])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    pub(crate) fn pane_in_mode(&self, target: &str) -> String {
        let output = Command::new("tmux")
            .args(["-S", &self.socket.display().to_string()])
            .args(["display-message", "-p", "-t", target, "#{pane_in_mode}"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", self.diagnostics());
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    pub(crate) fn run(&self, args: &[&str]) {
        let output = Command::new("tmux")
            .args(["-S", &self.socket.display().to_string()])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "tmux {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for TmuxServer {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-S", &self.socket.display().to_string(), "kill-server"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = fs::remove_file(&self.socket);
    }
}

/// Runs a niles command against `server`, with a stub agent binary on PATH.
pub(crate) fn niles_in(
    server: &TmuxServer,
    workspace: &Path,
    bin: &Path,
    args: &[&str],
) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_niles"));
    command
        .args(args)
        .current_dir(workspace)
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").expect("PATH must be set in the test environment")
            ),
        )
        .env("NILES_HOME", niles_home(workspace))
        .env("TMUX", server.tmux_env());
    command
}

/// Blocks until `path` exists, or fails the test saying it never appeared.
pub(crate) fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("{} never appeared", path.display());
}

pub(crate) fn write_stub_agent(bin: &Path) {
    let codex = bin.join("codex");
    fs::write(&codex, "#!/bin/sh\nsleep 30\n").unwrap();
    let mut permissions = fs::metadata(&codex).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    fs::set_permissions(&codex, permissions).unwrap();
}

pub(crate) fn write_worker_meta(
    workspace: &Path,
    session: &str,
    id: &str,
    task_label: Option<&str>,
) {
    let worker_dir = workspace.join(".niles/worker").join(id);
    let label = match task_label {
        Some(label) => format!(",\n  \"task_label\": \"{label}\""),
        // No task label: the field is omitted from the manifest JSON.
        None => String::new(),
    };
    fs::write(
        worker_dir.join("meta.json"),
        format!(
            r#"{{
  "niles_schema": 2,
  "id": "{id}",
  "agent": "codex",
  "project": "{}",
  "window": "{session}:niles-{id}",
  "brief": "{}",
  "launch": "{}"{label}
}}
"#,
            workspace.display(),
            worker_dir.join("brief.md").display(),
            worker_dir.join("launch.sh").display(),
        ),
    )
    .unwrap();
}

pub(crate) fn remove_dir_all_eventually(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match fs::remove_dir_all(path) {
            Ok(()) => return,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
            Err(_err) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
                if !path.exists() {
                    return;
                }
            }
            Err(err) => panic!("failed to remove {}: {err}", path.display()),
        }
    }
}

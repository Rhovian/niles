pub(crate) use crate::common::*;
pub(crate) use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

/// Starts a wait and returns the child. Callers use [`settle`] when the wait must already be polling.
pub(crate) fn spawn_wait(workspace: &Path, args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_niles"))
        .arg("wait")
        .args(args)
        .current_dir(workspace)
        .env("NILES_HOME", niles_home(workspace))
        .env_remove("TMUX")
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
        .env_remove("TMUX")
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

/// A private tmux server on its own socket, torn down when the test ends.
///
/// A per-test socket keeps parallel tests isolated from each other and the developer's tmux.
pub(crate) struct TmuxServer {
    pub(crate) socket: std::path::PathBuf,
    pub(crate) session: String,
}

impl TmuxServer {
    pub(crate) fn start(workspace: &Path, session: &str) -> Self {
        // Not under the workspace: a unix socket path is capped near 104 bytes on macOS, and the
        // temp workspace names are long enough on their own to blow it.
        //
        // Counter, not timestamp: two tests in the same microsecond shared a socket.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let socket =
            std::path::PathBuf::from(format!("/tmp/nt-{}-{unique}.sock", std::process::id()));
        let server = Self {
            socket,
            session: session.to_owned(),
        };
        // Keep the session alive independently of its worker windows.
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

pub(crate) struct Lab {
    pub(crate) workspace: PathBuf,
    pub(crate) server: TmuxServer,
    bin: PathBuf,
}

impl Lab {
    pub(crate) fn start(prefix: &str) -> Self {
        let workspace = temp_workspace(prefix);
        let bin = workspace.join("bin");
        fs::create_dir_all(&bin).unwrap();
        write_executable(&bin.join("codex"), "#!/bin/sh\nsleep 30\n");
        let server = TmuxServer::start(&workspace, "niles");
        Self {
            workspace,
            server,
            bin,
        }
    }

    pub(crate) fn niles(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_niles"));
        command
            .args(args)
            .current_dir(&self.workspace)
            .env("PATH", path_with_bin(&self.bin))
            .env("NILES_HOME", niles_home(&self.workspace))
            .env("TMUX", self.server.tmux_env());
        command
    }

    pub(crate) fn worker(&self, id: &str, task_label: Option<&str>, status: &[u8]) -> PathBuf {
        write_worker(
            &self.workspace,
            id,
            &format!("{}:niles-{id}", self.server.session),
            task_label,
            status,
        )
    }

    pub(crate) fn worker_window(&self, id: &str, status: &[u8]) -> PathBuf {
        self.server.new_window(&format!("niles-{id}"));
        self.worker(id, None, status)
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = fs::remove_dir_all(&self.workspace);
        }
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

pub(crate) use crate::common::*;
pub(crate) use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

/// The tmux `-t` form of a recorded `session:window`. Niles anchors both
/// components with `=` so tmux cannot prefix-match a neighbouring window.
pub(crate) fn exact_target(window: &str) -> String {
    let (session, name) = window.split_once(':').expect("recorded session:window");
    format!("={session}:={name}")
}
/// A private tmux server on its own socket, torn down when the test ends.
///
/// `worker_close_wakes_waiters_with_nonzero_closed_status` waits on a worker that must have a
/// real tmux window, or `wait`'s window-gone check reports it gone before the close lands. A stub
/// tmux that answers window queries with comfortable lies would hide that, so this drives real
/// tmux on a private socket the developer's own session never sees.
pub(crate) struct TmuxServer {
    pub(crate) socket: PathBuf,
    pub(crate) session: String,
}

impl TmuxServer {
    pub(crate) fn start(workspace: &Path, session: &str) -> Self {
        // Not under the workspace: a unix socket path is capped near 104 bytes on macOS, and the
        // temp workspace names are long enough on their own to blow it.
        //
        // Named from a counter, not a timestamp: two tests starting in the same microsecond got
        // the same socket, and the second joined the first's server, whose `Drop` then killed it
        // mid-test.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let socket = PathBuf::from(format!("/tmp/nt-{}-{unique}.sock", std::process::id()));
        let server = Self {
            socket,
            session: session.to_owned(),
        };
        // A long-lived command rather than a shell: a shell that exits takes the last window with
        // it and destroys the session, which is a different state from a worker whose window has.
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
        // Explicit index: tmux's auto-indexing collides when base-index != 0 and more than one
        // window is created in a session (it keeps re-choosing the same index). Names are what
        // `wait` matches on, so the index is arbitrary as long as it is unique.
        static NEXT: AtomicU64 = AtomicU64::new(10);
        let index = NEXT.fetch_add(1, Ordering::Relaxed);
        self.run(&[
            "new-window",
            "-d",
            "-t",
            &format!("{}:{}", self.session, index),
            "-n",
            name,
        ]);
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
pub(crate) fn write_worker_fixture(workspace: &Path, id: &str, status_body: &str) -> PathBuf {
    write_worker_fixture_with_task(workspace, id, status_body, None)
}

pub(crate) fn write_worker_fixture_with_window(
    workspace: &Path,
    id: &str,
    status_body: &str,
    window: &str,
) -> PathBuf {
    write_worker_fixture_with_task_and_window(workspace, id, status_body, None, window)
}

pub(crate) fn write_worker_fixture_with_task(
    workspace: &Path,
    id: &str,
    status_body: &str,
    task_label: Option<&str>,
) -> PathBuf {
    write_worker_fixture_with_task_and_window(
        workspace,
        id,
        status_body,
        task_label,
        &format!("niles:niles-{id}"),
    )
}

pub(crate) fn write_worker_fixture_with_task_and_window(
    workspace: &Path,
    id: &str,
    status_body: &str,
    task_label: Option<&str>,
    window: &str,
) -> PathBuf {
    let worker_root = workspace.join(".niles/worker");
    let worker_dir = worker_root.join(id);
    fs::create_dir_all(&worker_dir).unwrap();
    let brief = worker_dir.join("brief.md");
    let launch = worker_dir.join("launch.sh");
    let status = worker_dir.join("status.log");
    fs::write(&brief, "brief").unwrap();
    fs::write(&launch, "launch").unwrap();
    fs::write(&status, status_body).unwrap();
    let task_label_field = match task_label {
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
  "window": "{window}",
  "brief": "{}",
  "launch": "{}",
  "status": "{}"{task_label_field}
}}
"#,
            workspace.display(),
            brief.display(),
            launch.display(),
            status.display()
        ),
    )
    .unwrap();
    worker_dir
}

pub(crate) fn write_corrupt_worker_fixture(workspace: &Path, id: &str) -> PathBuf {
    let worker_dir = workspace.join(".niles/worker").join(id);
    fs::create_dir_all(&worker_dir).unwrap();
    fs::write(worker_dir.join("status.log"), "working: bad metadata\n").unwrap();
    fs::write(
        worker_dir.join("meta.json"),
        format!(
            r#"{{
  "id": "{id}",
  "agent": "codex"
}}
"#
        ),
    )
    .unwrap();
    worker_dir
}

pub(crate) fn write_orphan_recovery_tmux(bin: &Path, missing_session: &str) {
    write_executable(
        &bin.join("tmux"),
        &format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'niles-test-session\n'; exit 0 ;;
  list-windows)
    if [ "$2" = "-a" ]; then
      if [ -n "${{TMUX_TAGGED_WINDOWS:-}}" ]; then
        printf '%s\n' "$TMUX_TAGGED_WINDOWS"
      fi
      exit 0
    fi
    if [ "$3" = "={missing_session}" ]; then
      printf "can't find session: {missing_session}\n" >&2
      exit 1
    fi
    exit 0
    ;;
  capture-pane) printf 'final pane\n'; exit 0 ;;
  *) exit 0 ;;
esac
"#
        ),
    );
}

pub(crate) fn write_worker_test_bins(root: &Path) -> (PathBuf, PathBuf) {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tmux_log = root.join("tmux.log");
    write_executable(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'niles-test-session\n'; exit 0 ;;
  has-session) exit 0 ;;
  list-windows)
    if [ "${TMUX_LIST_WINDOWS_FAIL:-}" = 1 ]; then
      printf 'server unreachable\nretry later\n' >&2
      exit 1
    fi
    if [ "$2" = "-a" ]; then
      if [ -n "${TMUX_TAGGED_WINDOWS:-}" ]; then
        printf '%s\n' "$TMUX_TAGGED_WINDOWS"
      fi
      exit 0
    fi
    if [ -n "${TMUX_WINDOWS:-}" ]; then
      printf '%s\n' "$TMUX_WINDOWS"
    fi
    exit 0
    ;;
  capture-pane)
    if [ "${TMUX_CAPTURE_EMPTY:-}" = 1 ]; then
      exit 0
    fi
    printf '%s\n' "${TMUX_CAPTURE:-pane output}"
    exit 0
    ;;
  *) exit 0 ;;
esac
"#,
    );
    write_executable(
        &bin.join("claude"),
        r#"#!/bin/sh
exit 0
"#,
    );
    (bin, tmux_log)
}

pub(crate) fn path_with_bin(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH must be set in the test environment")
    )
}

pub(crate) fn latest_archive_dir(workspace: &Path, id: &str) -> PathBuf {
    let archive_root = workspace.join(".niles/worker/archive");
    let prefix = format!("{id}-");
    let mut archives = fs::read_dir(&archive_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
        })
        .collect::<Vec<_>>();
    archives.sort();
    archives.pop().expect("expected worker archive")
}

pub(crate) fn assert_archived_with_closed_sentinel(workspace: &Path, id: &str) {
    assert!(!workspace.join(".niles/worker").join(id).exists());
    let archive_dir = latest_archive_dir(workspace, id);
    assert!(
        fs::read_to_string(archive_dir.join("status.log"))
            .unwrap()
            .contains(&format!("closed: {id}"))
    );
}

pub(crate) fn assert_global_index_absent(home: &Path) {
    let path = home.join("runs/index.json");
    assert!(!path.exists(), "global index should not exist: {path:?}");
}

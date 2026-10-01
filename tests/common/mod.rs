#![allow(
    dead_code,
    reason = "each integration-test binary uses a different subset of these helpers"
)]

use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

const STUB_TMUX: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'niles-test-session\n'; exit 0 ;;
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
    if [ -n "${TMUX_MISSING_SESSION:-}" ] && [ "$3" = "=${TMUX_MISSING_SESSION}" ]; then
      printf "can't find session: %s\n" "$TMUX_MISSING_SESSION" >&2
      exit 1
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
    if [ -f "$TMUX_PANE_FILE" ]; then
      cat "$TMUX_PANE_FILE"
    fi
    exit 0
    ;;
  send-keys)
    if [ -n "${TMUX_PANE_FILE:-}" ]; then
      if [ "$4" = "-l" ]; then
        printf 'composer: %s\n' "$5" >> "$TMUX_PANE_FILE"
      else
        printf 'submitted\n' >> "$TMUX_PANE_FILE"
      fi
    fi
    exit 0
    ;;
  *) exit 0 ;;
esac
"#;

pub struct TestEnv {
    pub root: PathBuf,
    pub home: PathBuf,
    pub bin: PathBuf,
    pub tmux_log: PathBuf,
}

impl TestEnv {
    pub fn new(prefix: &str) -> Self {
        Self::with_tmux(prefix, STUB_TMUX)
    }

    pub fn with_tmux(prefix: &str, tmux: &str) -> Self {
        let root = fs::canonicalize(temp_workspace(prefix)).unwrap();
        let home = root.join("home");
        let bin = root.join("bin");
        let tmux_log = root.join("tmux.log");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        write_executable(&bin.join("tmux"), tmux);
        for agent in ["claude", "codex"] {
            write_executable(&bin.join(agent), "#!/bin/sh\nexit 0\n");
        }
        Self {
            root,
            home,
            bin,
            tmux_log,
        }
    }

    pub fn niles(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_niles"));
        command
            .args(args)
            .current_dir(cwd)
            .env("PATH", path_with_bin(&self.bin))
            .env("NILES_HOME", &self.home)
            .env("TMUX_LOG", &self.tmux_log)
            .env("TMUX", "/tmp/niles-test-tmux,0,0");
        command
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.niles(&self.root, args).output().unwrap()
    }

    pub fn tmux_log(&self) -> String {
        fs::read_to_string(&self.tmux_log).unwrap()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

pub fn niles_bare(cwd: &Path, home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_niles"));
    command
        .current_dir(cwd)
        .env("NILES_HOME", home)
        .env_remove("TMUX");
    command
}

pub fn path_with_bin(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH must be set in the test environment")
    )
}

pub fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub fn assert_command_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} stdout:\n{}\n{label} stderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn assert_failure_contains(label: &str, output: &Output, needle: &str) {
    assert!(
        !output.status.success(),
        "{label} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        stdout_of(output),
        stderr_of(output)
    );
    assert!(
        stderr_of(output).contains(needle),
        "{label} stderr did not contain {needle:?}:\n{}",
        stderr_of(output)
    );
}

pub fn niles_home(workspace: &Path) -> PathBuf {
    workspace.join(".niles-test-home")
}

pub fn temp_workspace(prefix: &str) -> PathBuf {
    let workspace = std::env::temp_dir().join(format!(
        "{prefix}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&workspace).unwrap();
    workspace
}

pub fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

pub fn write_workspace_manifest(
    workspace: &Path,
    lead: &str,
    worker: &str,
    reviewer: &str,
    security: &str,
) {
    fs::create_dir_all(workspace.join(".niles")).unwrap();
    fs::write(
        workspace.join(".niles/manifest.yaml"),
        format!(
            "lead: {lead}\nworker: {worker}\nreviewer: {reviewer}\nsecurity: {security}\nniles_schema: 2\n"
        ),
    )
    .unwrap();
}

pub fn write_worker(
    workspace: &Path,
    id: &str,
    window: &str,
    task_label: Option<&str>,
    status: &[u8],
) -> PathBuf {
    let worker_dir = workspace.join(".niles/worker").join(id);
    fs::create_dir_all(&worker_dir).unwrap();
    let brief = worker_dir.join("brief.md");
    let launch = worker_dir.join("launch.sh");
    fs::write(&brief, "brief").unwrap();
    fs::write(&launch, "launch").unwrap();
    fs::write(worker_dir.join("status.log"), status).unwrap();
    let task_label = match task_label {
        Some(label) => format!(",\n  \"task_label\": \"{label}\""),
        None => String::new(),
    };
    fs::write(
        worker_dir.join("meta.json"),
        format!(
            r#"{{
  "niles_schema": 2,
  "id": "{id}",
  "role": "worker",
  "agent": "codex",
  "created_at": "2026-01-02T03:04:05Z",
  "project": "{}",
  "window": "{window}",
  "brief": "{}",
  "launch": "{}"{task_label}
}}
"#,
            workspace.display(),
            brief.display(),
            launch.display(),
        ),
    )
    .unwrap();
    worker_dir
}

pub fn append_status(path: &Path, bytes: &[u8]) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(bytes).unwrap();
}

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
};

const TEST_ENV_MARKER: &str = ".niles-test-env";

const STUB_TMUX: &str = r#"#!/bin/sh
[ "$1" != -u ] || shift
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message)
    if [ "$5" = '#{pane_dead}' ]; then
      if [ "${TMUX_PANE_MISSING:-}" = 1 ]; then printf "can't find pane\n" >&2; exit 1; fi
      if [ "${TMUX_PANE_EMPTY:-}" = 1 ]; then printf '\n'; exit 0; fi
      if [ "${TMUX_PANE_UNCERTAIN:-}" = 1 ]; then printf "server unreachable\n" >&2; exit 1; fi
      printf '%s\n' "${TMUX_PANE_DEAD:-0}"; exit 0
    fi
    if [ "$3" = '#{client_width} #{client_height}' ]; then printf '160 45\n'; exit 0; fi
    if [ "$3" = "-t" ]; then printf '%s\n' "${TMUX_PROJECT_TAG:-}"; else printf 'niles-test-session\n'; fi
    exit 0 ;;
  display) printf '%s\n' "${TMUX_PROJECT_TAG:-}"; exit 0 ;;
  has-session)
    if [ "${TMUX_SESSION_EXISTS:-}" = 1 ]; then exit 0; fi
    printf "can't find session\n" >&2; exit 1 ;;
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
    pub fn with_manifest(prefix: &str, agent: &str) -> Self {
        let env = Self::new(prefix);
        write_spawn_manifest(&env.root, agent);
        env
    }

    pub fn new(prefix: &str) -> Self {
        Self::with_tmux(prefix, STUB_TMUX)
    }

    pub fn with_tmux(prefix: &str, tmux: &str) -> Self {
        let root = fs::canonicalize(temp_workspace(prefix)).unwrap();
        let home = root.join("home");
        let bin = root.join("bin");
        let tmux_log = root.join("tmux.log");
        fs::create_dir_all(&home).unwrap();
        copy_skills(&home.join(".niles"));
        fs::write(root.join(TEST_ENV_MARKER), "").unwrap();
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
            .env("HOME", &self.home)
            .env_remove("NILES_SETUP")
            .env("TMUX_LOG", &self.tmux_log)
            .env("TMUX", "/tmp/niles-test-tmux,0,0");
        command
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.niles(&self.root, args).output().unwrap()
    }

    pub fn spawn_command(&self, skill: &str, args: &[&str]) -> Command {
        let mut command = self.niles(&self.root, &["spawn", "--skill", skill]);
        command.args(args);
        command
    }

    pub fn spawn(&self, skill: &str, args: &[&str]) -> Output {
        self.spawn_command(skill, args).output().unwrap()
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
        .env("HOME", home)
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
    let workspace = tempfile::Builder::new()
        .prefix(&format!("{prefix}-"))
        .tempdir()
        .unwrap()
        .keep();
    fs::canonicalize(workspace).unwrap()
}

pub fn read_json(path: impl AsRef<Path>) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

pub fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

const SKILLS: [&str; 6] = [
    "lead", "worker", "reviewer", "security", "design", "research",
];

pub fn write_workspace_manifest(
    workspace: &Path,
    lead: &str,
    worker: &str,
    reviewer: &str,
    security: &str,
) {
    write_manifest(
        workspace,
        SKILLS
            .into_iter()
            .zip([lead, worker, reviewer, security, "claude", "claude"]),
    );
}

pub fn write_spawn_manifest(workspace: &Path, agent: &str) {
    write_manifest(workspace, SKILLS.map(|skill| (skill, agent)));
}

pub fn write_manifest<'a>(
    workspace: &Path,
    bindings: impl IntoIterator<Item = (&'a str, &'a str)>,
) {
    let state = state_dir(workspace);
    copy_skills(state.parent().unwrap().parent().unwrap());
    fs::create_dir_all(&state).unwrap();
    let body = bindings
        .into_iter()
        .map(|(skill, agent)| {
            let exact = match agent {
                "claude" => "claude:sonnet:high",
                "codex" => "codex:gpt-6.1-sol:medium",
                _ => agent,
            };
            format!("{skill}:\n  standard: {{primary: {exact}, secondaries: []}}\n")
        })
        .collect::<String>();
    fs::write(
        state_dir(workspace).join("manifest.yaml"),
        format!("entry: lead\n{body}"),
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
    let worker_dir = state_dir(workspace).join("worker").join(id);
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
  "id": "{id}",
  "skill": "worker",
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

// Match the explicit HOME used by subprocesses, including secondary workspaces in a TestEnv.
pub fn state_dir(workspace: &Path) -> PathBuf {
    let home = workspace
        .ancestors()
        .find_map(|ancestor| {
            ancestor
                .join(TEST_ENV_MARKER)
                .is_file()
                .then(|| ancestor.join("home"))
        })
        .unwrap_or_else(|| niles_home(workspace));
    state_dir_at(&home, workspace)
}

pub fn state_dir_at(home: &Path, workspace: &Path) -> PathBuf {
    home.join(".niles/state")
        .join(workspace.to_str().unwrap().replace('/', "-"))
}

pub fn setup_manifest_body() -> &'static str {
    include_str!("../../skills/niles-setup/SKILL.md")
        .split_once("```yaml\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0
}

pub fn copy_skills(root: &Path) {
    for entry in fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("skills")).unwrap() {
        let entry = entry.unwrap();
        if !entry.file_type().unwrap().is_dir() {
            continue;
        }
        let source = entry.path();
        let dest = root.join("skills").join(source.file_name().unwrap());
        fs::create_dir_all(&dest).unwrap();
        let document = dest.join("SKILL.md");
        if !document.try_exists().unwrap() && !dest.join("SKILL.ref").try_exists().unwrap() {
            fs::copy(source.join("SKILL.md"), document).unwrap();
        }
    }
}

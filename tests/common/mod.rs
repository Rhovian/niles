#![allow(
    dead_code,
    reason = "each integration-test binary uses a different subset of these helpers"
)]
use std::collections::HashSet;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::{env, fs, thread};

#[path = "../../src/test_support/process.rs"]
mod process;
pub use process::*;

#[path = "../../src/store/publication.rs"]
mod publication;
pub(crate) use publication::atomic_write;

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
  display) if [ "$2" = -pt ]; then printf '/tmp/test-socket:%%1\n'; [ "${SEND_STARTED:-}" != 1 ] || echo started > "$STARTED"; exit 0; fi; printf '%s\n' "${TMUX_PROJECT_TAG:-}"; exit 0 ;;
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
  new-window)
    [ -z "${TMUX_NEW_WINDOW_HOOK:-}" ] || sh -c "$TMUX_NEW_WINDOW_HOOK"
    if [ "${TMUX_FAIL_NEW_WINDOW:-}" = 1 ]; then
      printf 'create window failed: index 1 in use\n' >&2
      exit 1
    fi
    if [ -n "${META_PATH:-}" ]; then mkdir -p "$META_PATH"; fi
    exit 0 ;;
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
    [ -z "${TMUX_SEND_HOOK:-}" ] || sh -c "$TMUX_SEND_HOOK"
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
        let root = temp_workspace(prefix);
        let home = root.join("home");
        let bin = root.join("bin");
        let tmux_log = root.join("tmux.log");
        fs::create_dir_all(&home).unwrap();
        copy_skills(&home.join(".niles"));
        atomic_write(root.join(TEST_ENV_MARKER), "").unwrap();
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
        if !thread::panicking() {
            fs::remove_dir_all(&self.root).unwrap();
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

pub fn run_bare(cwd: &Path, args: &[&str]) -> Output {
    niles_bare(cwd, &fixture_home(cwd))
        .args(args)
        .output()
        .unwrap()
}

pub const GIT_ENV: [(&str, &str); 12] = [
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_AUTHOR_NAME", "Niles Test"),
    ("GIT_AUTHOR_EMAIL", "test@example.com"),
    ("GIT_COMMITTER_NAME", "Niles Test"),
    ("GIT_COMMITTER_EMAIL", "test@example.com"),
    ("GIT_CONFIG_COUNT", "2"),
    ("GIT_CONFIG_KEY_0", "commit.gpgsign"),
    ("GIT_CONFIG_VALUE_0", "false"),
    ("GIT_CONFIG_KEY_1", "core.hooksPath"),
    ("GIT_CONFIG_VALUE_1", "/dev/null"),
    ("GIT_TERMINAL_PROMPT", "0"),
];

pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .envs(GIT_ENV)
        .output()
        .unwrap();
    assert_command_success(&format!("git {args:?}"), &output);
    stdout_of(&output)
}

pub fn edit_manifest(ws: &Path, edit: impl FnOnce(String) -> String) -> String {
    let path = state_dir(ws).join("manifest.yaml");
    let original = fs::read_to_string(&path).unwrap();
    atomic_write(path, edit(original.clone())).unwrap();
    original
}

pub fn edit_json(path: impl AsRef<Path>, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = path.as_ref();
    let mut value = read_json(path);
    edit(&mut value);
    atomic_write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

pub fn register_project(env: &TestEnv, name: &str, target: &Path) {
    let projects = env.home.join(".niles/projects");
    fs::create_dir_all(&projects).unwrap();
    symlink(target, projects.join(name)).unwrap();
}

pub fn write_skill(root: &Path, name: &str, description: &str, body: &str) -> PathBuf {
    let path = root.join("skills").join(name).join("SKILL.md");
    write_skill_document(&path, name, description, body);
    path
}

pub fn write_skill_document(path: &Path, name: &str, description: &str, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    atomic_write(
        path,
        format!(
            "---\nname: {name}\ndescription: {}\n---\n{body}",
            serde_json::to_string(description).unwrap()
        ),
    )
    .unwrap();
}

pub fn path_with_bin(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        env::var("PATH").expect("PATH must be set in the test environment")
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

pub fn assert_success_contains(label: &str, output: &Output, needle: &str) {
    assert_command_success(label, output);
    assert!(
        stdout_of(output).contains(needle),
        "{label} stdout did not contain {needle:?}:\n{}",
        stdout_of(output)
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
    copy_skills(&fixture_home(workspace).join(".niles"));
    fs::create_dir_all(&state).unwrap();
    let bindings = bindings.into_iter().collect::<Vec<_>>();
    let mut agents = bindings
        .iter()
        .map(|(_, agent)| match *agent {
            "claude" => "claude:sonnet:high",
            "codex" => "codex:gpt-6.1-sol:medium",
            _ => *agent,
        })
        .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    agents.retain(|agent| seen.insert(*agent));
    let (primary, secondaries) = agents.split_first().unwrap();
    let composition = bindings
        .iter()
        .map(|(skill, _)| format!("{skill}: {{}}\n"))
        .collect::<String>();
    atomic_write(state.join("manifest.yaml"), format!(
        "entry: lead\nentry_agent: {primary}\ntiers:\n  standard: {{primary: {primary}, secondaries: [{}]}}\n{composition}", secondaries.join(", ")
    )).unwrap();
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
    atomic_write(&brief, "brief").unwrap();
    atomic_write(&launch, "launch").unwrap();
    atomic_write(worker_dir.join("status.log"), status).unwrap();
    let mut meta = serde_json::json!({
        "id": id, "skill": "worker", "agent": "codex",
        "created_at": "2026-01-02T03:04:05Z", "project": workspace,
        "window": window, "brief": brief, "launch": launch,
    });
    if let Some(label) = task_label {
        meta.as_object_mut()
            .unwrap()
            .insert("task_label".into(), label.into());
    }
    atomic_write(
        worker_dir.join("meta.json"),
        serde_json::to_vec_pretty(&meta).unwrap(),
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
    state_dir_at(&fixture_home(workspace), workspace)
}

fn fixture_home(workspace: &Path) -> PathBuf {
    workspace
        .ancestors()
        .find_map(|ancestor| {
            ancestor
                .join(TEST_ENV_MARKER)
                .is_file()
                .then(|| ancestor.join("home"))
        })
        .unwrap_or_else(|| niles_home(workspace))
}

pub fn state_dir_at(home: &Path, workspace: &Path) -> PathBuf {
    use sha2::{Digest, Sha256};
    home.join(".niles/state")
        .join(format!(
            "{:x}",
            Sha256::digest(workspace.to_str().unwrap().as_bytes())
        ))
        .join("@state")
}

pub fn setup_manifest_body() -> &'static str {
    include_str!("../../src/test_support/manifest.yaml")
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

#[cfg(feature = "remote")]
pub fn stub_tailscale(env: &TestEnv) {
    write_executable(
        &env.bin.join("tailscale"),
        r#"#!/bin/sh
case "$*" in
  'ip -4') echo 127.0.0.1 ;;
  'whois --json --proto=tcp 127.0.0.1:'*)
    if [ -f "$HOME/whois-fail" ]; then echo 'daemon unavailable' >&2; exit 1; fi
    if [ -f "$HOME/whois-empty" ]; then echo '{"Node":{"StableID":""}}'; exit 0; fi
    if [ -f "$HOME/whois-bad" ]; then echo '{}'; exit 0; fi
    if [ -f "$HOME/whois-other" ]; then echo '{"Node":{"StableID":"other"}}';
    else echo '{"Node":{"StableID":"relay"},"UserProfile":{"LoginName":"relay-owner"}}'; fi ;;
  *) echo "unexpected tailscale arguments: $*" >&2; exit 1 ;;
esac
"#,
    );
}

pub fn reserve_project(home: &Path, workspace: &Path) -> fs::File {
    use sha2::{Digest, Sha256};
    let root = home.join(".niles/locks");
    fs::create_dir_all(&root).unwrap();
    let key = serde_json::to_vec(&serde_json::json!({"Project": workspace})).unwrap();
    let file = fs::File::create(root.join(format!("{:x}", Sha256::digest(key)))).unwrap();
    file.lock().unwrap();
    file
}

include!("../../src/test_support/assertions.rs");
include!("../../src/test_support/json.rs");

pub fn assert_fields(actual: &serde_json::Value, expected: serde_json::Value) {
    for (path, value) in expected.as_object().unwrap() {
        assert_eq!(actual.pointer(path), Some(value), "{path}");
    }
}

use super::support::*;
use std::process::{Command, Stdio};

fn git_output(cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Niles Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Niles Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .output()
        .unwrap()
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = git_output(cwd, args);
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        stderr_of(&output)
    );
    stdout_of(&output)
}

struct Repo {
    env: TestEnv,
    workspace: PathBuf,
    tree: PathBuf,
}

impl Repo {
    fn new(prefix: &str) -> Self {
        let env = TestEnv::new(prefix);
        let bare = env.root.join("remote.git");
        let workspace = env.root.join("workspace");
        git(
            &env.root,
            &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
        );
        git(
            &env.root,
            &["clone", bare.to_str().unwrap(), workspace.to_str().unwrap()],
        );
        fs::write(workspace.join("file"), "base\n").unwrap();
        git(&workspace, &["add", "file"]);
        git(&workspace, &["commit", "-m", "base"]);
        git(&workspace, &["push", "origin", "main"]);
        let tree = env
            .home
            .join(".niles/trees")
            .join(workspace.strip_prefix("/").unwrap())
            .join("fix/x");
        Self {
            env,
            workspace,
            tree,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = self.env.niles(&self.workspace, args);
        command
            .env("HOME", &self.env.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Niles Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Niles Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
            .env("GIT_CONFIG_VALUE_0", "false");
        command
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        self.command(args).output().unwrap()
    }

    fn configure(&self, worktree: &str) {
        write_workspace_manifest(&self.workspace, "claude", "claude", "claude", "claude");
        let path = state_dir(&self.workspace).join("manifest.yaml");
        let body = fs::read_to_string(&path).unwrap();
        fs::write(path, format!("{body}worktree:\n{worktree}\n")).unwrap();
    }

    fn spawn(&self, id: &str, base: bool) {
        // configure() writes exact manifest bindings; the same agent spelling also works without one.
        let mut args = vec![
            "spawn",
            "--role",
            "research",
            id,
            "--agent",
            "claude:sonnet:high",
            "--worktree",
            "fix/x",
        ];
        if base {
            args.extend(["--base", "origin/main"]);
        }
        args.push("Fix");
        assert_command_success("managed spawn", &self.run(&args));
    }
}

#[test]
fn create_join_and_retire_managed_tree() {
    let repo = Repo::new("niles-managed-create");
    repo.spawn("first", true);
    assert_eq!(
        git(&repo.tree, &["branch", "--show-current"]).trim(),
        "fix/x"
    );
    assert!(
        !git_output(&repo.tree, &["rev-parse", "--abbrev-ref", "fix/x@{u}"])
            .status
            .success()
    );
    let workers = repo.run(&["workers"]);
    assert_command_success("workers", &workers);
    assert!(stdout_of(&workers).contains(repo.tree.to_str().unwrap()));
    repo.spawn("second", false);
    let first = repo.run(&["close", "first"]);
    assert_command_success("close first", &first);
    assert!(stdout_of(&first).contains("tree: kept: in use by second"));
    let second = repo.run(&["close", "second"]);
    assert_command_success("close second", &second);
    assert!(stdout_of(&second).contains("tree: removed:"));
    assert!(!repo.tree.exists());
    assert!(git(&repo.workspace, &["branch", "--list", "fix/x"]).contains("fix/x"));
}

#[test]
fn dirty_and_unpushed_trees_are_kept() {
    let repo = Repo::new("niles-managed-keep");
    repo.spawn("dirty", true);
    fs::write(repo.tree.join("untracked"), "change").unwrap();
    let close = repo.run(&["close", "dirty"]);
    assert_command_success("close dirty", &close);
    assert!(stdout_of(&close).contains("tree: kept: 1 uncommitted files"));
    fs::remove_file(repo.tree.join("untracked")).unwrap();
    fs::write(repo.tree.join("file"), "next\n").unwrap();
    git(&repo.tree, &["add", "file"]);
    git(&repo.tree, &["commit", "-m", "unpublished"]);
    repo.spawn("unpushed", false);
    let close = repo.run(&["close", "unpushed"]);
    assert_command_success("close unpushed", &close);
    assert!(stdout_of(&close).contains("tree: kept: 1 commits not on any remote"));
}

#[test]
fn group_close_keeps_tree_result_inside_the_table() {
    let repo = Repo::new("niles-managed-group");
    repo.spawn("group", true);
    let close = repo.run(&["close", "--all"]);
    assert_command_success("close all", &close);
    assert!(stdout_of(&close).contains("  group,tree,removed:"));
    assert!(!repo.tree.exists());
}

#[test]
fn worktree_hook_runs_once_with_env_and_exports_literal_values_to_worker() {
    let repo = Repo::new("niles-worktree-hook");
    let cache = repo.workspace.join("target");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("warm"), "dependency").unwrap();
    repo.configure(&format!(
        "  env:\n    CARGO_TARGET_DIR: '{}'\n    LITERAL: '$(touch injected); spaces'\n    NILES_AGENT_DIR: /wrong-worker\n  setup: |\n    test -f \"$CARGO_TARGET_DIR/warm\"\n    printf '%s' \"$LITERAL\" >> setup-runs\n    pwd > setup-cwd",
        cache.display()
    ));
    repo.spawn("first", true);
    repo.spawn("second", false);
    assert_eq!(
        fs::read_to_string(repo.tree.join("setup-runs")).unwrap(),
        "$(touch injected); spaces"
    );
    assert_eq!(
        fs::read_to_string(repo.tree.join("setup-cwd"))
            .unwrap()
            .trim(),
        repo.tree.to_str().unwrap()
    );
    assert!(!repo.tree.join("injected").exists());
    assert!(!repo.tree.join("target").exists());
    let launch = state_dir(&repo.workspace).join("worker/second/launch.sh");
    let agent = repo.env.bin.join("claude");
    write_executable(
        &agent,
        "#!/bin/sh\nprintf '%s' \"$CARGO_TARGET_DIR\" > worker-cache\nprintf '%s' \"$LITERAL\" > worker-literal\nprintf '%s' \"$NILES_AGENT_DIR\" > worker-agent-dir\n",
    );
    let output = Command::new("sh")
        .arg(launch)
        .current_dir(&repo.tree)
        .env("PATH", path_with_bin(&repo.env.bin))
        .output()
        .unwrap();
    assert_command_success("launch worker", &output);
    assert_eq!(
        fs::read_to_string(repo.tree.join("worker-cache")).unwrap(),
        cache.to_str().unwrap()
    );
    assert_eq!(
        fs::read_to_string(repo.tree.join("worker-literal")).unwrap(),
        "$(touch injected); spaces"
    );
    assert_eq!(
        fs::read_to_string(repo.tree.join("worker-agent-dir")).unwrap(),
        state_dir(&repo.workspace)
            .join("worker/second")
            .to_str()
            .unwrap()
    );
    assert!(!repo.tree.join("injected").exists());
}

#[test]
fn failing_worktree_setup_aborts_spawn_and_removes_new_tree() {
    let repo = Repo::new("niles-worktree-hook-failure");
    repo.configure("  setup: 'touch partial; echo setup-failed >&2; exit 7'");
    let output = repo.run(&[
        "spawn",
        "--role",
        "research",
        "failed",
        "--worktree",
        "fix/x",
        "--base",
        "origin/main",
        "Fix",
    ]);
    assert!(!output.status.success());
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("setup-failed")
            && stderr.contains("worktree setup")
            && stderr.contains("exit status: 7"),
        "{stderr}"
    );
    assert!(!repo.tree.exists());
    assert!(!state_dir(&repo.workspace).join("worker/failed").exists());
    assert!(!repo.env.tmux_log().contains("new-window"));
    repo.configure("  setup: 'touch ready'");
    repo.spawn("retry", false);
    assert!(repo.tree.join("ready").exists());
}

#[test]
fn worktree_configuration_does_not_apply_to_plain_or_explicit_tree_spawns() {
    let repo = Repo::new("niles-worktree-hook-scope");
    repo.configure("  env: {HOOK_ONLY: configured}\n  setup: 'exit 99'");
    for (id, tree) in [("plain", false), ("explicit", true)] {
        let mut args = vec!["spawn", "--role", "research", id, "Fix"];
        if tree {
            args.extend(["--tree", repo.workspace.to_str().unwrap()]);
        }
        assert_command_success(id, &repo.run(&args));
        let launch = fs::read_to_string(
            state_dir(&repo.workspace)
                .join("worker")
                .join(id)
                .join("launch.sh"),
        )
        .unwrap();
        assert!(!launch.contains("HOOK_ONLY"));
    }
}

#[test]
fn worktree_setup_uses_eof_stdin_and_keeps_spawn_stdout_clean() {
    let repo = Repo::new("niles-hook-io");
    repo.configure(
        "  setup: 'if read value; then exit 99; fi; echo setup-out; echo setup-err >&2'",
    );
    let input = repo.env.root.join("input");
    fs::write(&input, "unexpected input\n").unwrap();
    let output = repo
        .command(&[
            "spawn",
            "worker",
            "--worktree",
            "fix/x",
            "--base",
            "origin/main",
            "Fix",
        ])
        .stdin(fs::File::open(input).unwrap())
        .output()
        .unwrap();
    assert_command_success("setup stdin/output", &output);
    assert!(stdout_of(&output).contains("spawned: worker"));
    assert!(!stdout_of(&output).contains("setup-out"));
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("setup-out") && stderr.contains("setup-err"),
        "{stderr}"
    );
}

#[test]
fn worktree_setup_cleanup_failure_retains_error_and_blocks_reuse() {
    let repo = Repo::new("niles-hook-cleanup");
    repo.configure("  setup: 'git worktree lock \"$PWD\"; exit 7'");
    let output = repo.run(&[
        "spawn",
        "failed",
        "--worktree",
        "fix/x",
        "--base",
        "origin/main",
        "Fix",
    ]);
    assert!(!output.status.success());
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("exit status: 7")
            && stderr.contains("also failed to remove new tree")
            && stderr.contains("locked"),
        "{stderr}"
    );
    assert!(repo.tree.exists());
    let retry = repo.run(&["spawn", "retry", "--worktree", "fix/x", "Fix"]);
    assert!(!retry.status.success());
    assert!(stderr_of(&retry).contains("creation/setup is incomplete"));
    assert!(!repo.env.tmux_log().contains("new-window"));
}

#[test]
fn worktree_setup_blocks_concurrent_join_and_reuse_after_interruption() {
    let repo = Repo::new("niles-hook-interrupted");
    let ready = repo.env.root.join("ready");
    let release = repo.env.root.join("release");
    let finished = repo.env.root.join("finished");
    assert!(
        Command::new("mkfifo")
            .args([&ready, &release, &finished])
            .status()
            .unwrap()
            .success()
    );
    repo.configure(&format!(
        "  setup: |\n    echo ready > '{}'\n    read value < '{}'\n    echo finished > '{}'",
        ready.display(),
        release.display(),
        finished.display()
    ));
    let mut creator = repo
        .command(&[
            "spawn",
            "creator",
            "--worktree",
            "fix/x",
            "--base",
            "origin/main",
            "Fix",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // FIFOs synchronize with the real setup process without timing-dependent polling.
    assert_eq!(fs::read_to_string(&ready).unwrap(), "ready\n");
    let join = repo.run(&["spawn", "joiner", "--worktree", "fix/x", "Fix"]);
    assert!(!join.status.success());
    assert!(stderr_of(&join).contains("creation is busy"));
    assert!(!repo.env.tmux_log().contains("new-window"));
    creator.kill().unwrap();
    fs::write(&release, "continue\n").unwrap();
    assert_eq!(fs::read_to_string(&finished).unwrap(), "finished\n");
    assert!(!creator.wait_with_output().unwrap().status.success());
    let retry = repo.run(&["spawn", "retry", "--worktree", "fix/x", "Fix"]);
    assert!(!retry.status.success());
    assert!(stderr_of(&retry).contains("creation/setup is incomplete"));
    assert!(repo.tree.exists());
    assert!(!repo.env.tmux_log().contains("new-window"));
}

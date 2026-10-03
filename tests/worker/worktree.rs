use super::support::*;
use std::process::Command;

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
        let tree = env.root.join("workspace-trees/fix/x");
        Self {
            env,
            workspace,
            tree,
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        self.env
            .niles(&self.workspace, args)
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

    fn spawn(&self, id: &str, base: bool) {
        let mut args = vec!["spawn", id, "--agent", "claude", "--worktree", "fix/x"];
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

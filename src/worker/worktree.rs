use std::{
    fs,
    os::unix::fs::OpenOptionsExt,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{store, workspace_manifest::WorktreeConfig};

use super::meta::read_meta_if_exists;

const DETACHED_HEAD: &str = "(detached HEAD)";

pub enum SpawnTree {
    Path(Utf8PathBuf),
    Worktree {
        branch: String,
        base: Option<String>,
    },
}

fn git(workspace: &Utf8Path, args: &[&str]) -> Result<String> {
    let output = crate::util::git_command()
        .args(args)
        .current_dir(workspace)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }
    String::from_utf8(output.stdout).context("git output is not UTF-8")
}

// Keyed by the full workspace path so two workspaces with the same name never share a root.
fn root(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    let workspace = workspace
        .strip_prefix("/")
        .context("workspace path is not absolute")?;
    let home = crate::util::home_dir()?;
    Ok(home.join(".niles/trees").join(workspace))
}

fn registered_branch(workspace: &Utf8Path, path: &Utf8Path) -> Result<Option<String>> {
    for entry in git(workspace, &["worktree", "list", "--porcelain"])?.split("\n\n") {
        let mut lines = entry.lines();
        if lines.next() == Some(format!("worktree {path}").as_str()) {
            let branch = lines.find_map(|line| line.strip_prefix("branch "));
            return Ok(Some(
                branch.map_or(DETACHED_HEAD, |branch| branch).to_owned(),
            ));
        }
    }
    Ok(None)
}

pub(super) fn create_or_join(
    workspace: &Utf8Path,
    branch: &str,
    base: Option<&str>,
    config: Option<&WorktreeConfig>,
) -> Result<Utf8PathBuf> {
    git(workspace, &["check-ref-format", "--branch", branch])?;
    let state = store::paths::project_dir(workspace)?
        .join("worktrees")
        .join(branch);
    fs::create_dir_all(&state).with_context(|| format!("create worktree state {state}"))?;
    // Git forbids branch components starting with '.', so these files cannot collide with branches.
    let lock_path = state.join(".lock");
    let lock = crate::wait::cursor::open_lock_file(&lock_path)?;
    lock.try_lock()
        .with_context(|| format!("worktree {branch} creation is busy; cannot lock {lock_path}"))?;
    let pending = state.join(".pending");
    // Rust 1.89 File::try_lock provides exclusion, but releases on death, not completion:
    // https://doc.rust-lang.org/1.89.0/std/fs/struct.File.html#method.try_lock
    if pending
        .try_exists()
        .with_context(|| format!("inspect {pending}"))?
    {
        bail!(
            "worktree {branch} creation/setup is incomplete; stop any setup process, remove the incomplete worktree, then remove {pending} before recreating it"
        );
    }
    let path = root(workspace)?.join(branch);
    let wanted = format!("refs/heads/{branch}");
    match registered_branch(workspace, &path)? {
        Some(found) if found == wanted => {
            if base.is_some() {
                bail!("tree for {branch} exists at {path}; omit --base to join it");
            }
            Ok(path)
        }
        Some(found) => bail!("tree at {path} is on {found}, not {wanted}"),
        None => {
            let mut args = vec!["worktree", "add"];
            if let Some(base) = base {
                args.extend(["--no-track", "-b", branch, path.as_str(), base]);
            } else {
                args.extend([path.as_str(), branch]);
            }
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&pending)
                .with_context(|| format!("record unfinished worktree creation in {pending}"))?;
            git(workspace, &args)?;
            if let Some(config) = config
                && let Some(command) = &config.setup
                && let Err(error) = setup(&path, command, config)
            {
                // A failed creation must not become a reusable tree whose setup never finished.
                git(workspace, &["worktree", "remove", "--force", path.as_str()])
                    .with_context(|| format!("{error:#}; also failed to remove new tree {path}"))?;
                fs::remove_file(&pending)
                    .with_context(|| format!("{error:#}; failed to clear {pending}"))?;
                return Err(error);
            }
            fs::remove_file(&pending)
                .with_context(|| format!("complete worktree creation: remove {pending}"))?;
            Ok(path)
        }
    }
}

fn setup(path: &Utf8Path, command: &str, config: &WorktreeConfig) -> Result<()> {
    let status = Command::new("sh")
        .args(["-c", command])
        .current_dir(path)
        .envs(&config.env)
        .stdin(Stdio::null())
        .stdout(Stdio::from(std::io::stderr()))
        .status()
        .with_context(|| format!("run worktree setup in {path}"))?;
    if !status.success() {
        bail!("worktree setup in {path} failed: {status}");
    }
    Ok(())
}

pub(super) fn retire(workspace: &Utf8Path, tree: &Utf8Path) -> Result<Option<String>> {
    if !tree.starts_with(root(workspace)?) || registered_branch(workspace, tree)?.is_none() {
        return Ok(None);
    }
    for entry in store::worker_locations(workspace)? {
        if read_meta_if_exists(&entry.worker_dir)?
            .is_some_and(|meta| meta.tree.as_deref() == Some(tree))
        {
            return Ok(Some(format!("kept: in use by {}", entry.id)));
        }
    }
    let dirty = git(tree, &["status", "--porcelain"])?.lines().count();
    if dirty > 0 {
        return Ok(Some(format!("kept: {dirty} uncommitted files")));
    }
    let commits = git(tree, &["rev-list", "HEAD", "--not", "--remotes"])?
        .lines()
        .count();
    if commits > 0 {
        return Ok(Some(format!("kept: {commits} commits not on any remote")));
    }
    git(workspace, &["worktree", "remove", tree.as_str()])?;
    Ok(Some(format!("removed: {tree}")))
}

/// Use Git to remove only this repository's managed trees, including dirty trees.
pub(crate) fn remove_project_worktrees(workspace: &Utf8Path) -> Result<()> {
    let root = root(workspace)?;
    if !root.try_exists()? {
        return Ok(());
    }
    let root = crate::util::utf8_path(fs::canonicalize(root)?, "managed worktree root")?;
    // Git's -z porcelain format preserves paths containing newlines or quoting characters:
    // https://git-scm.com/docs/git-worktree#_porcelain_format
    for path in git(workspace, &["worktree", "list", "--porcelain", "-z"])?
        .split('\0')
        .filter_map(|field| field.strip_prefix("worktree "))
    {
        let path = Utf8Path::new(path);
        if path != workspace && path.starts_with(&root) {
            git(workspace, &["worktree", "remove", "--force", path.as_str()])?;
        }
    }
    Ok(())
}

use std::process::Command;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::store;

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
    let output = Command::new("git")
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

fn root(workspace: &Utf8Path) -> Result<Utf8PathBuf> {
    let parent = workspace.parent().context("workspace has no parent")?;
    let name = workspace
        .file_name()
        .context("workspace has no directory name")?;
    Ok(parent.join(format!("{name}-trees")))
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
) -> Result<Utf8PathBuf> {
    git(workspace, &["check-ref-format", "--branch", branch])?;
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
            git(workspace, &args)?;
            Ok(path)
        }
    }
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

use super::reference::{GithubRef, PinnedRef};
use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use std::{collections::BTreeMap, fs, process::Command};

const MAX_DIRECTORY_BYTES: u64 = 8 * 1024 * 1024;

fn git(dir: &Utf8Path, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    // Git 2.50.1 Documentation/githooks.adoc: repository-local environment
    // overrides cwd. Clear repository paths, retaining operator credential config.
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_GRAFT_FILE",
        "GIT_SHALLOW_FILE",
        "GIT_PREFIX",
        "GIT_INTERNAL_SUPER_PREFIX",
    ] {
        command.env_remove(name);
    }
    let output = command
        .args([
            "--literal-pathspecs",
            "-c",
            "core.symlinks=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .context("failed to run git for skill import")?;
    ensure!(
        output.status.success(),
        "skill git {} failed: {}",
        args[0],
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).context("skill git output is not UTF-8")
}
fn url(reference: &GithubRef) -> String {
    format!(
        "https://github.com/{}/{}.git",
        reference.owner, reference.repo
    )
}

pub(crate) fn pin(reference: GithubRef) -> Result<PinnedRef> {
    if reference.is_pinned() {
        return reference.try_into();
    }
    let cwd = crate::util::current_dir_utf8()?;
    let rev = &reference.rev;
    let output = git(
        &cwd,
        &[
            "ls-remote",
            "--",
            &url(&reference),
            rev,
            &format!("refs/heads/{rev}"),
            &format!("refs/tags/{rev}"),
            &format!("refs/tags/{rev}^{{}}"),
        ],
    )?;
    // Git 2.50.1 Documentation/git-ls-remote.adoc emits a tag and its ^{} peel
    // separately. It does not choose between a same-named branch and tag for us.
    let mut refs = BTreeMap::new();
    for line in output.lines() {
        let (sha, name) = line
            .split_once('\t')
            .context("invalid git ls-remote output")?;
        let expected = if rev == "HEAD" || rev.starts_with("refs/") {
            name == rev || name == format!("{rev}^{{}}")
        } else {
            name == format!("refs/heads/{rev}")
                || name == format!("refs/tags/{rev}")
                || name == format!("refs/tags/{rev}^{{}}")
        };
        if expected {
            refs.insert(name, sha);
        }
    }
    let candidates: Vec<_> = refs
        .iter()
        .filter(|(name, _)| !name.ends_with("^{}"))
        .collect();
    ensure!(
        candidates.len() == 1,
        "skill revision {rev:?} is missing or ambiguous"
    );
    let (name, sha) = candidates[0];
    let peeled = format!("{name}^{{}}");
    let sha = refs.get(peeled.as_str()).copied().map_or(*sha, |sha| sha);
    reference.with_sha(sha.to_owned())
}

pub(super) fn directory(reference: &PinnedRef, global: &Utf8Path) -> Utf8PathBuf {
    let reference = reference.reference();
    global
        .join("cache/skills/github.com")
        .join(&reference.owner)
        .join(&reference.repo)
        .join(&reference.rev)
        .join(&reference.path)
        // The ref grammar reserves @ for revisions. This leaf cannot collide
        // with any remote path, so ancestor and nested skills have distinct keys.
        .join("@skill")
}

pub(super) fn fetch(reference: &PinnedRef, global: &Utf8Path) -> Result<Utf8PathBuf> {
    let spec = reference.reference();
    let root = global
        .join("cache/skills/github.com")
        .join(&spec.owner)
        .join(&spec.repo);
    let dir = directory(reference, global);
    if !dir.join("SKILL.md").try_exists()? {
        fs::create_dir_all(&root)?;
        let staging = root.join(format!(".fetch-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&staging)?;
        let result = (|| {
            let checkout = staging.join("checkout");
            fs::create_dir(&checkout)?;
            git(&checkout, &["init", "-q"])?;
            git(
                &checkout,
                &[
                    "fetch",
                    "--depth",
                    "1",
                    "--filter=blob:none",
                    "--",
                    &url(spec),
                    &spec.rev,
                ],
            )?;
            if !spec.path.is_empty() {
                let tree = git(&checkout, &["ls-tree", "FETCH_HEAD", "--", &spec.path])?;
                ensure!(
                    tree.starts_with("040000 tree "),
                    "import path must be a Git directory: {}",
                    spec.path
                );
            }
            // Git 2.50.1 Documentation/git-sparse-checkout.adoc uses gitignore
            // patterns in non-cone mode, not literal directory paths; quote glob syntax.
            let mut pattern = String::from("/");
            for ch in spec.path.chars() {
                if "\\*?[]!# ".contains(ch) {
                    pattern.push('\\');
                }
                pattern.push(ch);
            }
            if spec.path.is_empty() {
                pattern.push('*');
            } else {
                pattern.push('/');
            }
            git(
                &checkout,
                &["sparse-checkout", "set", "--no-cone", "--", &pattern],
            )?;
            git(&checkout, &["checkout", "-q", "--detach", "FETCH_HEAD"])?;
            fs::remove_dir_all(checkout.join(".git"))?;
            check_size(&checkout.join(&spec.path))?;
            fs::create_dir_all(dir.parent().context("cache has no parent")?)?;
            install(&checkout.join(&spec.path), &dir)?;
            Ok(())
        })();
        finish_fetch(&staging, result)?;
    }
    check_size(&dir)?;
    Ok(dir)
}

fn finish_fetch(staging: &Utf8Path, result: Result<()>) -> Result<()> {
    let cleanup = fs::remove_dir_all(staging)
        .with_context(|| format!("failed to clean skill staging directory {staging}"));
    match (result, cleanup) {
        (Err(fetch), Err(cleanup)) => Err(fetch.context(format!("{cleanup:#}"))),
        (result, Ok(())) => result,
        (Ok(()), Err(cleanup)) => Err(cleanup),
    }
}

fn check_size(root: &Utf8Path) -> Result<()> {
    let mut total = 0;
    let mut pending = vec![root.to_owned()];
    // std::fs::read_dir (Rust 1.89 std/fs.rs) is nonrecursive and has no byte limit.
    // Sum regular-file sizes before publishing or reading the immutable cache tree.
    while let Some(dir) = pending.pop() {
        ensure!(
            fs::symlink_metadata(&dir)?.is_dir(),
            "skill cache path must be a directory: {dir}"
        );
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = Utf8PathBuf::from_path_buf(entry.path())
                .map_err(|_| anyhow::anyhow!("skill cache path is not UTF-8"))?;
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                pending.push(path);
            } else {
                ensure!(
                    metadata.is_file(),
                    "skill cache file must be regular: {path}"
                );
                total += metadata.len();
                ensure!(
                    total <= MAX_DIRECTORY_BYTES,
                    "skill directory exceeds 8 MiB: {root}"
                );
            }
        }
    }
    Ok(())
}

// Rust 1.89 std/fs.rs: rename is atomic but rejects a populated target
// directory. Only a completed competing entry can satisfy that race.
fn install(source: &Utf8Path, target: &Utf8Path) -> Result<()> {
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(error) => {
            if super::files::exists(&target.join("SKILL.md"), super::Origin::Workspace)? {
                Ok(())
            } else {
                Err(error).with_context(|| format!(
                    "failed to install skill cache {target}; remove incomplete cache entry before retrying"
                ))
            }
        }
    }
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;

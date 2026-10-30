use std::{fs, os::unix::fs::symlink};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    tmux::SessionName,
    util::{read_dir_utf8_paths, utf8_path},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectName(String);

impl ProjectName {
    pub fn parse(name: &str) -> Result<Self> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            bail!("project name must use ASCII letters, digits, _ or -");
        }
        Ok(Self(name.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn session(&self) -> Result<SessionName> {
        SessionName::new(self.0.clone())
    }
}

#[derive(Clone)]
pub(super) struct Entry {
    pub name: ProjectName,
    pub path: Utf8PathBuf,
}

impl Entry {
    pub(super) fn has_live_session(&self) -> Result<bool> {
        let session = self.name.session()?;
        Ok(
            crate::tmux::project_session(&session)?.as_deref() == Some(self.path.as_str())
                && crate::tmux::lead_running(&session)?,
        )
    }
}

pub(super) fn directory() -> Result<Utf8PathBuf> {
    Ok(crate::util::home_dir()?.join(".niles/projects"))
}

pub(super) fn entries() -> Result<Vec<Entry>> {
    read_dir_utf8_paths(&directory()?)?
        .into_iter()
        .filter(|link| !link.file_name().is_some_and(|name| name.starts_with('.')))
        .map(|link| {
            let name = ProjectName::parse(link.file_name().context("registry entry has no name")?)?;
            let path = utf8_path(
                fs::read_link(&link).with_context(|| format!("invalid registry link {link}"))?,
                "registry target",
            )?;
            if !path.is_absolute() {
                bail!("registry target for {} is not absolute", name.as_str());
            }
            Ok(Entry { name, path })
        })
        .collect()
}

pub(super) fn register(name: &ProjectName, path: &Utf8Path) -> std::io::Result<()> {
    let dir = directory().map_err(std::io::Error::other)?;
    fs::create_dir_all(&dir)?;
    symlink(path, dir.join(name.as_str()))
}

pub(super) fn find(name: &str) -> Result<Option<Entry>> {
    Ok(entries()?
        .into_iter()
        .find(|entry| entry.name.as_str() == name))
}

/// Remove orchestration data only after checking the current registration and live workers.
pub(super) fn deregister(entry: &Entry) -> Result<()> {
    let link = directory()?.join(entry.name.as_str());
    if fs::read_link(&link)? != entry.path.as_std_path() {
        bail!("registration changed; select the project again");
    }
    if !entry.path.is_absolute()
        || entry.path.parent().is_none()
        || entry.path.components().any(|part| {
            matches!(
                part,
                camino::Utf8Component::ParentDir | camino::Utf8Component::CurDir
            )
        })
    {
        bail!("unsafe project path: {}", entry.path);
    }
    if !crate::worker::live_worker_locations(&entry.path)?.is_empty() {
        bail!(
            "cannot deregister {}: close its live workers first",
            entry.name.as_str()
        );
    }
    let session = entry.name.session()?;
    if crate::tmux::project_session(&session)?.as_deref() == Some(entry.path.as_str()) {
        crate::tmux::kill_session(&session)?;
    }
    crate::worker::remove_project_worktrees(&entry.path)?;
    crate::util::remove_dir_all_if_exists(&crate::store::paths::project_dir(&entry.path)?)?;
    fs::remove_file(&link).with_context(|| format!("remove registration {link}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_fit_tmux_sessions() {
        assert!(ProjectName::parse("api_2-prod").is_ok());
        for name in ["", "a.b", "two words", "é"] {
            assert!(ProjectName::parse(name).is_err());
        }
    }
    #[test]
    fn deregister_refuses_live_workers_then_deletes_state_and_dirty_managed_trees() {
        use crate::{
            store,
            test_support::{remove_workspace, temp_test_path},
        };
        let project = temp_test_path("deregister");
        fs::create_dir_all(&project).unwrap();
        let git = |args: &[&str]| {
            let output = crate::util::git_command()
                .current_dir(&project)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ]);
        let trees = crate::util::home_dir()
            .unwrap()
            .join(".niles/trees")
            .join(project.strip_prefix("/").unwrap());
        let tree = trees.join("managed");
        let external = temp_test_path("external-tree");
        git(&["worktree", "add", "-b", "managed", tree.as_str()]);
        git(&["worktree", "add", "-b", "external", external.as_str()]);
        fs::write(tree.join("dirty"), "uncommitted").unwrap();
        let name = ProjectName::parse(project.file_name().unwrap()).unwrap();
        register(&name, &project).unwrap();
        let entry = Entry {
            name,
            path: project.clone(),
        };
        let state = store::paths::project_dir(&project).unwrap();
        assert!(state.starts_with(crate::util::home_dir().unwrap().join(".niles/state/v2")));
        assert_eq!(state.file_name(), Some("@state"));
        let dir = store::workers_dir(&project).unwrap().join("w");
        fs::create_dir_all(&dir).unwrap();
        store::write_json(&dir.join("meta.json"), &serde_json::json!({
            "id":"w", "skill":"worker", "agent":"codex", "created_at":"2026-01-01T00:00:00Z",
            "project":project, "window":"api:w", "brief":dir.join("brief.md"), "launch":dir.join("launch.sh")
        })).unwrap();
        let link = directory().unwrap().join(entry.name.as_str());
        let error = deregister(&entry).unwrap_err().to_string();
        assert!(error.contains("live workers"), "{error}");
        assert!(link.exists() && tree.exists() && dir.exists());
        fs::remove_file(dir.join("meta.json")).unwrap();
        deregister(&entry).unwrap();
        assert!(link.symlink_metadata().is_err());
        assert!(!state.exists());
        assert!(!tree.exists());
        assert!(project.join(".git").exists() && external.exists());
        let listing = crate::util::git_command()
            .current_dir(&project)
            .args(["worktree", "list", "--porcelain"])
            .output()
            .unwrap();
        assert!(
            !String::from_utf8(listing.stdout)
                .unwrap()
                .contains(tree.as_str())
        );
        git(&["worktree", "remove", external.as_str()]);
        fs::remove_dir_all(trees).unwrap();
        remove_workspace(&project).unwrap();
    }

    #[test]
    fn deregister_missing_project_and_refuse_changed_registration() {
        let path = crate::test_support::temp_test_path("missing-project");
        let name = ProjectName::parse(path.file_name().unwrap()).unwrap();
        register(&name, &path).unwrap();
        let mut entry = Entry { name, path };
        entry.path.push("changed");
        assert!(
            deregister(&entry)
                .unwrap_err()
                .to_string()
                .contains("registration changed")
        );
        entry.path.pop();
        deregister(&entry).unwrap();
        assert!(find(entry.name.as_str()).unwrap().is_none());
    }
}

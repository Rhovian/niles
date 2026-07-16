use std::{
    fs,
    io::{ErrorKind, Write},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::{
    schema::{self, ArtifactKind},
    tmux::{SessionName, WindowTarget},
    util::{absolute_path, slugify},
};

use super::read_latest_manager_session;

const POINTER_FILE: &str = "tmux-session.json";

#[derive(Debug, Serialize, Deserialize)]
struct WorkspaceTmuxSession {
    session: String,
    created_at: DateTime<Utc>,
}

pub(crate) fn resolve_workspace_tmux_session(workspace: &Utf8Path) -> Result<SessionName> {
    let workspace = absolute_path(workspace)?;
    if let Some(pointer) = read_pointer(&workspace)? {
        return pointer.session_name();
    }

    let session = match manager_session(&workspace)? {
        Some(session) => session,
        None => deterministic_session(&workspace)?,
    };
    write_pointer_first_writer_wins(&workspace, session)
}

fn manager_session(workspace: &Utf8Path) -> Result<Option<SessionName>> {
    let Some(meta) = read_latest_manager_session(workspace)? else {
        return Ok(None);
    };
    let Some(window) = meta.window else {
        return Ok(None);
    };
    if window.is_empty() {
        return Ok(None);
    }
    let target = WindowTarget::parse(&window)
        .with_context(|| format!("latest manager session window is invalid: {window}"))?;
    Ok(Some(target.session().clone()))
}

fn deterministic_session(workspace: &Utf8Path) -> Result<SessionName> {
    let workspace_slug = workspace
        .file_name()
        .map(slugify)
        .unwrap_or_else(|| slugify(workspace.as_str()));
    SessionName::new(format!(
        "niles-{workspace_slug}-{}",
        deterministic_tail(workspace)
    ))
}

fn deterministic_tail(workspace: &Utf8Path) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in workspace.as_str().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

fn read_pointer(workspace: &Utf8Path) -> Result<Option<WorkspaceTmuxSession>> {
    schema::read_optional_json(&pointer_path(workspace), ArtifactKind::WorkspaceTmuxSession)
}

fn write_pointer_first_writer_wins(
    workspace: &Utf8Path,
    session: SessionName,
) -> Result<SessionName> {
    let pointer = WorkspaceTmuxSession {
        session: session.as_str().to_owned(),
        created_at: Utc::now(),
    };
    match create_pointer(&pointer_path(workspace), &pointer) {
        Ok(()) => Ok(session),
        Err(err)
            if err
                .downcast_ref::<std::io::Error>()
                .is_some_and(is_already_exists) =>
        {
            read_pointer(workspace)?
                .context(
                    "workspace tmux session pointer was created concurrently but is unreadable",
                )?
                .session_name()
        }
        Err(err) => Err(err),
    }
}

fn create_pointer(path: &Utf8Path, pointer: &WorkspaceTmuxSession) -> Result<()> {
    let Some(parent) = path.parent() else {
        bail!("workspace tmux session pointer path {path} has no parent");
    };
    fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    let body = pointer_json(pointer)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("failed to create workspace tmux session pointer {path}"))?;
    file.write_all(body.as_bytes())
        .with_context(|| format!("failed to write workspace tmux session pointer {path}"))
}

fn pointer_json(pointer: &WorkspaceTmuxSession) -> Result<String> {
    let mut value = serde_json::to_value(pointer)
        .context("failed to serialize workspace tmux session pointer")?;
    let Some(object) = value.as_object_mut() else {
        bail!("workspace tmux session pointer must serialize as an object");
    };
    // write_json cannot be used here because pointer creation needs O_EXCL first-writer semantics.
    object.insert(
        "niles_schema".to_owned(),
        JsonValue::from(schema::CURRENT_SCHEMA),
    );
    serde_json::to_string_pretty(&value)
        .context("failed to serialize workspace tmux session pointer")
}

fn pointer_path(workspace: &Utf8Path) -> Utf8PathBuf {
    workspace.join(".niles").join("sessions").join(POINTER_FILE)
}

fn is_already_exists(err: &std::io::Error) -> bool {
    err.kind() == ErrorKind::AlreadyExists
}

impl WorkspaceTmuxSession {
    fn session_name(self) -> Result<SessionName> {
        SessionName::new(self.session).context("workspace tmux session pointer is invalid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn deterministic_session_is_non_ambient_and_stable() {
        let workspace = temp_workspace("deterministic");

        let first = deterministic_session(&workspace).unwrap();
        let second = deterministic_session(&workspace).unwrap();

        assert_eq!(first, second);
        assert!(
            first
                .as_str()
                .starts_with("niles-niles-workspace-session-deterministic-")
        );
        assert_ne!(first.as_str(), "niles");
        fs::remove_dir_all(workspace).unwrap();
    }

    fn temp_workspace(label: &str) -> Utf8PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = Utf8PathBuf::from_path_buf(
            std::env::temp_dir().join(format!("niles-workspace-session-{label}-{nanos}")),
        )
        .unwrap();
        fs::create_dir_all(&path).unwrap();
        path
    }
}

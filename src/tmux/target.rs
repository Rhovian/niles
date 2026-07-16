use std::fmt;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SessionName(String);

impl SessionName {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            bail!("tmux session name cannot be empty");
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowTarget {
    session: SessionName,
    window: String,
}

impl WindowTarget {
    pub(crate) fn new(session: SessionName, window: impl Into<String>) -> Result<Self> {
        let window = window.into();
        if window.is_empty() {
            bail!("tmux window name cannot be empty");
        }
        Ok(Self { session, window })
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        let Some((session, window)) = value.rsplit_once(':') else {
            bail!("tmux window target `{value}` must be session:window");
        };
        Self::new(SessionName::new(session.to_owned())?, window.to_owned())
            .with_context(|| format!("invalid tmux window target `{value}`"))
    }

    pub(crate) fn session(&self) -> &SessionName {
        &self.session
    }

    pub(crate) fn window(&self) -> &str {
        &self.window
    }

    pub(crate) fn render(&self) -> String {
        format!("{}:{}", self.session, self.window)
    }
}

impl fmt::Display for WindowTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TargetState {
    Live,
    WindowDead,
    OrphanRecovered { actual: WindowTarget },
    OrphanGone,
    OrphanLegacyCandidate { candidate: WindowTarget },
    Unknown { error: String },
}

impl fmt::Display for TargetState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Live => f.write_str("live"),
            Self::WindowDead => f.write_str("window-dead"),
            Self::OrphanRecovered { actual } => write!(f, "orphan-recovered:{actual}"),
            Self::OrphanGone => f.write_str("orphan-gone"),
            Self::OrphanLegacyCandidate { candidate } => {
                write!(f, "orphan-legacy-candidate:{candidate}")
            }
            Self::Unknown { error } => write!(f, "unknown:{error}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaggedWindow {
    target: WindowTarget,
    project: Option<String>,
    worker_id: Option<String>,
}

pub(crate) fn target_state(recorded: &WindowTarget, project: &Utf8Path, id: &str) -> TargetState {
    let recorded_query = recorded_window_state(recorded);
    match recorded_query {
        RecordedWindowState::Live => TargetState::Live,
        // A missing window in a live session still runs tag-based recovery; a
        // matching project/id tag is a positive identity match and safe to kill.
        RecordedWindowState::WindowMissing => recover_missing_target(recorded, project, id, false),
        RecordedWindowState::SessionMissing => recover_missing_target(recorded, project, id, true),
        RecordedWindowState::Unknown { error } => TargetState::Unknown { error },
    }
}

fn recorded_window_state(recorded: &WindowTarget) -> RecordedWindowState {
    let output = match super::output([
        "list-windows",
        "-t",
        recorded.session().as_str(),
        "-F",
        "#{window_name}",
    ]) {
        Ok(output) => output,
        Err(err) => {
            return RecordedWindowState::Unknown {
                error: format!("{err:#}"),
            };
        }
    };

    if output.status.success() {
        if window_list_contains(&output.stdout, recorded.window()) {
            RecordedWindowState::Live
        } else {
            RecordedWindowState::WindowMissing
        }
    } else {
        let stderr = normalize_stderr(&output.stderr);
        if is_missing_session_error(&stderr) {
            RecordedWindowState::SessionMissing
        } else {
            RecordedWindowState::Unknown {
                error: format!(
                    "tmux list-windows failed for session {}: {stderr}",
                    recorded.session()
                ),
            }
        }
    }
}

fn recover_missing_target(
    recorded: &WindowTarget,
    project: &Utf8Path,
    id: &str,
    session_missing: bool,
) -> TargetState {
    let windows = match list_tagged_windows() {
        Ok(windows) => windows,
        Err(err) => {
            return TargetState::Unknown {
                error: format!("{err:#}"),
            };
        }
    };

    let tagged_matches = windows
        .iter()
        .filter(|window| {
            window.project.as_deref() == Some(project.as_str())
                && window.worker_id.as_deref() == Some(id)
        })
        .map(|window| window.target.clone())
        .collect::<Vec<_>>();
    match tagged_matches.as_slice() {
        [actual] => {
            return TargetState::OrphanRecovered {
                actual: actual.clone(),
            };
        }
        [] => {}
        [first, second, rest @ ..] => {
            let mut targets = vec![first.render(), second.render()];
            targets.extend(rest.iter().map(WindowTarget::render));
            return TargetState::Unknown {
                error: format!(
                    "multiple tmux windows carry worker tags: {}",
                    targets.join(", ")
                ),
            };
        }
    }

    let legacy_candidates = windows
        .iter()
        .filter(|window| {
            window.target.window() == recorded.window()
                && window.project.is_none()
                && window.worker_id.is_none()
        })
        .map(|window| window.target.clone())
        .collect::<Vec<_>>();
    match legacy_candidates.as_slice() {
        [candidate] => TargetState::OrphanLegacyCandidate {
            candidate: candidate.clone(),
        },
        [] if session_missing => TargetState::OrphanGone,
        [] => TargetState::WindowDead,
        [first, second, rest @ ..] => {
            let mut targets = vec![first.render(), second.render()];
            targets.extend(rest.iter().map(WindowTarget::render));
            TargetState::Unknown {
                error: format!(
                    "multiple untagged tmux windows match recorded name: {}",
                    targets.join(", ")
                ),
            }
        }
    }
}

fn list_tagged_windows() -> Result<Vec<TaggedWindow>> {
    let output = super::output([
        "list-windows",
        "-a",
        "-F",
        "#{session_name}:#{window_name}\t#{@niles-project}\t#{@niles-worker-id}",
    ])
    .context("failed to list tmux windows across sessions")?;
    if !output.status.success() {
        bail!(
            "tmux list-windows across sessions failed: {}",
            normalize_stderr(&output.stderr)
        );
    }
    parse_tagged_windows(&output.stdout)
}

fn parse_tagged_windows(stdout: &[u8]) -> Result<Vec<TaggedWindow>> {
    let text = String::from_utf8_lossy(stdout);
    let mut rows = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 3 {
            bail!(
                "malformed tmux tagged window line {}: expected 3 fields, got {}",
                line_number + 1,
                fields.len()
            );
        }
        rows.push(TaggedWindow {
            target: WindowTarget::parse(fields[0]).with_context(|| {
                format!(
                    "invalid tmux target on tagged window line {}",
                    line_number + 1
                )
            })?,
            project: nonempty(fields[1]),
            worker_id: nonempty(fields[2]),
        });
    }
    Ok(rows)
}

fn window_list_contains(stdout: &[u8], window_name: &str) -> bool {
    String::from_utf8_lossy(stdout)
        .lines()
        .any(|line| line == window_name)
}

fn normalize_stderr(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_missing_session_error(stderr: &str) -> bool {
    stderr.contains("can't find session")
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

enum RecordedWindowState {
    Live,
    WindowMissing,
    SessionMissing,
    Unknown { error: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_window_target() {
        let target = WindowTarget::parse("aquila:niles-auth-fix").unwrap();

        assert_eq!(target.session().as_str(), "aquila");
        assert_eq!(target.window(), "niles-auth-fix");
        assert_eq!(target.render(), "aquila:niles-auth-fix");
    }

    #[test]
    fn rejects_missing_target_separator() {
        let err = WindowTarget::parse("niles-auth-fix").unwrap_err();

        assert!(err.to_string().contains("session:window"));
    }

    #[test]
    fn rejects_empty_session() {
        let err = WindowTarget::parse(":niles-auth-fix").unwrap_err();

        assert!(err.to_string().contains("session name cannot be empty"));
    }

    #[test]
    fn rejects_empty_window() {
        let err = WindowTarget::parse("aquila:").unwrap_err();
        let chain = err.chain().map(ToString::to_string).collect::<Vec<_>>();

        assert!(
            chain
                .iter()
                .any(|message| message.contains("window name cannot be empty"))
        );
    }

    #[test]
    fn parses_tagged_window_rows() {
        let rows = parse_tagged_windows(
            b"home:niles-auth\t/Users/j/code/niles\tauth\nother:niles-docs\t\t\n",
        )
        .unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].target.render(), "home:niles-auth");
        assert_eq!(rows[0].project.as_deref(), Some("/Users/j/code/niles"));
        assert_eq!(rows[0].worker_id.as_deref(), Some("auth"));
        assert_eq!(rows[1].project, None);
        assert_eq!(rows[1].worker_id, None);
    }
}

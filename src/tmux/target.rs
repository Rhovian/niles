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

    /// The stored, human-readable form. This is what lands in worker metadata
    /// and what `parse` round-trips, so it must stay free of target syntax.
    pub(crate) fn render(&self) -> String {
        format!("{}:{}", self.session, self.window)
    }

    /// The form to hand tmux after `-t`. See [`exact`].
    pub(crate) fn target_arg(&self) -> String {
        format!("{}:{}", exact(self.session.as_str()), exact(&self.window))
    }
}

/// Where tmux is asked to act.
///
/// A worker window is addressed as `session:window`, both halves anchored. The lead's own pane is
/// a `%N` pane id, which tmux accepts as a complete target on its own: there is no session or
/// window to spell, and inventing one would address a different thing — the lead is typed into
/// through the pane it is running in, and that pane id is the only fact about it we have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneTarget(String);

impl PaneTarget {
    pub(crate) fn window(target: &WindowTarget) -> Self {
        Self(target.target_arg())
    }

    /// A `%N` pane id, as tmux reports it in `$TMUX_PANE`.
    pub(crate) fn pane(id: &str) -> Result<Self> {
        let id = id.trim();
        if id.is_empty() {
            bail!("tmux pane id cannot be empty");
        }
        Ok(Self(id.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PaneTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// tmux resolves a `-t` component exact, then by glob, then by *prefix*, and
/// it does so independently for the session and window halves. Unanchored,
/// `-t niles:niles-auth` reaches `niles-auth-fix` once `niles-auth` is gone —
/// exit 0, wrong window, no warning. `=` pins the component to an exact match.
pub(crate) fn exact(name: &str) -> String {
    format!("={name}")
}

impl fmt::Display for WindowTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TargetState {
    Live,
    /// The recorded window is still there, but its agent has exited. The pane is kept so its
    /// output stays readable, so this is a window to clean up — not one that is already gone.
    PaneExited,
    WindowDead,
    OrphanRecovered {
        actual: WindowTarget,
    },
    OrphanGone,
    OrphanLegacyCandidate {
        candidate: WindowTarget,
    },
    Unknown {
        error: String,
    },
}

impl fmt::Display for TargetState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Live => f.write_str("live"),
            Self::PaneExited => f.write_str("agent-exited"),
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
        RecordedWindowState::PaneExited => TargetState::PaneExited,
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
        &exact(recorded.session().as_str()),
        "-F",
        LIVE_WINDOW_FORMAT,
    ]) {
        Ok(output) => output,
        Err(err) => {
            return RecordedWindowState::Unknown {
                error: format!("{err:#}"),
            };
        }
    };

    if output.status.success() {
        match window_presence(&output.stdout, recorded.window()) {
            WindowPresence::Live => RecordedWindowState::Live,
            WindowPresence::PaneExited => RecordedWindowState::PaneExited,
            WindowPresence::Absent => RecordedWindowState::WindowMissing,
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
        "#{session_name}:#{window_name}\t#{@niles-project}\t#{@niles-worker-id}\t#{pane_dead}",
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
        if fields.len() != 4 {
            bail!(
                "malformed tmux tagged window line {}: expected 4 fields, got {}",
                line_number + 1,
                fields.len()
            );
        }
        // A window whose pane has exited is kept for its output, but it is not somewhere a
        // worker can still be recovered to.
        if is_dead_pane(fields[3]) {
            continue;
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

/// `name\tpane_dead`. Worker windows are kept after their agent exits so the pane stays
/// readable, so presence in the listing is no longer the same question as being alive.
const LIVE_WINDOW_FORMAT: &str = "#{window_name}\t#{pane_dead}";

/// Whether the window is listed, and whether anything is still running in it. These are
/// different questions: a window whose agent has exited is kept so its output stays readable,
/// and it still occupies its name until the worker is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowPresence {
    Live,
    PaneExited,
    Absent,
}

pub(super) fn window_presence(stdout: &[u8], window_name: &str) -> WindowPresence {
    for line in String::from_utf8_lossy(stdout).lines() {
        match line.split_once('\t') {
            Some((name, dead)) if name == window_name => {
                return if is_dead_pane(dead) {
                    WindowPresence::PaneExited
                } else {
                    WindowPresence::Live
                };
            }
            // A listing without the dead column predates this format; treat presence as live.
            None if line == window_name => return WindowPresence::Live,
            _ => {}
        }
    }
    WindowPresence::Absent
}

fn is_dead_pane(field: &str) -> bool {
    field.trim() == "1"
}

pub(super) fn normalize_stderr(stderr: &[u8]) -> String {
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
    PaneExited,
    WindowMissing,
    SessionMissing,
    Unknown { error: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_session_names_stay_permissive() {
        // Workers already running inside a dotted session recorded that name;
        // tightening `new` would strand them.
        assert!(SessionName::new("my.proj").is_ok());
    }

    #[test]
    fn target_arg_anchors_both_components() {
        // Unanchored, tmux resolves `-t niles:niles-auth` to `niles-auth-fix`
        // once `niles-auth` is gone: exit 0, wrong window, no warning.
        let target = WindowTarget::parse("niles:niles-auth").unwrap();

        assert_eq!(target.target_arg(), "=niles:=niles-auth");
    }

    #[test]
    fn the_stored_form_stays_free_of_target_syntax() {
        // `render` lands in worker metadata and must round-trip through
        // `parse`; anchoring it would corrupt every recorded worker.
        let target = WindowTarget::parse("niles:niles-auth").unwrap();

        assert_eq!(target.render(), "niles:niles-auth");
        assert_eq!(
            WindowTarget::parse(&target.render()).unwrap().render(),
            "niles:niles-auth"
        );
    }

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
            b"home:niles-auth\t/Users/j/code/niles\tauth\t0\nother:niles-docs\t\t\t0\n",
        )
        .unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].target.render(), "home:niles-auth");
        assert_eq!(rows[0].project.as_deref(), Some("/Users/j/code/niles"));
        assert_eq!(rows[0].worker_id.as_deref(), Some("auth"));
        assert_eq!(rows[1].project, None);
        assert_eq!(rows[1].worker_id, None);
    }

    /// Worker windows outlive their agent so the pane stays readable, so "listed" and "live"
    /// are now different questions.
    #[test]
    fn a_dead_pane_is_present_but_not_live() {
        assert_eq!(
            window_presence(b"niles-run\t0\n", "niles-run"),
            WindowPresence::Live
        );
        // Present, so still to be cleaned up — not the same as absent.
        assert_eq!(
            window_presence(b"niles-run\t1\n", "niles-run"),
            WindowPresence::PaneExited
        );
        assert_eq!(
            window_presence(b"niles-run\t0\n", "run"),
            WindowPresence::Absent
        );
    }

    /// A kept-but-dead pane is output to read, not a window a worker can be recovered to.
    #[test]
    fn dead_panes_are_not_recovery_candidates() {
        let rows = parse_tagged_windows(
            b"home:niles-auth\t/repo\tauth\t1\nhome:niles-docs\t/repo\tdocs\t0\n",
        )
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].worker_id.as_deref(), Some("docs"));
    }
}

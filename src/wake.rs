use std::fmt;

use camino::{Utf8Path, Utf8PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WakeKind {
    Done,
    Failed,
    Blocked,
    NeedsDecision,
    Closed,
    Working,
}

impl WakeKind {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "blocked" => Some(Self::Blocked),
            "needs-decision" => Some(Self::NeedsDecision),
            "closed" => Some(Self::Closed),
            "working" => Some(Self::Working),
            _ => None,
        }
    }

    pub(crate) fn parse_line(line: &str) -> Option<Self> {
        let (state, _) = line.split_once(':')?;
        Self::parse(state)
    }

    pub(crate) fn is_actionable(self) -> bool {
        !matches!(self, Self::Working)
    }

    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Closed)
    }
}

impl fmt::Display for WakeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::NeedsDecision => "needs-decision",
            Self::Closed => "closed",
            Self::Working => "working",
        })
    }
}

pub(crate) fn status_log_path(dir: &Utf8Path) -> Utf8PathBuf {
    dir.join("status.log")
}

pub(crate) fn line(kind: WakeKind, detail: &str) -> String {
    format!("{kind}: {detail}")
}

pub(crate) fn is_actionable_wake(line: &str) -> bool {
    WakeKind::parse_line(line).is_some_and(WakeKind::is_actionable)
}

pub(crate) fn is_closed_wake(line: &str) -> bool {
    WakeKind::parse_line(line).is_some_and(WakeKind::is_terminal)
}

pub(crate) fn worker_contract_examples(status_path: &Utf8Path) -> String {
    worker_contract_examples_for_target(status_path.as_str())
}

pub(crate) fn manager_worker_contract_examples(status_target: &str) -> String {
    render_echo_examples(
        &[
            line(WakeKind::Done, "short result"),
            line(WakeKind::Blocked, "blocker summary"),
            line(WakeKind::NeedsDecision, "decision needed"),
            line(WakeKind::Failed, "failure summary"),
            line(WakeKind::Closed, "worker closed"),
        ],
        status_target,
    )
}

fn worker_contract_examples_for_target(status_target: &str) -> String {
    render_echo_examples(
        &[
            line(WakeKind::Done, "short result"),
            line(WakeKind::Blocked, "blocker summary"),
            line(WakeKind::NeedsDecision, "decision needed"),
            line(WakeKind::Failed, "failure summary"),
        ],
        status_target,
    )
}

fn render_echo_examples(lines: &[String], status_target: &str) -> String {
    lines
        .iter()
        .map(|line| format!("echo \"{line}\" >> {status_target}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_case_sensitive_wake_kind_prefixes() {
        assert_eq!(WakeKind::parse_line("done: shipped"), Some(WakeKind::Done));
        assert_eq!(
            WakeKind::parse_line("needs-decision: choose"),
            Some(WakeKind::NeedsDecision)
        );
        assert_eq!(
            WakeKind::parse_line("working: launch"),
            Some(WakeKind::Working)
        );
        assert_eq!(WakeKind::parse_line("Done: shipped"), None);
        assert_eq!(WakeKind::parse_line("done shipped"), None);
        assert_eq!(WakeKind::parse_line("done : shipped"), None);
    }

    #[test]
    fn classifies_actionable_and_terminal_lines() {
        assert!(is_actionable_wake("done: shipped"));
        assert!(is_actionable_wake("closed: auth-fix"));
        assert!(!is_actionable_wake("working: launch"));
        assert!(!is_actionable_wake("note: launch"));
        assert!(is_closed_wake("closed: auth-fix"));
        assert!(!is_closed_wake("failed: auth-fix"));
    }

    #[test]
    fn formats_wake_lines() {
        assert_eq!(line(WakeKind::Closed, "auth-fix"), "closed: auth-fix");
    }

    #[test]
    fn renders_contract_echo_examples() {
        assert_eq!(
            worker_contract_examples(Utf8Path::new("/tmp/status.log")),
            "echo \"done: short result\" >> /tmp/status.log\n\
echo \"blocked: blocker summary\" >> /tmp/status.log\n\
echo \"needs-decision: decision needed\" >> /tmp/status.log\n\
echo \"failed: failure summary\" >> /tmp/status.log"
        );
    }
}

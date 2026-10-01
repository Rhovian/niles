use std::{borrow::Cow, fmt};

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
    const ALL: [Self; 6] = [
        Self::Done,
        Self::Failed,
        Self::Blocked,
        Self::NeedsDecision,
        Self::Closed,
        Self::Working,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::NeedsDecision => "needs-decision",
            Self::Closed => "closed",
            Self::Working => "working",
        }
    }

    pub(crate) fn parse_line(line: &str) -> Option<Self> {
        let (state, _) = line.split_once(':')?;
        Self::ALL.into_iter().find(|kind| kind.as_str() == state)
    }

    pub(crate) fn actionable(line: &str) -> Option<Self> {
        Self::parse_line(line).filter(|kind| *kind != Self::Working)
    }
}

impl fmt::Display for WakeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub(crate) fn status_log_path(dir: &Utf8Path) -> Utf8PathBuf {
    dir.join("status.log")
}

pub(crate) fn line(kind: WakeKind, detail: &str) -> String {
    format!("{kind}: {detail}")
}

pub(crate) fn is_actionable_wake(line: &str) -> bool {
    WakeKind::actionable(line).is_some()
}

pub(crate) fn complete_lines(log: &[u8]) -> impl Iterator<Item = (usize, Cow<'_, str>)> {
    let mut end = 0;
    log.split_inclusive(|byte| *byte == b'\n')
        .take_while(|raw| raw.ends_with(b"\n"))
        .map(move |raw| {
            end += raw.len();
            let mut raw = &raw[..raw.len() - 1];
            while let Some(stripped) = raw.strip_suffix(b"\r") {
                raw = stripped;
            }
            (end, String::from_utf8_lossy(raw))
        })
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
    fn classifies_actionable_lines() {
        assert!(is_actionable_wake("done: shipped"));
        assert!(is_actionable_wake("closed: auth-fix"));
        assert!(!is_actionable_wake("working: launch"));
        assert!(!is_actionable_wake("note: launch"));
    }

    #[test]
    fn complete_lines_trim_all_carriage_returns_without_shifting_offsets() {
        let lines = complete_lines(b"done: x\r\r\n").collect::<Vec<_>>();
        assert_eq!(lines, vec![(10, Cow::Borrowed("done: x"))]);
    }
}

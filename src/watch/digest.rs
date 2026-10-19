//! Pending lead messages share one delivery and landing-only commit path.
use super::checkin::Checkin;
use crate::{telemetry::SessionState, wake::WakeKind};
use camino::Utf8PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Item {
    Reported {
        id: String,
        kind: WakeKind,
        log_len: u64,
    },
    Checkin {
        id: String,
        worker_dir: Utf8PathBuf,
        state: Option<SessionState>,
        planned: Checkin,
        next: Option<Checkin>,
    },
}

impl Item {
    pub(super) fn id(&self) -> &str {
        match self {
            Self::Reported { id, .. } | Self::Checkin { id, .. } => id,
        }
    }
    fn phrase(&self) -> String {
        match self {
            Self::Reported { id, kind, .. } => format!("{id} reported ({kind})"),
            Self::Checkin {
                id,
                state: Some(SessionState::Waiting),
                ..
            } => {
                format!("{id} stopped without a report")
            }
            Self::Checkin { id, .. } => format!("no report from {id}"),
        }
    }
    #[cfg(test)]
    pub(super) fn text(&self) -> String {
        text(std::slice::from_ref(self))
    }
}

// libc 0.2.189 write (src/unix/mod.rs:1188) writes bytes, without message
// aggregation; tmux::send_line (src/tmux/send.rs) submits one string. Combine
// pending state before that single submission and its commit.
pub(super) fn text(items: &[Item]) -> String {
    let phrases: Vec<_> = items.iter().map(Item::phrase).collect();
    format!("niles: {} — check workers", phrases.join(", "))
}

/// Accept both old single-report lines and current mixed batches in watch.log.
pub(super) fn parse_reports(text: &str) -> impl Iterator<Item = (&str, WakeKind)> {
    text.strip_prefix("niles: ")
        .and_then(|text| text.strip_suffix(" — check workers"))
        .into_iter()
        .flat_map(|text| text.split(", "))
        .filter_map(|phrase| {
            let (id, kind) = phrase.split_once(" reported (")?;
            Some((id, WakeKind::parse(kind.strip_suffix(')')?)?))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_batches_and_old_reports_parse() {
        for kind in WakeKind::ALL {
            let item = Item::Reported {
                id: "w1".to_owned(),
                kind,
                log_len: 4,
            };
            assert_eq!(
                parse_reports(&text(&[item])).collect::<Vec<_>>(),
                vec![("w1", kind)]
            );
        }
        assert_eq!(parse_reports("niles: w1 reported (done), no report from w2, w3 reported (blocked) — check workers").collect::<Vec<_>>(),
            vec![("w1", WakeKind::Done), ("w3", WakeKind::Blocked)]);
    }
}

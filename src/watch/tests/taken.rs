//! A line `niles wait` has handed the lead, or is about to, is not reported again.
use super::*;
use crate::store::atomic_write;
use crate::test_support;
use crate::{util, wait};

const BLOCKED: &str = "blocked: click OK\n";
const REPORTED: &str = "niles: impl reported (blocked) — check workers";

#[test]
fn a_line_a_wait_delivered_is_not_reported() {
    for (label, acked, respawned) in [
        ("taken-acked", true, false),
        ("taken-published", false, false),
        ("taken-respawned", false, true),
    ] {
        let (root, mut memory) = one_worker(label, None);
        append_log(&root, "impl", BLOCKED);
        let dir = worker_dir(&root, "impl");
        if acked {
            let delivered = status(&root, "impl").len();
            atomic_write(dir.join("status.cursor"), format!("{delivered}\n")).unwrap();
        } else {
            wait::wait(&root, Duration::from_secs(1), Duration::ZERO, None).unwrap();
        }
        if respawned {
            fs::remove_dir_all(&dir).unwrap();
            write_meta(&root, "impl", at(1_000), "niles-test");
            atomic_write(dir.join("status.log"), format!("{WINDOW}{BLOCKED}")).unwrap();
        }

        let mut sink = RecordingSink::default();
        tick(&root, at(1_000), &mut memory, &mut sink, &running()).unwrap();
        let expected: &[_] = if respawned { &[REPORTED] } else { &[] };
        assert_eq!(sink.attempts, expected, "{label}: {sink:?}");

        let mut sink = RecordingSink::default();
        util::append_line(&dir.join("status.log"), BLOCKED.trim_end()).unwrap();
        tick(&root, at(1_001), &mut memory, &mut sink, &running()).unwrap();
        assert_eq!(sink.sent, [REPORTED], "{label}");
        test_support::remove_workspace(&root).unwrap();
    }
}

//! A line `niles wait` has handed the lead, or is about to, is not reported again.

use super::*;

const BLOCKED: &str = "blocked: click OK\n";
const REPORTED: &str = "niles: impl reported (blocked) — check workers";

#[test]
fn a_line_a_wait_delivered_is_not_reported() {
    let (root, mut memory) = one_worker("taken-delivered", None);
    append_log(&root, "impl", BLOCKED);
    let delivered = status(&root, "impl").len();
    fs::write(
        worker_dir(&root, "impl").join("status.cursor"),
        format!("{delivered}\n"),
    )
    .unwrap();

    let mut sink = RecordingSink::default();
    tick(&root, at(1_000), &mut memory, &mut sink, &running()).unwrap();
    assert!(sink.attempts.is_empty(), "{sink:?}");

    append_log(&root, "impl", BLOCKED);
    tick(&root, at(1_001), &mut memory, &mut sink, &running()).unwrap();
    assert_eq!(sink.sent, vec![REPORTED]);
    crate::test_support::remove_workspace(&root).unwrap();
}

//! A line `niles wait` has handed the lead, or is about to, is not reported again.

use super::*;

const BLOCKED: &str = "blocked: click OK\n";
const REPORTED: &str = "niles: impl reported (blocked) — check workers";

#[test]
fn a_line_a_wait_delivered_is_not_reported() {
    let root = workspace("taken-delivered");
    write_worker(&root, "impl", WINDOW);
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    append_log(&root, "impl", BLOCKED);
    let delivered = status(&root, "impl").len();
    fs::write(
        worker_dir(&root, "impl").join("status.cursor"),
        format!("{delivered}\n"),
    )
    .unwrap();

    let mut sink = RecordingSink::default();
    tick(&root, at(1_000), &mut memory, &mut sink, &running());
    assert!(sink.attempts.is_empty(), "{sink:?}");

    append_log(&root, "impl", BLOCKED);
    tick(&root, at(1_001), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent, vec![REPORTED]);
}

#[test]
fn a_line_that_lands_while_a_wait_runs_is_left_to_it() {
    let root = workspace("taken-running");
    write_worker(&root, "impl", WINDOW);
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    // Held the way a running wait holds it.
    let wait = fs::File::create(root.join(".niles/worker/wait.running")).unwrap();
    wait.lock_shared().unwrap();
    append_log(&root, "impl", BLOCKED);

    let mut sink = RecordingSink::default();
    tick(&root, at(1_000), &mut memory, &mut sink, &running());
    assert!(sink.attempts.is_empty(), "{sink:?}");

    // The wait ended without delivering it, so the line is still the lead's to hear about.
    drop(wait);
    tick(&root, at(1_001), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent, vec![REPORTED]);
}

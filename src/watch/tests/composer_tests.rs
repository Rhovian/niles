use super::*;

fn typed() -> RecordingSink {
    RecordingSink {
        typed: true,
        ..RecordingSink::default()
    }
}

#[test]
fn a_typed_composer_holds_every_nudge_until_the_ceiling() {
    let root = workspace("composer-ceiling");
    write_worker(&root, "alpha", WINDOW);
    write_worker(&root, "beta", WINDOW);
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    append_log(&root, "alpha", DONE);
    append_log(&root, "beta", DONE);
    let mut sink = typed();

    tick(&root, at(1_000), &mut memory, &mut sink, &running());
    tick(&root, at(1_299), &mut memory, &mut sink, &running());
    assert!(sink.sent.is_empty());
    tick(&root, at(1_300), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent.len(), 2);
    append_log(&root, "alpha", DONE);
    tick(&root, at(1_301), &mut memory, &mut sink, &running());
    assert_eq!(
        sink.sent.len(),
        3,
        "past the ceiling, nothing waits for the composer to clear"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_cleared_composer_delivers_and_restarts_the_hold() {
    let root = workspace("composer-clear");
    write_worker(&root, "impl", WINDOW);
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    append_log(&root, "impl", DONE);
    let mut sink = typed();

    tick(&root, at(1_000), &mut memory, &mut sink, &running());
    sink.typed = false;
    tick(&root, at(1_001), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent.len(), 1);
    sink.typed = true;
    append_log(&root, "impl", DONE);
    // Past the first hold's ceiling, so this holds only if the clear reading restarted it.
    tick(&root, at(1_350), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent.len(), 1);

    fs::remove_dir_all(root).unwrap();
}

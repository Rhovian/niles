use super::*;

#[test]
fn composer_hold_reaches_ceiling_and_restarts_after_clear() {
    let root = workspace("composer-hold");
    write_worker(&root, "impl", WINDOW);
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    append_log(&root, "impl", DONE);
    let mut sink = RecordingSink {
        typed: true,
        ..RecordingSink::default()
    };

    tick(&root, at(1_000), &mut memory, &mut sink, &running());
    tick(&root, at(1_299), &mut memory, &mut sink, &running());
    assert!(sink.sent.is_empty());
    tick(&root, at(1_300), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent.len(), 1);
    append_log(&root, "impl", DONE);
    tick(&root, at(1_301), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent.len(), 2);
    sink.typed = false;
    tick(&root, at(1_302), &mut memory, &mut sink, &running());
    sink.typed = true;
    append_log(&root, "impl", DONE);
    tick(&root, at(1_350), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent.len(), 2);
    fs::remove_dir_all(root).unwrap();
}

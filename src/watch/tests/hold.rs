use super::*;

#[test]
fn composer_hold_reaches_ceiling_and_restarts_after_clear() {
    let (root, mut memory) = one_worker("composer-hold", None);
    append_log(&root, "impl", DONE);
    let mut sink = RecordingSink {
        typed: true,
        ..RecordingSink::default()
    };

    tick(&root, at(1_000), &mut memory, &mut sink, &running()).unwrap();
    tick(&root, at(1_299), &mut memory, &mut sink, &running()).unwrap();
    assert!(sink.sent.is_empty());
    tick(&root, at(1_300), &mut memory, &mut sink, &running()).unwrap();
    assert_eq!(sink.sent.len(), 1);
    append_log(&root, "impl", DONE);
    tick(&root, at(1_301), &mut memory, &mut sink, &running()).unwrap();
    assert_eq!(sink.sent.len(), 2);
    sink.typed = false;
    tick(&root, at(1_302), &mut memory, &mut sink, &running()).unwrap();
    sink.typed = true;
    append_log(&root, "impl", DONE);
    tick(&root, at(1_350), &mut memory, &mut sink, &running()).unwrap();
    assert_eq!(sink.sent.len(), 2);
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn working_worker_moves_checkin_without_a_nudge() {
    let (root, mut memory) = one_worker("working-hold", Some(due_checkin()));
    let dir = worker_dir(&root, "impl");
    let mut sink = RecordingSink {
        activity: Some(SessionState::Working),
        ..RecordingSink::default()
    };
    tick(&root, at(1_300), &mut memory, &mut sink, &running()).unwrap();
    assert!(sink.sent.is_empty());
    assert_eq!(Checkin::read(&dir).unwrap().unwrap().deadline, at(1_600));
    crate::test_support::remove_workspace(&root).unwrap();
}

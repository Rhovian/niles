use super::*;

#[test]
fn mixed_batch_commits_all_items_only_after_one_successful_send() {
    let root = workspace("mixed-batch");
    for id in ["a", "b", "c", "d"] {
        write_worker(&root, id, WINDOW);
    }
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    append_log(&root, "a", DONE);
    append_log(&root, "b", "blocked: choose\n");
    let checkin = due_checkin();
    for id in ["c", "d"] {
        checkin.write(&worker_dir(&root, id)).unwrap();
    }
    let mut sink = RecordingSink {
        fail: true,
        activity: Some(SessionState::Starting),
        ..RecordingSink::default()
    };
    tick(&root, at(1_300), &mut memory, &mut sink, &running());
    assert_eq!(sink.attempts.len(), 1);
    assert!(sink.sent.is_empty());
    for id in ["c", "d"] {
        assert_eq!(
            Checkin::read(&worker_dir(&root, id)).unwrap(),
            Some(checkin)
        );
    }
    sink.fail = false;
    tick(&root, at(1_301), &mut memory, &mut sink, &running());
    assert_eq!(sink.attempts.len(), 2);
    assert_eq!(
        sink.sent,
        [
            "niles: a reported (done), b reported (blocked), no report from c, no report from d — check workers"
        ]
    );
    for id in ["c", "d"] {
        assert!(
            Checkin::read(&worker_dir(&root, id))
                .unwrap()
                .unwrap()
                .deadline
                > checkin.deadline
        );
    }
    tick(&root, at(1_302), &mut memory, &mut sink, &running());
    assert_eq!(sink.attempts.len(), 2);
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn reports_that_accumulate_behind_a_composer_draft_land_in_one_line() {
    let root = workspace("draft-batch");
    for id in ["a", "b"] {
        write_worker(&root, id, WINDOW);
    }
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    let mut sink = RecordingSink {
        typed: true,
        ..RecordingSink::default()
    };
    append_log(&root, "a", DONE);
    tick(&root, at(1_000), &mut memory, &mut sink, &running());
    append_log(&root, "b", DONE);
    tick(&root, at(1_001), &mut memory, &mut sink, &running());
    assert!(sink.attempts.is_empty());
    sink.typed = false;
    tick(&root, at(1_002), &mut memory, &mut sink, &running());
    assert_eq!(
        sink.sent,
        ["niles: a reported (done), b reported (done) — check workers"]
    );
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn workspace_wait_suppresses_all_reports_but_due_checkins_still_land() {
    let root = workspace("wait-batch");
    for id in ["a", "b", "c", "d"] {
        write_worker(&root, id, WINDOW);
    }
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    for id in ["a", "b"] {
        append_log(&root, id, DONE);
    }
    let checkin = due_checkin();
    checkin.write(&worker_dir(&root, "c")).unwrap();
    let wait = fs::File::create(
        crate::store::workers_dir(&root)
            .unwrap()
            .join("wait.running"),
    )
    .unwrap();
    wait.lock_shared().unwrap();
    append_log(&root, "d", "blocked: click OK\n");
    let mut sink = RecordingSink {
        activity: Some(SessionState::Starting),
        ..RecordingSink::default()
    };
    tick(&root, at(1_300), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent, ["niles: no report from c — check workers"]);
    // Model a concurrent fork retaining the same open file description until exec.
    let inherited = wait.try_clone().unwrap();
    // flock(2): dup/fork share a lock; closing one descriptor need not release it.
    wait.unlock().unwrap();
    drop(wait);
    tick(&root, at(1_301), &mut memory, &mut sink, &running());
    assert_eq!(
        sink.sent[1],
        "niles: a reported (done), b reported (done), d reported (blocked) — check workers"
    );
    assert_eq!(sink.sent.len(), 2);
    drop(inherited);
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn watch_log_batches_feed_one_telemetry_report_per_worker() {
    let root = workspace("log-batch");
    fs::write(root.join("watch.log"), concat!(
        "2026-01-02T03:04:05Z nudged single: niles: single reported (done) — check workers\n",
        "2026-01-02T03:04:06Z nudged a,b,c: niles: a reported (blocked), no report from b, c reported (failed) — check workers\n",
        "not a report\n",
    )).unwrap();
    let events = super::super::delivered_reports(&root).unwrap();
    assert_eq!(
        events
            .iter()
            .map(|(_, id, kind)| (id.as_str(), *kind))
            .collect::<Vec<_>>(),
        [
            ("single", crate::wake::WakeKind::Done),
            ("a", crate::wake::WakeKind::Blocked),
            ("c", crate::wake::WakeKind::Failed)
        ]
    );
    crate::test_support::remove_workspace(&root).unwrap();
}

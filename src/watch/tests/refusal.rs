use super::*;

fn setup(label: &str, failure: Option<&str>, finished_at: DateTime<Utc>) -> Utf8PathBuf {
    let project = workspace(label);
    write_worker(&project, "impl", "");
    let dir = worker_dir(&project, "impl");
    let session_dir = dir.join("pi");
    fs::create_dir_all(&session_dir).unwrap();
    let complete = serde_json::json!({"type":"message","timestamp":finished_at,
        "message":{"role":"assistant","stopReason":if failure.is_some() { "error" } else { "stop" },
        "errorMessage":failure}});
    fs::write(session_dir.join("session.jsonl"), format!("{complete}\n")).unwrap();
    let path = dir.join("meta.json");
    let mut meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    meta["agent"] = "pi:tencent/hy3".into();
    meta["agent_family"] = "pi".into();
    meta["model"] = "tencent/hy3".into();
    meta["session_link"] = serde_json::json!({"family":"pi","session_dir":session_dir});
    fs::write(path, meta.to_string()).unwrap();
    armed_checkin(300, 0, at(1000)).write(&dir).unwrap();
    project
}

#[test]
fn terminal_normal_and_stale_completions_disarm_and_nudge_once() {
    for (label, failure, timestamp, status_line, nudge) in [
        (
            "provider-refusal",
            Some("Refused\nDetails"),
            at(1100),
            "blocked: provider refused (pi:tencent/hy3): Refused\n",
            reported("impl", "blocked"),
        ),
        (
            "provider-normal",
            None,
            at(1100),
            "",
            "niles: impl stopped without a report — check workers".into(),
        ),
        (
            "provider-stale",
            Some("Refused"),
            at(999),
            "",
            "niles: impl stopped without a report — check workers".into(),
        ),
    ] {
        let project = setup(label, failure, timestamp);
        let mut memory = WatchMemory::at_start(&worker_snapshot(&project).unwrap());
        let mut sink = RecordingSink::default();
        tick(&project, at(1300), &mut memory, &mut sink, &running()).unwrap();
        assert_eq!(status(&project, "impl"), status_line, "{label}");
        assert_eq!(sink.sent, [nudge], "{label}");
        assert_eq!(
            Checkin::read(&worker_dir(&project, "impl")).unwrap(),
            None,
            "{label}"
        );
        tick(&project, at(1600), &mut memory, &mut sink, &running()).unwrap();
        assert_eq!(sink.sent.len(), 1, "{label}");
        assert!(sink.captures.is_empty(), "{label}");
        crate::test_support::remove_workspace(&project).unwrap();
    }
}

#[test]
fn unreadable_activity_nudges_and_rearms_without_a_blocked_line() {
    let project = setup("provider-read-error", None, at(1100));
    let dir = worker_dir(&project, "impl");
    fs::remove_file(dir.join("pi/session.jsonl")).unwrap();
    fs::create_dir(dir.join("pi/session.jsonl")).unwrap();
    let mut memory = WatchMemory::at_start(&worker_snapshot(&project).unwrap());
    let mut sink = RecordingSink::default();
    tick(&project, at(1300), &mut memory, &mut sink, &running()).unwrap();
    assert_eq!(status(&project, "impl"), "");
    assert_eq!(sink.sent, ["niles: no report from impl — check workers"]);
    assert!(Checkin::read(&dir).unwrap().unwrap().deadline > at(1300));
    assert!(
        sink.notes
            .iter()
            .any(|note| note.contains("failed to read activity"))
    );
    crate::test_support::remove_workspace(&project).unwrap();
}

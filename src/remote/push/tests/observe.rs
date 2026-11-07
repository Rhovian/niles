use super::*;

#[test]
fn waiting_and_operator_transitions_do_not_replay_history_or_retain_exited_sessions() {
    let fixture = crate::remote::tests::Fixture::new(1);
    let observer = Arc::new(Observer::default());
    let poll = || {
        observer
            .lock()
            .unwrap()
            .changes(fixture.records.lock().unwrap().clone())
    };
    let append_user = |text: &str| {
        fixture.append(&format!(
            "{}\n",
            serde_json::json!({"type":"message","message":{"role":"user","content":text}})
        ));
    };
    let (_root, store, subscription) = super::fixture();
    let store = Arc::new(store);
    let (other, _, _) = keyed_subscription("https://fcm.googleapis.com/fcm/send/other");
    for subscription in [&subscription, &other] {
        store
            .save("alice".into(), subscription.clone(), None)
            .unwrap();
    }
    let devices = store.subscriptions().unwrap();
    let send = |text: &str, subscription: &Subscription, success: bool| {
        let record = fixture.records.lock().unwrap()[0].clone();
        let mut chat = Chat::lead(&record).unwrap();
        chat.target = Some(crate::tmux::TmuxTarget::pane("%1").unwrap());
        let message = serde_json::from_value(
            serde_json::json!({"text": text, "sender_device": DeviceId::from_endpoint(&subscription.0.endpoint)}),
        )
        .unwrap();
        let sender = crate::remote::message::Sender {
            user: "alice".into(),
            observer: observer.clone(),
        };
        crate::remote::message::deliver(chat, message, Some(sender), |_, text| {
            let mut tracker = observer
                .0
                .try_lock()
                .expect("paste must not hold the observer lock");
            assert_eq!(
                tracker.sessions["project:session"].pending.len(),
                1,
                "record before paste"
            );
            if !success {
                anyhow::bail!("paste failed");
            }
            append_user(&text.trim().replace("\r\n", "\n"));
            tracker.observe(
                &Session::from_record(&record),
                Ok(Chat::lead(&record).unwrap()),
            );
            Ok(())
        })
    };
    append_user("history");
    let waiting = poll();
    assert_eq!(waiting[0].reason, Reason::Waiting);
    assert!(devices.iter().all(|device| waiting[0].includes(device)));
    assert!(poll().is_empty());
    fixture.append(&crate::remote::tests::message("assistant response"));
    for text in [
        "niles: w1 reported (done) — check workers",
        "niles: no report from w1 — check workers",
        "[Request interrupted by user]",
        "<environment_context>injected</environment_context>",
        "<command-name>/clear</command-name>",
        "<local-command-stdout>output</local-command-stdout>",
        " \n",
    ] {
        append_user(text);
    }
    assert!(poll().is_empty());
    for waiting in [true, false] {
        fixture.records.lock().unwrap()[0].waiting = waiting;
        for sender in [None, Some(&subscription), Some(&other)] {
            if let Some(sender) = sender {
                assert_eq!(
                    send(" hi\r\nthere \n", sender, true).unwrap(),
                    StatusCode::OK
                );
            } else {
                append_user("identical text");
            }
            let notifications = poll();
            assert_eq!(notifications.len(), 1);
            assert_eq!(notifications[0].reason, Reason::OperatorMessage);
            for device in &devices {
                assert_eq!(
                    notifications[0].includes(device),
                    sender.map(|s| &s.0.endpoint) != Some(&device.subscription.0.endpoint)
                );
            }
            assert!(poll().is_empty());
        }
    }
    // Multiple devices and terminal input between polls retain independent recipients.
    send("same", &subscription, true).unwrap();
    send("same", &other, true).unwrap();
    append_user("terminal");
    let notifications = poll();
    assert_eq!(notifications.len(), 3);
    assert_eq!(
        notifications[0]
            .excluded
            .as_ref()
            .map(|sender| &sender.device),
        Some(&DeviceId::from_endpoint(&subscription.0.endpoint))
    );
    assert_eq!(
        notifications[1]
            .excluded
            .as_ref()
            .map(|sender| &sender.device),
        Some(&DeviceId::from_endpoint(&other.0.endpoint))
    );
    assert!(notifications[2].excluded.is_none());
    let mut unregistered = subscription.clone();
    unregistered.0.endpoint = "https://wns.example/unregistered".into();
    assert_eq!(
        send("unregistered", &unregistered, true).unwrap(),
        StatusCode::OK
    );
    let notifications = poll();
    assert!(
        devices
            .iter()
            .all(|device| notifications[0].includes(device))
    );
    assert_err_contains(send("failed", &subscription, false), "paste failed");
    assert!(
        observer.lock().unwrap().sessions["project:session"]
            .pending
            .is_empty()
    );
    append_user("terminal finishes");
    fixture.records.lock().unwrap()[0].waiting = true;
    let notifications = poll();
    assert_eq!(
        notifications.iter().map(|n| n.reason).collect::<Vec<_>>(),
        vec![Reason::OperatorMessage, Reason::Waiting]
    );
    let records = std::mem::take(&mut *fixture.records.lock().unwrap());
    assert!(poll().is_empty());
    *fixture.records.lock().unwrap() = records;
    assert_eq!(poll()[0].reason, Reason::Waiting);
}
#[tokio::test]
async fn polling_failure_is_reported_without_any_browser_connection() {
    let sensitive = "private/session-and-prompt";
    let json = serde_json::from_str::<u64>(&format!("\"{sensitive}\"")).unwrap_err();
    let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, sensitive);
    for (error, detail) in [
        (anyhow::Error::new(json), "JSON Data at line 1, column"),
        (anyhow::Error::new(io), "I/O PermissionDenied"),
        (
            anyhow::anyhow!(sensitive),
            "invalid transcript or session metadata",
        ),
    ] {
        let diagnostic = observation_diagnostic(&error.context(sensitive));
        assert!(diagnostic.contains(detail), "{diagnostic}");
        assert!(!diagnostic.contains(sensitive), "{diagnostic}");
    }
    let (_root, store, _) = fixture();
    let source: Source = Arc::new(|| anyhow::bail!("catalog failure"));
    assert_err_contains(
        tokio::time::timeout(
            Duration::from_secs(3),
            watch(source, Arc::new(store), Arc::default()),
        )
        .await
        .unwrap(),
        "catalog failure",
    );
    let good = crate::remote::tests::Fixture::new(0);
    let late = crate::remote::tests::Fixture::new(0);
    late.records.lock().unwrap()[0].meta.id = "late".into();
    fs::remove_file(late.root.join("001.jsonl")).unwrap();
    let mut tracker = Tracker::default();
    let records = || {
        [
            good.records.lock().unwrap().clone(),
            late.records.lock().unwrap().clone(),
        ]
        .concat()
    };
    tracker.changes(records());
    fs::write(
        late.root.join("001.jsonl"),
        "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"old prompt\"}}\n",
    )
    .unwrap();
    assert!(
        tracker.changes(records()).is_empty(),
        "late history is a baseline"
    );
    for bad in ["not JSON\n", "{\"timestamp\":\"invalid\"}\n"] {
        // A fresh observer proves each failure independently stops only the bad session.
        let mut tracker = Tracker::default();
        tracker.changes(records());
        late.append(bad);
        assert!(tracker.changes(records()).is_empty());
        assert!(tracker.sessions["project:late"].follower.is_err());
        good.append(
            "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"new prompt\"}}\n",
        );
        let notifications = tracker.changes(records());
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].session.id, "project:session");
        fs::write(late.root.join("001.jsonl"), "").unwrap();
    }
}

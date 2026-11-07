use super::*;
use crate::test_support::{remove_workspace, write_worker_meta};

struct Fixture {
    workspace: crate::test_support::TempDir,
    dir: Utf8PathBuf,
    targets: BTreeMap<String, Target>,
    output: Vec<u8>,
}

impl Fixture {
    fn poll(&mut self) -> Result<bool> {
        poll_workers(&self.workspace, &mut self.targets, &mut self.output)
    }

    fn append(&self, text: &str) {
        fs::OpenOptions::new()
            .append(true)
            .open(self.dir.join("status.log"))
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        remove_workspace(&self.workspace).unwrap();
    }
}

fn fixture(label: &str, log: &str) -> Fixture {
    let workspace = crate::test_support::temp_dir(label);
    write_worker_meta(
        &workspace,
        "worker",
        "unresolved",
        "2026-01-02T03:04:05Z".parse().unwrap(),
        None,
    );
    let dir = store::workers_dir(&workspace).unwrap().join("worker");
    fs::write(dir.join("status.log"), log).unwrap();
    Fixture {
        workspace,
        dir,
        targets: BTreeMap::new(),
        output: Vec::new(),
    }
}

#[test]
fn repeated_polls_retain_scan_offsets_without_persisting_unprinted_lines() {
    let log = "working: busy\n".repeat(10_000);
    let mut f = fixture("wait-scanned", &log);
    assert!(!f.poll().unwrap());
    assert_eq!(f.targets["worker"].scanned, log.len() as u64);
    assert_eq!(fs::read_to_string(f.dir.join("status.cursor")).unwrap(), "");

    // Make a re-read of the already scanned prefix observable without timing a large log.
    fs::OpenOptions::new()
        .write(true)
        .open(f.dir.join("status.log"))
        .unwrap()
        .write_all(b"done: old\n")
        .unwrap();
    assert!(!f.poll().unwrap());
    assert!(f.output.is_empty());
    f.append("done: new\n");
    assert!(f.poll().unwrap());
    assert_eq!(f.output, b"worker done: new\n");
}

#[test]
fn replacing_or_truncating_a_log_invalidates_its_cached_offset() {
    for (replace, lines, taken, expected) in [
        (false, 10, "", &b"worker done: new\n"[..]),
        (true, 10, "", &b"worker done: new\n"[..]),
        (true, 1, "done: taken\n", &b"worker blocked: next\n"[..]),
    ] {
        let log = "working: busy\n".repeat(lines);
        let mut f = fixture("wait-replaced", &log);
        assert!(!f.poll().unwrap());
        if replace {
            let replacement = if taken.is_empty() {
                format!("done: new\n{log}")
            } else {
                format!("{taken}blocked: next\n")
            };
            fs::write(f.dir.join("new.log"), replacement).unwrap();
            fs::rename(f.dir.join("new.log"), f.dir.join("status.log")).unwrap();
        } else {
            fs::write(f.dir.join("status.log"), "done: new\n").unwrap();
        }
        if !taken.is_empty() {
            fs::write(f.dir.join("status.cursor"), format!("{}\n", taken.len())).unwrap();
        }
        assert!(f.poll().unwrap());
        assert_eq!(f.output, expected);
    }
}

#[test]
fn skipped_closed_lines_neither_consume_a_cursor_nor_hide_a_later_report() {
    let mut f = fixture("wait-closed", "closed: old\nworking: new\n");
    assert!(!f.poll().unwrap());
    assert_eq!(fs::read_to_string(f.dir.join("status.cursor")).unwrap(), "");
    f.append("closed: again\ndone: new\nblocked: next\n");
    assert!(f.poll().unwrap());
    assert_eq!(f.output, b"worker done: new\n");
    f.output.clear();
    assert!(f.poll().unwrap());
    assert_eq!(f.output, b"worker blocked: next\n");
}

#[test]
fn an_idle_unreported_worker_is_recorded_once_after_the_grace() {
    let mut f = fixture("wait-idle-gone", "working: busy\n");
    let sessions = f.dir.join("sessions");
    fs::create_dir(&sessions).unwrap();
    fs::write(sessions.join("session.jsonl"), "{\"type\":\"message\",\"timestamp\":\"2026-10-01T16:00:00Z\",\"message\":{\"role\":\"assistant\",\"stopReason\":\"stop\"}}\n").unwrap();
    write_worker_meta(
        &f.workspace,
        "worker",
        "unresolved",
        "2026-01-02T03:04:05Z".parse().unwrap(),
        Some(crate::telemetry::SessionLink::Pi {
            session_dir: sessions,
        }),
    );
    assert!(!f.poll().unwrap());
    let mut past = super::super::liveness::Liveness::default();
    assert!(
        past.observe(
            Some(worker::WaitHealth::Idle),
            Instant::now() - Duration::from_secs(31)
        )
        .is_none()
    );
    f.targets.get_mut("worker").unwrap().liveness = past;
    assert!(f.poll().unwrap());
    assert_eq!(
        f.output,
        b"worker gone: worker idle for 30s without reporting\n"
    );
    f.targets.clear();
    f.output.clear();
    assert!(!f.poll().unwrap());
    assert!(f.output.is_empty());
}

#[test]
fn corrupt_worker_metadata_stops_the_wait_without_consuming_reports() {
    let mut f = fixture("wait-corrupt-meta", "done: pending\n");
    let broken = f.dir.parent().unwrap().join("broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("meta.json"), "{").unwrap();
    let error = f.poll().unwrap_err();
    assert!(format!("{error:#}").contains(broken.join("meta.json").as_str()));
    assert!(f.output.is_empty());
    assert!(!f.dir.join("status.cursor").exists());
    fs::remove_dir_all(broken).unwrap();
    assert!(f.poll().unwrap());
    assert_eq!(f.output, b"worker done: pending\n");
}

#[test]
fn a_later_probe_error_preserves_printed_wakes_and_unprinted_cursors() {
    let mut f = fixture("wait-partial-probe-error", "done: ready\n");
    let broken = f.dir.parent().unwrap().join("z-broken");
    let sessions = broken.join("sessions");
    fs::create_dir_all(sessions.join("unreadable.jsonl")).unwrap();
    write_worker_meta(
        &f.workspace,
        "z-broken",
        "unresolved",
        "2026-01-02T03:04:05Z".parse().unwrap(),
        Some(crate::telemetry::SessionLink::Pi {
            session_dir: sessions.clone(),
        }),
    );
    fs::write(broken.join("status.log"), "working: busy\n").unwrap();
    let error = f.poll().unwrap_err();
    assert_eq!(f.output, b"worker done: ready\n");
    assert_eq!(
        fs::read_to_string(f.dir.join("status.cursor")).unwrap(),
        "12\n"
    );
    assert_eq!(
        fs::read_to_string(broken.join("status.cursor")).unwrap(),
        ""
    );
    assert_eq!(error.to_string(), "failed to probe worker z-broken");
    assert!(format!("{error:#}").contains(sessions.join("unreadable.jsonl").as_str()));

    fs::write(broken.join("status.log"), "done: repaired\n").unwrap();
    f.targets.clear();
    f.output.clear();
    assert!(f.poll().unwrap());
    assert_eq!(f.output, b"z-broken done: repaired\n");
}

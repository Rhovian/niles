use super::*;
use crate::test_support::{remove_workspace, temp_test_path};

fn fixture(label: &str, log: &str) -> (Utf8PathBuf, Utf8PathBuf) {
    let workspace = temp_test_path(label);
    fs::create_dir_all(&workspace).unwrap();
    let dir = store::workers_dir(&workspace).unwrap().join("worker");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("meta.json"),
        serde_json::json!({
            "id": "worker", "role": "worker", "agent": "codex",
            "created_at": "2026-01-02T03:04:05Z", "project": workspace,
            "window": "test:worker", "brief": dir.join("brief.md"), "launch": dir.join("launch.sh"),
        })
        .to_string(),
    )
    .unwrap();
    fs::write(dir.join("status.log"), log).unwrap();
    (workspace, dir)
}

#[test]
fn repeated_polls_retain_scan_offsets_without_persisting_unprinted_lines() {
    let log = "working: busy\n".repeat(10_000);
    let (workspace, dir) = fixture("wait-scanned", &log);
    let mut targets = BTreeMap::new();
    let mut output = Vec::new();
    assert!(!poll_workers(&workspace, &mut targets, &mut output).unwrap());
    assert_eq!(targets["worker"].scanned, log.len() as u64);
    assert_eq!(fs::read_to_string(dir.join("status.cursor")).unwrap(), "");

    // Make a re-read of the already scanned prefix observable without timing a large log.
    fs::OpenOptions::new()
        .write(true)
        .open(dir.join("status.log"))
        .unwrap()
        .write_all(b"done: old\n")
        .unwrap();
    assert!(!poll_workers(&workspace, &mut targets, &mut output).unwrap());
    assert!(output.is_empty());
    fs::OpenOptions::new()
        .append(true)
        .open(dir.join("status.log"))
        .unwrap()
        .write_all(b"done: new\n")
        .unwrap();
    assert!(poll_workers(&workspace, &mut targets, &mut output).unwrap());
    assert_eq!(output, b"worker done: new\n");
    remove_workspace(&workspace).unwrap();
}

#[test]
fn replacing_or_truncating_a_log_invalidates_its_cached_offset() {
    for replace in [false, true] {
        let log = "working: busy\n".repeat(10);
        let (workspace, dir) = fixture("wait-replaced", &log);
        let mut targets = BTreeMap::new();
        let mut output = Vec::new();
        assert!(!poll_workers(&workspace, &mut targets, &mut output).unwrap());
        if replace {
            fs::write(dir.join("new.log"), format!("done: new\n{log}")).unwrap();
            fs::rename(dir.join("new.log"), dir.join("status.log")).unwrap();
        } else {
            fs::write(dir.join("status.log"), "done: new\n").unwrap();
        }
        assert!(poll_workers(&workspace, &mut targets, &mut output).unwrap());
        assert_eq!(output, b"worker done: new\n");
        remove_workspace(&workspace).unwrap();
    }
}

#[test]
fn skipped_closed_lines_neither_consume_a_cursor_nor_hide_a_later_report() {
    let (workspace, dir) = fixture("wait-closed", "closed: old\nworking: new\n");
    let mut targets = BTreeMap::new();
    let mut output = Vec::new();
    assert!(!poll_workers(&workspace, &mut targets, &mut output).unwrap());
    assert_eq!(fs::read_to_string(dir.join("status.cursor")).unwrap(), "");
    fs::OpenOptions::new()
        .append(true)
        .open(dir.join("status.log"))
        .unwrap()
        .write_all(b"closed: again\ndone: new\nblocked: next\n")
        .unwrap();
    assert!(poll_workers(&workspace, &mut targets, &mut output).unwrap());
    assert_eq!(output, b"worker done: new\n");
    output.clear();
    assert!(poll_workers(&workspace, &mut targets, &mut output).unwrap());
    assert_eq!(output, b"worker blocked: next\n");
    remove_workspace(&workspace).unwrap();
}

#[test]
fn replacing_a_log_keeps_the_senders_persisted_cursor_authoritative() {
    let (workspace, dir) = fixture("wait-replaced-send", "working: busy\n");
    let mut targets = BTreeMap::new();
    let mut output = Vec::new();
    assert!(!poll_workers(&workspace, &mut targets, &mut output).unwrap());
    let sent = "done: taken\n";
    fs::write(dir.join("new.log"), format!("{sent}blocked: next\n")).unwrap();
    fs::rename(dir.join("new.log"), dir.join("status.log")).unwrap();
    fs::write(dir.join("status.cursor"), format!("{}\n", sent.len())).unwrap();
    assert!(poll_workers(&workspace, &mut targets, &mut output).unwrap());
    assert_eq!(output, b"worker blocked: next\n");
    remove_workspace(&workspace).unwrap();
}

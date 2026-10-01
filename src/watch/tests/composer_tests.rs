use super::*;
use crate::agents::ComposerState;

fn pending(label: &str) -> (Utf8PathBuf, WatchMemory) {
    let root = workspace(label);
    write_worker(&root, "impl", WINDOW);
    let memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    append_log(&root, "impl", DONE);
    (root, memory)
}

fn typed() -> RecordingSink {
    RecordingSink {
        composer: ComposerState::Typed,
        ..RecordingSink::default()
    }
}

#[test]
fn typed_composer_holds_a_nudge() {
    let (root, mut memory) = pending("composer-hold");
    let mut sink = typed();
    tick(&root, at(1_000), &mut memory, &mut sink, &running());
    assert!(sink.sent.is_empty());
    tick(&root, at(1_299), &mut memory, &mut sink, &running());
    assert!(sink.sent.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn empty_composer_delivers_after_a_hold() {
    let (root, mut memory) = pending("composer-clear");
    tick(&root, at(1_000), &mut memory, &mut typed(), &running());
    let mut empty = RecordingSink {
        composer: ComposerState::Empty,
        ..RecordingSink::default()
    };
    tick(&root, at(1_001), &mut memory, &mut empty, &running());
    assert_eq!(empty.sent.len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_composer_delivers_and_resets_the_hold() {
    let (root, mut memory) = pending("composer-unknown");
    tick(&root, at(1_000), &mut memory, &mut typed(), &running());
    let mut unknown = RecordingSink::default();
    tick(&root, at(1_001), &mut memory, &mut unknown, &running());
    assert_eq!(unknown.sent.len(), 1);
    append_log(&root, "impl", DONE);
    let mut again = typed();
    tick(&root, at(1_299), &mut memory, &mut again, &running());
    assert!(again.sent.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ceiling_delivers_all_pending_nudges_under_a_persistent_typed_reading() {
    let root = workspace("composer-ceiling");
    write_worker(&root, "alpha", WINDOW);
    write_worker(&root, "beta", WINDOW);
    let mut memory = WatchMemory::at_start(&worker_snapshot(&root).unwrap());
    append_log(&root, "alpha", DONE);
    append_log(&root, "beta", DONE);
    let mut sink = typed();
    tick(&root, at(1_000), &mut memory, &mut sink, &running());
    assert!(sink.sent.is_empty());
    tick(&root, at(1_300), &mut memory, &mut sink, &running());
    assert_eq!(sink.sent.len(), 2);
    append_log(&root, "alpha", DONE);
    tick(&root, at(1_301), &mut memory, &mut sink, &running());
    assert_eq!(
        sink.sent.len(),
        3,
        "the ceiling persists until the composer clears"
    );
    fs::remove_dir_all(root).unwrap();
}

impl Default for RecordingSink {
    fn default() -> Self {
        Self {
            attempts: Vec::new(),
            sent: Vec::new(),
            captures: Vec::new(),
            notes: Vec::new(),
            fail: false,
            capture_failure: false,
            screen: String::new(),
            composer: ComposerState::Unknown,
        }
    }
}

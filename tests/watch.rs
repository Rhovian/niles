#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "watch/activity.rs"]
mod activity;
mod common;

use common::*;
use std::{fs, path::PathBuf, process::Output};

/// A tmux stub that records its calls, models the pane `send_line` watches, and prints chatter on
/// every command whose output niles does not read.
///
/// The chatter is the point: niles runs tmux from inside the lead's own process, so a call that
/// inherits stdout would put a line about windows and panes in the middle of the lead's screen.
/// `capture-pane` and `display-message` are answered silently because their output *is* read.
const STUB_TMUX: &str = r#"#!/bin/sh
[ "$1" != -u ] || shift
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
  display-message) printf 'niles-test-session\n'; exit 0 ;;
  list-windows)
    if [ "$2" = "-a" ]; then
      exit 0
    fi
    if [ -n "${TMUX_WINDOWS:-}" ]; then
      printf '%s\n' "$TMUX_WINDOWS"
    fi
    exit 0
    ;;
  send-keys)
    printf 'tmux chatter on a call nobody reads\n'
    if [ "$4" = "-l" ]; then
      printf 'composer: %s\n' "$5" >> "$TMUX_PANE_FILE"
      # A worker that reports while the message is still being typed, which is the window between
      # reading the check-in baseline and arming it.
      if [ -n "${TMUX_WORKER_STATUS:-}" ]; then
        printf 'done: reported while the send was in flight\n' >> "$TMUX_WORKER_STATUS"
      fi
    else
      printf 'submitted\n' >> "$TMUX_PANE_FILE"
    fi
    exit 0
    ;;
  capture-pane)
    if [ -f "$TMUX_PANE_FILE" ]; then cat "$TMUX_PANE_FILE"; fi
    exit 0
    ;;
  *)
    printf 'tmux chatter on a call nobody reads\n'
    exit 0
    ;;
esac
"#;

struct Fixture {
    env: TestEnv,
    pane_file: PathBuf,
}

fn fixture(prefix: &str) -> Fixture {
    let env = TestEnv::with_tmux(prefix, STUB_TMUX);
    write_workspace_manifest(&env.root, "claude", "claude", "claude", "claude");

    Fixture {
        pane_file: env.root.join("pane.txt"),
        env,
    }
}

impl Fixture {
    fn niles(&self, args: &[&str]) -> Output {
        let output = self
            .env
            .niles(&self.env.root, args)
            .env("TMUX_PANE_FILE", &self.pane_file)
            .env("TMUX_PANE", "%9")
            // Where the stub tmux writes the report that lands mid-send.
            .env("TMUX_WORKER_STATUS", self.status_log("impl"))
            .output()
            .unwrap();
        assert!(
            !stdout_of(&output).contains("tmux chatter"),
            "{args:?}: {}",
            stdout_of(&output)
        );
        assert!(
            !stderr_of(&output).contains("tmux chatter"),
            "{args:?}: {}",
            stderr_of(&output)
        );
        output
    }

    /// Keep `--checkin` after the worker id: clap hands that position over as task text.
    fn spawn(&self, id: &str, delay: Option<&str>) -> Output {
        let mut args = vec![
            "spawn",
            "--skill",
            "research",
            id,
            "--agent",
            "claude:sonnet:high",
        ];
        if let Some(delay) = delay {
            args.extend(["--checkin", delay]);
        }
        args.extend(["Fix", "auth"]);
        self.niles(&args)
    }

    fn checkin_path(&self, id: &str) -> PathBuf {
        self.worker_file(id, "checkin")
    }

    fn checkin(&self, id: &str) -> String {
        fs::read_to_string(self.checkin_path(id)).unwrap()
    }

    /// The workspace manifest with top-level `checkin`/`recheck` settings: the
    /// cadence a test is about, and nothing else changed.
    fn write_manifest(&self, keys: &str) {
        write_workspace_manifest(&self.env.root, "claude", "claude", "claude", "claude");
        edit_manifest(&self.env.root, |body| format!("{body}{keys}"));
    }

    fn status_log(&self, id: &str) -> PathBuf {
        self.worker_file(id, "status.log")
    }

    fn worker_file(&self, id: &str, name: &str) -> PathBuf {
        state_dir(&self.env.root).join("worker").join(id).join(name)
    }
}

/// The lead arms a check-in with the dispatch, and `quiet` is how it takes one back.
#[test]
fn spawn_arms_a_check_in_and_quiet_disarms_it() {
    let fixture = fixture("niles-watch-quiet");

    let spawn = fixture.spawn("impl", None);
    assert_command_success("spawn", &spawn);
    let printed = stdout_of(&spawn);
    assert!(printed.contains("checkin: 5m"), "{printed}");
    assert!(printed.contains("quiet: niles quiet impl"), "{printed}");

    let body = fixture.checkin("impl");
    // A worker that has not written a line yet: nothing in its log can answer this check-in.
    assert!(body.contains("armed_len=0"), "{body}");
    assert!(body.contains("deadline="), "{body}");
    // The cadence is in the state, not re-read from a manifest on every tick: what this dispatch
    // waits, and what a fire does next.
    assert!(body.contains("delay=300"), "{body}");
    assert!(body.contains("recheck=backoff"), "{body}");

    let quiet = fixture.niles(&["quiet", "impl"]);
    assert_success_contains("quiet", &quiet, "quiet: impl");
    assert!(
        !fixture.checkin_path("impl").exists(),
        "quiet is the lead disarming a check-in by hand"
    );

    // A second quiet has nothing to do and is not an error.
    let again = fixture.niles(&["quiet", "impl"]);
    assert_success_contains("quiet again", &again, "quiet: impl (no check-in armed)");
}

/// Every `--checkin` spelling the help offers, through the two commands that arm one.
#[test]
fn the_checkin_flag_arms_the_delay_it_was_given() {
    let fixture = fixture("niles-watch-forms");

    let seconds = fixture.spawn("impl", Some("90s"));
    assert_success_contains("spawn --checkin 90s", &seconds, "checkin: 90s");
    // The flag was written after the worker id, where clap hands it over as task text: it must not
    // reach the worker's brief.
    let brief =
        fs::read_to_string(state_dir(&fixture.env.root).join("worker/impl/brief.md")).unwrap();
    assert!(brief.contains("Fix auth"), "{brief}");
    assert!(!brief.contains("--checkin"), "{brief}");

    // `send` is an assignment too, so it arms the same state.
    let send = fixture.niles(&["send", "impl", "--checkin", "1h", "again"]);
    assert_command_success("send --checkin 1h", &send);
    assert!(
        fixture.checkin("impl").contains("delay=3600"),
        "{}",
        fixture.checkin("impl")
    );
    // The flag came after the worker id here too, so the message that was typed is the message.
    let log = fixture.env.tmux_log();
    assert!(
        log.contains("send-keys -t =niles-test-session:=niles-impl -l \x1b[200~again"),
        "the flag must not reach the worker as message text:\n{log}"
    );

    assert!(fixture.checkin_path("impl").exists());
    let off = fixture.niles(&["send", "impl", "--checkin", "off", "never mind"]);
    assert_success_contains("send --checkin off", &off, "checkin: off");
    assert!(
        !fixture.checkin_path("impl").exists(),
        "the armed deadline must go with the printed `checkin: off`"
    );

    // And `off` arms none at all.
    let off = fixture.spawn("docs", Some("off"));
    assert_success_contains("spawn --checkin off", &off, "checkin: off");
    assert!(!fixture.checkin_path("docs").exists());
}

/// The baseline a check-in is armed with has to be read *before* the message is typed. A report
/// that lands in the send is the worker answering this assignment; a baseline taken afterwards
/// would fold that line into the arithmetic and leave nothing able to answer it.
///
/// The stub tmux writes that report on the paste, which is exactly the window in question.
#[test]
fn a_report_during_the_send_still_answers_the_assignment_it_belongs_to() {
    let fixture = fixture("niles-watch-race");
    let spawn = fixture.spawn("impl", None);
    assert_command_success("spawn", &spawn);
    let status = fixture.status_log("impl");
    fs::write(&status, "working: launch\n").unwrap();
    let baseline = fs::read_to_string(&status).unwrap().len();

    let send = fixture.niles(&["send", "impl", "continue"]);
    assert_success_contains("send", &send, "sent: impl");
    assert!(
        fixture
            .env
            .tmux_log()
            .contains("send-keys -t =niles-test-session:=niles-impl -l \x1b[200~continue")
    );

    // The report did land while the send was in flight, and it is past the baseline.
    let body = fs::read_to_string(&status).unwrap();
    assert!(
        body.contains("done: reported while the send was in flight"),
        "{body}"
    );
    assert!(body.len() > baseline, "{body}");

    // So the armed length is the pre-send one, and that line can answer the assignment that caused
    // it. Read after the send it would be the longer length, and the report would answer nothing.
    let checkin = fixture.checkin("impl");
    assert!(
        checkin.contains(&format!("armed_len={baseline}")),
        "armed_len must be the length read before the send ({baseline} bytes): {checkin}"
    );
}

#[test]
fn an_omitted_checkin_flag_arms_the_manifests_default() {
    let fixture = fixture("niles-watch-manifest-checkin");
    fixture.write_manifest("checkin: 15m\nrecheck: 30m\n");

    let spawn = fixture.spawn("impl", None);
    assert_command_success("spawn", &spawn);
    let printed = stdout_of(&spawn);
    assert!(printed.contains("checkin: 15m"), "{printed}");

    let body = fixture.checkin("impl");
    assert!(body.contains("delay=900"), "{body}");
    assert!(body.contains("recheck=30m"), "{body}");

    // The flag decides the delay and the manifest still decides the policy behind it.
    let flagged = fixture.niles(&["send", "impl", "--checkin", "90s", "carry on"]);
    assert_success_contains("send --checkin 90s", &flagged, "checkin: 90s");
    let body = fixture.checkin("impl");
    assert!(body.contains("delay=90"), "{body}");
    assert!(body.contains("recheck=30m"), "{body}");
}

#[test]
fn malformed_checkin_values_fail_before_dispatch() {
    for (label, manifest, flag, errors) in [
        (
            "flag",
            "",
            Some("soon"),
            &["invalid `--checkin`", "is not a duration"] as &[&str],
        ),
        (
            "checkin",
            "checkin: soon\n",
            None,
            &["checkin", "manifest.yaml"],
        ),
        (
            "recheck",
            "recheck: soon\n",
            None,
            &["recheck", "manifest.yaml"],
        ),
        (
            "bare",
            "checkin: 15\n",
            None,
            &["`15` needs a unit: ms, s, m or h", "manifest.yaml"],
        ),
    ] {
        let fixture = fixture(label);
        fixture.write_manifest(manifest);
        let spawn = fixture.spawn("impl", flag);
        for error in errors {
            assert_failure_contains(label, &spawn, error);
        }
        assert!(!fixture.checkin_path("impl").exists(), "{label}");
        assert!(
            !state_dir(&fixture.env.root).join("worker/impl").exists(),
            "{label}"
        );
        assert!(!fixture.env.tmux_log.exists(), "{label}");
    }
}

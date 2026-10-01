#![allow(clippy::unwrap_used, clippy::expect_used)]

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
        self.env
            .niles(&self.env.root, args)
            .env("TMUX_PANE_FILE", &self.pane_file)
            .env("TMUX_PANE", "%9")
            // Where the stub tmux writes the report that lands mid-send.
            .env("TMUX_WORKER_STATUS", self.status_log("impl"))
            .output()
            .unwrap()
    }

    /// `niles spawn <id>` with the default check-in: the dispatch most tests start from, and the
    /// one whose output they read.
    fn spawn(&self, id: &str) -> Output {
        self.niles(&["spawn", id, "--agent", "claude", "Fix", "auth"])
    }

    /// The same dispatch with the `--checkin` spelling under test. The flag is written after the
    /// worker id on purpose: that is where clap hands it over as trailing task text.
    fn spawn_with_checkin(&self, id: &str, delay: &str) -> Output {
        self.niles(&[
            "spawn",
            id,
            "--agent",
            "claude",
            "--checkin",
            delay,
            "Fix",
            "auth",
        ])
    }

    fn checkin_path(&self, id: &str) -> PathBuf {
        self.worker_file(id, "checkin")
    }

    fn checkin(&self, id: &str) -> String {
        fs::read_to_string(self.checkin_path(id)).unwrap()
    }

    /// The workspace manifest with extra keys under the role bindings: the `checkin`/`recheck`
    /// cadence a test is about, and nothing else changed.
    fn write_manifest(&self, keys: &str) {
        fs::write(
            self.env.root.join(".niles/manifest.yaml"),
            format!(
                "lead: claude\nworker: claude\nreviewer: claude\nsecurity: claude\n{keys}niles_schema: 2\n"
            ),
        )
        .unwrap();
    }

    fn status_log(&self, id: &str) -> PathBuf {
        self.worker_file(id, "status.log")
    }

    fn worker_file(&self, id: &str, name: &str) -> PathBuf {
        self.env.root.join(".niles/worker").join(id).join(name)
    }
}

/// The lead arms a check-in with the dispatch, and `quiet` is how it takes one back.
#[test]
fn spawn_arms_a_check_in_and_quiet_disarms_it() {
    let fixture = fixture("niles-watch-quiet");

    let spawn = fixture.spawn("impl");
    assert_command_success("spawn", &spawn);
    let printed = stdout_of(&spawn);
    assert!(printed.contains("checkin: 5m"), "{printed}");
    assert!(printed.contains("quiet: niles quiet impl"), "{printed}");

    let body = fixture.checkin("impl");
    assert!(body.contains("step=300"), "{body}");
    // A worker that has not written a line yet: nothing in its log can answer this check-in.
    assert!(body.contains("armed_len=0"), "{body}");
    assert!(body.contains("deadline="), "{body}");
    // The cadence is in the state, not re-read from a manifest on every tick: what this dispatch
    // waits, and what a fire does next.
    assert!(body.contains("delay=300"), "{body}");
    assert!(body.contains("recheck=backoff"), "{body}");

    let quiet = fixture.niles(&["quiet", "impl"]);
    assert_command_success("quiet", &quiet);
    assert!(
        stdout_of(&quiet).contains("quiet: impl"),
        "{}",
        stdout_of(&quiet)
    );
    assert!(
        !fixture.checkin_path("impl").exists(),
        "quiet is the lead disarming a check-in by hand"
    );

    // A second quiet has nothing to do and is not an error.
    let again = fixture.niles(&["quiet", "impl"]);
    assert_command_success("quiet again", &again);
    assert!(
        stdout_of(&again).contains("quiet: impl (no check-in armed)"),
        "{}",
        stdout_of(&again)
    );
}

/// Every `--checkin` spelling the help offers, through the two commands that arm one.
#[test]
fn the_checkin_flag_arms_the_delay_it_was_given() {
    let fixture = fixture("niles-watch-forms");

    let seconds = fixture.spawn_with_checkin("impl", "90s");
    assert_command_success("spawn --checkin 90s", &seconds);
    assert!(
        stdout_of(&seconds).contains("checkin: 90s"),
        "{}",
        stdout_of(&seconds)
    );
    assert!(fixture.checkin("impl").contains("step=90"));
    // The flag was written after the worker id, where clap hands it over as task text: it must not
    // reach the worker's brief.
    let brief = fs::read_to_string(fixture.env.root.join(".niles/worker/impl/brief.md")).unwrap();
    assert!(brief.contains("Fix auth"), "{brief}");
    assert!(!brief.contains("--checkin"), "{brief}");

    // A bare number is minutes, the way the help words it.
    let minutes = fixture.spawn_with_checkin("review", "2");
    assert_command_success("spawn --checkin 2", &minutes);
    assert!(
        stdout_of(&minutes).contains("checkin: 2m"),
        "{}",
        stdout_of(&minutes)
    );

    // `send` is an assignment too, so it arms the same state.
    let send = fixture.niles(&["send", "impl", "--checkin", "1h", "again"]);
    assert_command_success("send --checkin 1h", &send);
    assert!(
        fixture.checkin("impl").contains("step=3600"),
        "{}",
        fixture.checkin("impl")
    );
    // The flag came after the worker id here too, so the message that was typed is the message.
    let log = fixture.env.tmux_log();
    assert!(
        log.contains("send-keys -t =niles-test-session:=niles-impl -l again"),
        "the flag must not reach the worker as message text:\n{log}"
    );

    // And `off` arms none at all.
    let off = fixture.spawn_with_checkin("docs", "off");
    assert_command_success("spawn --checkin off", &off);
    assert!(
        stdout_of(&off).contains("checkin: off"),
        "{}",
        stdout_of(&off)
    );
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
    let spawn = fixture.spawn("impl");
    assert_command_success("spawn", &spawn);
    let status = fixture.status_log("impl");
    fs::write(&status, "working: launch\n").unwrap();
    let baseline = fs::read_to_string(&status).unwrap().len();

    let send = fixture.niles(&["send", "impl", "carry on"]);
    assert_command_success("send", &send);

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

/// `--checkin off` has to mean off: a printed `checkin: off` over a check-in that is still armed
/// and due would be a lie the lead acts on.
#[test]
fn asking_for_no_check_in_takes_an_armed_one_with_it() {
    let fixture = fixture("niles-watch-off");
    let spawn = fixture.spawn("impl");
    assert_command_success("spawn", &spawn);
    assert!(fixture.checkin_path("impl").exists());

    let off = fixture.niles(&["send", "impl", "--checkin", "off", "never mind"]);
    assert_command_success("send --checkin off", &off);
    assert!(
        stdout_of(&off).contains("checkin: off"),
        "{}",
        stdout_of(&off)
    );
    assert!(
        !fixture.checkin_path("impl").exists(),
        "the armed deadline must go with the printed `checkin: off`"
    );
}

/// The watcher types into the lead's pane from inside the lead's own process, so every tmux call
/// niles makes on a worker's behalf has to keep its output to itself.
#[test]
fn a_chatty_tmux_never_reaches_the_lead_streams() {
    let fixture = fixture("niles-watch-chatter");

    let spawn = fixture.spawn("impl");
    assert_command_success("spawn", &spawn);
    // `spawn` drives tmux windows: every one of those calls is mergeable into the lead's screen.
    assert!(
        !stdout_of(&spawn).contains("tmux chatter"),
        "tmux output reached stdout:\n{}",
        stdout_of(&spawn)
    );
    assert!(
        !String::from_utf8_lossy(&spawn.stderr).contains("tmux chatter"),
        "tmux output reached stderr"
    );

    let send = fixture.niles(&["send", "impl", "continue"]);
    assert_command_success("send", &send);
    assert!(
        !stdout_of(&send).contains("tmux chatter"),
        "tmux output reached stdout:\n{}",
        stdout_of(&send)
    );
    // The send still told the lead what it did.
    assert!(
        stdout_of(&send).contains("sent: impl"),
        "{}",
        stdout_of(&send)
    );

    // The message really was typed into the worker's pane, so the chatter test is not passing
    // because nothing ran.
    let log = fixture.env.tmux_log();
    assert!(
        log.contains("send-keys -t =niles-test-session:=niles-impl -l continue"),
        "{log}"
    );
}

/// An unreadable `--checkin` is refused before anything is dispatched, rather than silently
/// arming the default.
#[test]
fn a_malformed_checkin_flag_fails_the_dispatch() {
    let fixture = fixture("niles-watch-bad-flag");

    let spawn = fixture.spawn_with_checkin("impl", "soon");

    assert!(!spawn.status.success());
    let stderr = String::from_utf8_lossy(&spawn.stderr);
    assert!(stderr.contains("is not a duration"), "{stderr}");
    assert!(!fixture.checkin_path("impl").exists());
}

/// The manifest's `checkin:` is the workspace's default first delay. A dispatch that names no
/// `--checkin` arms it, `recheck:` travels with the arm state, and the flag still wins over it.
#[test]
fn an_omitted_checkin_flag_arms_the_manifests_default() {
    let fixture = fixture("niles-watch-manifest-checkin");
    fixture.write_manifest("checkin: 15m\nrecheck: 30m\n");

    let spawn = fixture.spawn("impl");
    assert_command_success("spawn", &spawn);
    let printed = stdout_of(&spawn);
    assert!(printed.contains("checkin: 15m"), "{printed}");

    let body = fixture.checkin("impl");
    assert!(body.contains("delay=900"), "{body}");
    assert!(body.contains("step=900"), "{body}");
    assert!(body.contains("recheck=30m"), "{body}");

    // The flag decides the delay and the manifest still decides the policy behind it.
    let flagged = fixture.niles(&["send", "impl", "--checkin", "90s", "carry on"]);
    assert_command_success("send --checkin 90s", &flagged);
    assert!(
        stdout_of(&flagged).contains("checkin: 90s"),
        "{}",
        stdout_of(&flagged)
    );
    let body = fixture.checkin("impl");
    assert!(body.contains("delay=90"), "{body}");
    assert!(body.contains("step=90"), "{body}");
    assert!(body.contains("recheck=30m"), "{body}");
}

/// A malformed value in either check-in key fails the dispatch, naming the file to fix, rather than
/// quietly arming the built-in cadence the workspace just tried to change — and it fails before a
/// window is launched or a worker directory is written.
#[test]
fn a_malformed_manifest_checkin_key_fails_the_dispatch() {
    for (key, manifest) in [
        ("checkin", "checkin: soon\n"),
        ("recheck", "recheck: soon\n"),
    ] {
        let fixture = fixture(&format!("niles-watch-manifest-bad-{key}"));
        fixture.write_manifest(manifest);

        let spawn = fixture.spawn("impl");

        assert!(!spawn.status.success(), "{key} was accepted");
        let stderr = String::from_utf8_lossy(&spawn.stderr);
        assert!(stderr.contains(key), "{stderr}");
        assert!(
            stderr.contains(".niles/manifest.yaml"),
            "the error must name the file: {stderr}"
        );
        assert!(
            !fixture.env.root.join(".niles/worker/impl").exists(),
            "nothing was dispatched, so nothing was written"
        );
        // Nothing reached tmux either: a spawn that never got as far as launching a window leaves
        // no tmux log at all.
        assert!(!fixture.env.tmux_log.exists());
    }
}

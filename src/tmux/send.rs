use std::{
    fmt, thread,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};

use super::TmuxTarget;

const SEND_LINE_SUBMIT_KEY: &str = "C-m";

/// Pane rows captured while watching a send land. The composer sits at the bottom of the pane, so
/// this only has to be deep enough to see it fill and empty.
const SEND_WATCH_LINES: usize = 50;

struct Timings {
    /// How often the pane is re-captured while watching a send land.
    poll_interval: Duration,

    /// How long the pane must hold still before the paste counts as ingested.
    ///
    /// A TUI does not redraw once per keystroke, it redraws in bursts: a 6KB message handed to
    /// hermes moved the pane at 69ms, 193ms, 313ms, 554ms, 799ms and 923ms, with up to 245ms of
    /// stillness between bursts. "Unchanged since the last poll" is therefore not quiet — it is
    /// the gap between two bursts, and a submit sent into one is the swallowed submit this whole
    /// dance exists to avoid. The window is wider than the widest observed gap, and it is a
    /// floor on how long to keep looking rather than a guess at how long ingestion takes: the wait
    /// ends when the pane stops moving, however long that takes.
    quiet_window: Duration,

    /// Longest wait for a pasted message to render and go quiet before the submit key is sent.
    /// Bounded because a pane with an animation on it never goes quiet, and a send must not hang.
    settle_timeout: Duration,

    /// Longest wait for the pane to change after the submit key. A pane that never changes is what
    /// a swallowed submit looks like, and reporting that is the whole point of watching.
    submit_timeout: Duration,
}

const REAL_TIMINGS: Timings = Timings {
    poll_interval: Duration::from_millis(50),
    quiet_window: Duration::from_millis(500),
    settle_timeout: Duration::from_secs(6),
    submit_timeout: Duration::from_secs(3),
};

trait Pane: fmt::Display {
    fn leave_copy_mode(&self) -> Result<()>;
    fn capture_watched_lines(&self) -> Result<String>;
    fn paste_literal(&self, line: &str) -> Result<()>;
    fn send_submit_key(&self) -> Result<()>;
}

impl Pane for TmuxTarget {
    fn leave_copy_mode(&self) -> Result<()> {
        super::run(["copy-mode", "-q", "-t", self.as_str()])
    }

    fn capture_watched_lines(&self) -> Result<String> {
        super::capture_pane(self, SEND_WATCH_LINES)
    }

    fn paste_literal(&self, line: &str) -> Result<()> {
        super::run(send_line_literal_args(self.as_str(), line))
    }

    fn send_submit_key(&self) -> Result<()> {
        super::run(send_line_submit_args(self.as_str()))
    }
}

/// Sends a message to a pane and confirms it left the composer.
///
/// The text and the submit key are two separate tmux calls, and a TUI still ingesting a few KB of
/// pasted text swallows a `C-m` that arrives mid-reflow: the message then sits unsent in the
/// composer while niles reports success, which is the one direction this must never fail in. So
/// the pane is watched rather than timed. The paste is given until it renders and goes quiet
/// before the submit is sent, and the pane must change after it — a submit that changes nothing
/// is an error, not a `sent:`.
pub(crate) fn send_line(target: &TmuxTarget, line: &str) -> Result<()> {
    send_line_to_pane(target, line, &REAL_TIMINGS)
}

fn send_line_to_pane<P: Pane>(pane: &P, line: &str, timings: &Timings) -> Result<()> {
    pane.leave_copy_mode()?;
    let before = pane.capture_watched_lines()?;
    pane.paste_literal(line)?;
    let staged = settle_pane(pane, &before, timings)?;
    pane.send_submit_key()?;
    confirm_submit_took(pane, &staged, timings)
}

/// Waits for the paste to render and the pane to go quiet, and returns what it settled on.
///
/// A pane that never goes quiet may be an agent already doing something, so the submit is still
/// worth sending and [`confirm_submit_took`] judges whether it took. If the text never renders,
/// the pane may not be accepting input and the submit must not be sent blindly.
fn settle_pane<P: Pane>(pane: &P, before: &str, timings: &Timings) -> Result<String> {
    let deadline = Instant::now() + timings.settle_timeout;
    let mut previous = before.to_owned();
    let mut rendered = false;
    let mut quiet_since = None;
    loop {
        thread::sleep(timings.poll_interval);
        let current = pane.capture_watched_lines()?;
        if current == previous {
            let since = quiet_since.get_or_insert_with(Instant::now);
            if rendered && since.elapsed() >= timings.quiet_window {
                return Ok(current);
            }
        } else {
            rendered = true;
            quiet_since = None;
            previous = current;
        }
        if Instant::now() >= deadline {
            if !rendered {
                bail!(
                    "message text never appeared in {pane}: the pane may not be accepting input; \
                     the submit key was not sent"
                );
            }
            return Ok(previous);
        }
    }
}

/// Fails unless the pane changes after the submit key.
///
/// The composer emptying, the message appearing in the transcript, the agent starting to think —
/// any of them move the pane. Nothing moving is the observed failure: the text still sitting in
/// the composer, with the worker idle and niles about to call it sent. This is only as good as
/// the quiet `staged` was captured in — against a pane that was still moving when
/// [`settle_pane`] gave up, the next redraw counts as a change and the check passes for the
/// wrong reason.
fn confirm_submit_took<P: Pane>(pane: &P, staged: &str, timings: &Timings) -> Result<()> {
    let deadline = Instant::now() + timings.submit_timeout;
    loop {
        thread::sleep(timings.poll_interval);
        if pane.capture_watched_lines()? != staged {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "message was typed into {pane} but the submit key did not take: the pane has not \
                 changed in {}s. The text is most likely still sitting unsent in the composer — \
                 check it with `niles peek`, then send again once the pane is idle.",
                timings.submit_timeout.as_secs()
            );
        }
    }
}

fn send_line_literal_args<'a>(target: &'a str, line: &'a str) -> [&'a str; 5] {
    ["send-keys", "-t", target, "-l", line]
}

fn send_line_submit_args(target: &str) -> [&str; 4] {
    ["send-keys", "-t", target, SEND_LINE_SUBMIT_KEY]
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
    };

    use super::*;

    const TEST_TIMINGS: Timings = Timings {
        poll_interval: Duration::from_millis(1),
        quiet_window: Duration::from_millis(2),
        settle_timeout: Duration::from_millis(6),
        submit_timeout: Duration::from_millis(4),
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Phase {
        Initial,
        Pasted,
        Submitted,
    }

    struct ScriptedPane {
        phase: Cell<Phase>,
        actions: RefCell<Vec<&'static str>>,
        pasted_frames: RefCell<VecDeque<String>>,
        cycle_frames: bool,
        submitted_frame: String,
        rendered_at: Cell<Option<Instant>>,
        submit_at: Cell<Option<Instant>>,
    }

    impl ScriptedPane {
        fn new(pasted_frames: &[&str], cycle_frames: bool, submitted_frame: &str) -> Self {
            assert!(!pasted_frames.is_empty());
            Self {
                phase: Cell::new(Phase::Initial),
                actions: RefCell::new(Vec::new()),
                pasted_frames: RefCell::new(
                    pasted_frames
                        .iter()
                        .map(|frame| (*frame).to_owned())
                        .collect(),
                ),
                cycle_frames,
                submitted_frame: submitted_frame.to_owned(),
                rendered_at: Cell::new(None),
                submit_at: Cell::new(None),
            }
        }

        fn actions(&self) -> Vec<&'static str> {
            self.actions.borrow().clone()
        }
    }

    impl fmt::Display for ScriptedPane {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("fake-pane")
        }
    }

    impl Pane for ScriptedPane {
        fn leave_copy_mode(&self) -> Result<()> {
            self.actions.borrow_mut().push("leave-mode");
            Ok(())
        }

        fn capture_watched_lines(&self) -> Result<String> {
            match self.phase.get() {
                Phase::Initial => Ok("before".to_owned()),
                Phase::Pasted => {
                    let mut frames = self.pasted_frames.borrow_mut();
                    let frame = if self.cycle_frames {
                        let frame = frames.pop_front().expect("constructor requires frames");
                        frames.push_back(frame.clone());
                        frame
                    } else if frames.len() > 1 {
                        frames.pop_front().expect("constructor requires frames")
                    } else {
                        frames.front().expect("constructor requires frames").clone()
                    };
                    if frame != "before" && self.rendered_at.get().is_none() {
                        self.rendered_at.set(Some(Instant::now()));
                    }
                    Ok(frame)
                }
                Phase::Submitted => Ok(self.submitted_frame.clone()),
            }
        }

        fn paste_literal(&self, _line: &str) -> Result<()> {
            self.actions.borrow_mut().push("paste");
            self.phase.set(Phase::Pasted);
            Ok(())
        }

        fn send_submit_key(&self) -> Result<()> {
            self.actions.borrow_mut().push("submit");
            self.submit_at.set(Some(Instant::now()));
            self.phase.set(Phase::Submitted);
            Ok(())
        }
    }

    #[test]
    fn rendered_quiet_paste_is_submitted_and_confirmed() {
        let pane = ScriptedPane::new(&["staged"], false, "changed");

        send_line_to_pane(&pane, "hello", &TEST_TIMINGS).unwrap();

        assert_eq!(pane.actions(), ["leave-mode", "paste", "submit"]);
    }

    #[test]
    fn submit_waits_for_the_quiet_window() {
        let timings = Timings {
            poll_interval: Duration::from_millis(1),
            quiet_window: Duration::from_millis(20),
            settle_timeout: Duration::from_millis(100),
            submit_timeout: Duration::from_millis(10),
        };
        let pane = ScriptedPane::new(&["staged"], false, "changed");

        send_line_to_pane(&pane, "hello", &timings).unwrap();

        let rendered_at = pane.rendered_at.get().expect("paste should render");
        let submit_at = pane.submit_at.get().expect("submit should be sent");
        assert!(submit_at.duration_since(rendered_at) >= timings.quiet_window);
    }

    #[test]
    fn paste_that_never_renders_is_not_submitted() {
        let pane = ScriptedPane::new(&["before"], false, "changed");

        let error = send_line_to_pane(&pane, "hello", &TEST_TIMINGS).unwrap_err();

        assert!(error.to_string().contains("never appeared"));
        assert_eq!(pane.actions(), ["leave-mode", "paste"]);
    }

    #[test]
    fn unchanged_pane_after_submit_reports_how_to_inspect_it() {
        let pane = ScriptedPane::new(&["staged"], false, "staged");

        let error = send_line_to_pane(&pane, "hello", &TEST_TIMINGS).unwrap_err();

        assert!(error.to_string().contains("`niles peek`"));
        assert_eq!(pane.actions(), ["leave-mode", "paste", "submit"]);
    }

    #[test]
    fn rendered_paste_that_never_quiets_is_still_submitted() {
        let pane = ScriptedPane::new(&["staged", "moving"], true, "changed");

        send_line_to_pane(&pane, "hello", &TEST_TIMINGS).unwrap();

        assert_eq!(pane.actions(), ["leave-mode", "paste", "submit"]);
    }

    #[test]
    fn literal_args_preserve_multiline_message_as_one_argument() {
        assert_eq!(
            send_line_literal_args("niles:step", "line 1\nline 2"),
            ["send-keys", "-t", "niles:step", "-l", "line 1\nline 2"]
        );
    }

    #[test]
    fn submit_args_use_discrete_control_m() {
        assert_eq!(
            send_line_submit_args("niles:step"),
            ["send-keys", "-t", "niles:step", SEND_LINE_SUBMIT_KEY]
        );
    }
}

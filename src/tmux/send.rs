use crate::{store::Scope, tmux::TmuxTarget};
use std::{
    fmt, thread,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};

const PANE_IDENTITY: &str = "#{socket_path}:#{pane_id}";
const SEND_LINE_SUBMIT_KEY: &str = "C-m";

/// Pane rows captured while watching a send land. The composer sits at the bottom of the pane, so
/// this only has to be deep enough to see it fill and empty.
const SEND_WATCH_LINES: usize = 50;

struct Timings {
    now: fn() -> Instant,
    /// How often the pane is re-captured while watching a send land.
    poll_interval: Duration,

    /// How long the pane must hold still before the paste counts as ingested.
    ///
    /// Wider than the widest observed inter-burst gap (245ms); a floor, not a guess.
    quiet_window: Duration,

    /// Longest wait for a pasted message to render and go quiet before the submit key is sent.
    /// Bounded because a pane with an animation on it never goes quiet, and a send must not hang.
    settle_timeout: Duration,

    /// Longest wait for the pane to change after the submit key. A pane that never changes is what
    /// a swallowed submit looks like, and reporting that is the whole point of watching.
    submit_timeout: Duration,
}

const REAL_TIMINGS: Timings = Timings {
    now: Instant::now,
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
        super::run(&["copy-mode", "-q", "-t", self.as_str()])
    }

    fn capture_watched_lines(&self) -> Result<String> {
        super::capture_pane(self, SEND_WATCH_LINES)
    }

    fn paste_literal(&self, line: &str) -> Result<()> {
        if line.contains('\x1b') {
            bail!("message contains an escape character; cannot deliver it as a bracketed paste");
        }
        // tmux 3.4 cmd-paste-buffer.c adds framing with -p only when the pane requests it;
        // format.c exposes no bracket_paste_flag, so that state cannot be checked there.
        // Frame explicitly to avoid an unframed fallback on every supported tmux version.
        // https://github.com/tmux/tmux/blob/3.4/cmd-paste-buffer.c
        let paste = format!("\x1b[200~{line}\x1b[201~");
        super::run(&["send-keys", "-t", self.as_str(), "-l", &paste])
    }

    fn send_submit_key(&self) -> Result<()> {
        super::run(&["send-keys", "-t", self.as_str(), SEND_LINE_SUBMIT_KEY])
    }
}

/// Pastes a message to a pane and watches for a submit acknowledgement.
///
/// The paste and the submit key are separate tmux calls, and a TUI still ingesting a few KB of
/// pasted text swallows a `C-m` that arrives mid-reflow: the message then sits unsent in the
/// composer while niles reports success, which is the one direction this must never fail in. So
/// the pane is watched rather than timed. The paste is given until it renders and goes quiet
/// before the submit is sent, and the pane must change after it — a submit that changes nothing
/// is an error, not a `sent:`.
pub(crate) fn send_line(target: &TmuxTarget, line: &str) -> Result<()> {
    let identity = ["display", "-pt", target.as_str(), PANE_IDENTITY];
    let pane = super::query(&identity)?;
    let _lock = Scope::Pane(pane.trim()).reserve()?;
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
    let deadline = (timings.now)() + timings.settle_timeout;
    let mut previous = before.to_owned();
    let mut rendered = false;
    let mut quiet_since = None;
    loop {
        thread::sleep(timings.poll_interval);
        let current = pane.capture_watched_lines()?;
        if current == previous {
            let since = quiet_since.get_or_insert_with(timings.now);
            if rendered && (timings.now)().duration_since(*since) >= timings.quiet_window {
                return Ok(current);
            }
        } else {
            rendered = true;
            quiet_since = None;
            previous = current;
        }
        if (timings.now)() >= deadline {
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
    let deadline = (timings.now)() + timings.submit_timeout;
    loop {
        thread::sleep(timings.poll_interval);
        if pane.capture_watched_lines()? != staged {
            return Ok(());
        }
        if (timings.now)() >= deadline {
            bail!(
                "message was typed into {pane} but the submit key did not take: the pane has not \
                 changed in {}s. The text is most likely still sitting unsent in the composer — \
                 check it with `niles peek`, then send again once the pane is idle.",
                timings.submit_timeout.as_secs()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
    };

    use super::*;

    thread_local! {
        static CLOCK: Cell<Instant> = Cell::new(Instant::now());
    }

    const TEST_TIMINGS: Timings = Timings {
        now: || CLOCK.get(),
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
        pasted_text: RefCell<Option<String>>,
        pasted_frames: RefCell<VecDeque<&'static str>>,
        poll_step: Duration,
        cycle_frames: bool,
        submitted_frame: String,
        started_at: Instant,
    }

    impl ScriptedPane {
        fn new(frames: &[&'static str], cycle_frames: bool, submitted_frame: &str) -> Self {
            assert!(!frames.is_empty());
            Self {
                phase: Cell::new(Phase::Initial),
                actions: RefCell::new(Vec::new()),
                pasted_text: RefCell::new(None),
                pasted_frames: RefCell::new(frames.iter().copied().collect()),
                poll_step: TEST_TIMINGS.poll_interval,
                cycle_frames,
                submitted_frame: submitted_frame.to_owned(),
                started_at: CLOCK.get(),
            }
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
            if self.phase.get() != Phase::Initial {
                CLOCK.set(CLOCK.get() + self.poll_step);
            }
            match self.phase.get() {
                Phase::Initial => Ok("before".to_owned()),
                Phase::Pasted => {
                    let mut frames = self.pasted_frames.borrow_mut();
                    let frame = *frames.front().expect("constructor requires frames");
                    if self.cycle_frames {
                        frames.rotate_left(1);
                    } else if frames.len() > 1 {
                        frames.pop_front();
                    }
                    Ok(frame.to_owned())
                }
                Phase::Submitted => Ok(self.submitted_frame.clone()),
            }
        }

        fn paste_literal(&self, line: &str) -> Result<()> {
            *self.pasted_text.borrow_mut() = Some(line.to_owned());
            self.actions.borrow_mut().push("paste");
            self.phase.set(Phase::Pasted);
            Ok(())
        }

        fn send_submit_key(&self) -> Result<()> {
            self.actions.borrow_mut().push("submit");
            let minimum = if self.cycle_frames || self.poll_step == TEST_TIMINGS.settle_timeout {
                TEST_TIMINGS.settle_timeout
            } else {
                TEST_TIMINGS.poll_interval + TEST_TIMINGS.quiet_window
            };
            assert!(CLOCK.get() - self.started_at >= minimum);
            self.phase.set(Phase::Submitted);
            Ok(())
        }
    }

    #[test]
    fn paste_rendering_and_submission() {
        const SUBMIT: &[&str] = &["leave-mode", "paste", "submit"];
        const PASTE: &[&str] = &["leave-mode", "paste"];
        for (frames, cycle, submitted, error, actions) in [
            (&["staged"][..], false, "changed", None, SUBMIT),
            (
                &["before"][..],
                false,
                "changed",
                Some("never appeared"),
                PASTE,
            ),
            (
                &["staged"][..],
                false,
                "staged",
                Some("`niles peek`"),
                SUBMIT,
            ),
            (&["staged", "moving"][..], true, "changed", None, SUBMIT),
        ] {
            let mut pane = ScriptedPane::new(frames, cycle, submitted);
            // The successful static pane models a poll reaching the settle deadline.
            if !cycle && error.is_none() {
                pane.poll_step = TEST_TIMINGS.settle_timeout;
            }
            let brief = "brief\nwith multiple lines";
            let result = send_line_to_pane(&pane, brief, &TEST_TIMINGS);
            match error {
                Some(needle) => crate::test_support::assert_err_contains(result, needle),
                None => result.unwrap(),
            }
            assert_eq!(pane.pasted_text.borrow().as_deref(), Some(brief));
            assert_eq!(*pane.actions.borrow(), actions);
        }
    }
}

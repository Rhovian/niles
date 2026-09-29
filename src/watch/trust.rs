//! Bounded startup detection for workspace-trust prompts.
//!
//! This only observes a worker's visible pane and appends an ordinary blocked report. The
//! operator owns the decision; the watcher never sends input to the worker.

use std::{fs, io::Write};

use anyhow::Result;
use camino::Utf8Path;
use chrono::{DateTime, TimeDelta, Utc};

use crate::{
    tmux::{TmuxTarget, WindowTarget},
    wake::{self, WakeKind},
    worker::WorkerSnapshot,
};

use super::{Sink, checkin::Checkin};

const STARTUP_WINDOW: TimeDelta = TimeDelta::seconds(30);

pub(super) fn inspect_starting_workers(
    snapshot: &[WorkerSnapshot],
    now: DateTime<Utc>,
    sink: &mut dyn Sink,
) {
    for worker in snapshot {
        if let Err(err) = inspect_worker(worker, now, sink) {
            sink.note(&format!("failed to inspect {} startup: {err:#}", worker.id));
        }
    }
}

fn inspect_worker(worker: &WorkerSnapshot, now: DateTime<Utc>, sink: &mut dyn Sink) -> Result<()> {
    let Some((recorded, project)) = worker.startup_target(now, STARTUP_WINDOW) else {
        return Ok(());
    };
    let window = WindowTarget::parse(recorded)?;
    // Hold the existing inode across capture: a closed worker is never recreated, and a
    // replacement under the same id is not the destination of this inspection.
    let status_path = wake::status_log_path(&worker.worker_dir);
    let mut status = fs::OpenOptions::new().append(true).open(status_path)?;
    if status.metadata()?.len() != 0 {
        return Ok(());
    }
    let screen = sink.capture_visible(&TmuxTarget::window(&window))?;
    if !is_workspace_trust_prompt(&screen, project) {
        return Ok(());
    }

    // Re-check after capture so a worker report that arrived while tmux was read wins.
    if status.metadata()?.len() != 0 {
        return Ok(());
    }

    let line = wake::line(
        WakeKind::Blocked,
        &format!("workspace trust confirmation needs operator action in worker pane {window}"),
    );
    writeln!(status, "{line}")?;

    // This line came from the watcher, not the worker. Keep any existing check-in armed at the
    // same deadline and cadence, but move its baseline past the synthetic report so only a later
    // worker report answers it. A worker with no check-in remains unarmed.
    if let Some(mut checkin) = Checkin::read(&worker.worker_dir)? {
        checkin.armed_len = line.len() as u64 + 1;
        checkin.write(&worker.worker_dir)?;
    }
    Ok(())
}

fn is_workspace_trust_prompt(screen: &str, project: &Utf8Path) -> bool {
    if !has_displayed_project(screen, project) {
        return false;
    }

    let claude = screen.contains("Accessing workspace:")
        && screen.contains("Is this a project you created or one you trust?")
        && screen.contains("No, exit")
        && screen.contains("Yes, I trust this folder")
        && screen.contains("Enter to confirm");
    let codex = screen.contains("Folder access")
        && (screen.contains("Trust this folder?")
            || screen.contains("Do you trust the contents of this directory?"))
        && screen.contains("Trust and continue")
        && screen.contains("Quit");
    claude || codex
}

fn has_displayed_project(screen: &str, project: &Utf8Path) -> bool {
    screen.lines().any(|line| {
        let line = line.trim();
        line == project.as_str()
            || line
                .strip_prefix("Accessing workspace:")
                .is_some_and(|value| value.trim() == project.as_str())
    })
}

#[cfg(test)]
pub(super) fn claude_prompt(project: &Utf8Path) -> String {
    format!(
        "Accessing workspace: {project}\n\
         Quick safety check: Is this a project you created or one you trust?\n\
         ❯ No, exit\n  Yes, I trust this folder\nEnter to confirm · Esc to cancel\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECT: &str = "/private/tmp/niles-trust-probe/claude";
    #[test]
    fn matches_only_workspace_trust_prompts_for_the_exact_project() {
        let project = Utf8Path::new(PROJECT);
        let claude = claude_prompt(project);
        let codex =
            format!("Folder access\n{PROJECT}\nTrust this folder?\nTrust and continue\nQuit");
        let positives = [
            claude.clone(),
            claude
                .replace("❯ No, exit", "  No, exit")
                .replace("  Yes, I trust this folder", "❯ Yes, I trust this folder"),
            codex.clone(),
            codex.replace(
                "Trust this folder?",
                "Do you trust the contents of this directory?",
            ),
        ];
        let negatives = [
            claude.replace(PROJECT, "/private/tmp/somewhere-else"),
            claude_prompt(Utf8Path::new(&format!("{PROJECT}-suffix"))),
            format!("Notes for {PROJECT}: workspace trust in normal output"),
            format!("Command approval\n{PROJECT}\nRun this command?\n› 1. Yes\n  2. Quit"),
            format!("Login required\n{PROJECT}\nTrust this device?"),
        ];
        for screen in positives {
            assert!(is_workspace_trust_prompt(&screen, project), "{screen}");
        }
        for screen in negatives {
            assert!(!is_workspace_trust_prompt(&screen, project), "{screen}");
        }
    }
}

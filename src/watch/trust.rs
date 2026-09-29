//! Bounded startup detection for workspace-trust prompts.
//!
//! This only observes a worker's visible pane and appends an ordinary blocked report. The
//! operator owns the decision; the watcher never sends input to the worker.

use std::{fs, io::Write};

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
        let Some((recorded, project)) = worker.startup_target(now, STARTUP_WINDOW) else {
            continue;
        };
        let window = match WindowTarget::parse(recorded) {
            Ok(window) => window,
            Err(err) => {
                sink.note(&format!(
                    "startup inspection skipped for {}: invalid pane target: {err:#}",
                    worker.id
                ));
                continue;
            }
        };
        // Hold the existing inode across capture: a closed worker is never recreated, and a
        // replacement under the same id is not the destination of this inspection.
        let status_path = wake::status_log_path(&worker.worker_dir);
        let mut status = match fs::OpenOptions::new().append(true).open(&status_path) {
            Ok(status) if status.metadata().is_ok_and(|metadata| metadata.len() == 0) => status,
            Ok(_) => continue,
            Err(err) => {
                sink.note(&format!(
                    "startup inspection skipped for {}: failed to open its status log: {err:#}",
                    worker.id
                ));
                continue;
            }
        };
        let target = TmuxTarget::window(&window);
        let screen = match sink.capture_visible(&target) {
            Ok(screen) => screen,
            Err(err) => {
                sink.note(&format!(
                    "startup inspection failed for {}: {err:#}",
                    worker.id
                ));
                continue;
            }
        };
        if !is_workspace_trust_prompt(&screen, project) {
            continue;
        }

        match append_blocked_if_empty(&mut status, &worker.worker_dir, &window) {
            Ok(_) => {}
            Err(err) => sink.note(&format!(
                "failed to report the workspace-trust prompt for {}: {err:#}",
                worker.id
            )),
        }
    }
}

fn append_blocked_if_empty(
    status: &mut fs::File,
    worker_dir: &Utf8Path,
    window: &WindowTarget,
) -> anyhow::Result<bool> {
    // Re-check after capture so a worker report that arrived while tmux was read wins.
    if status.metadata()?.len() != 0 {
        return Ok(false);
    }

    let line = wake::line(
        WakeKind::Blocked,
        &format!("workspace trust confirmation needs operator action in worker pane {window}"),
    );
    writeln!(status, "{line}")?;

    // This line came from the watcher, not the worker. Keep any existing check-in armed at the
    // same deadline and cadence, but move its baseline past the synthetic report so only a later
    // worker report answers it. A worker with no check-in remains unarmed.
    if let Some(mut checkin) = Checkin::read(worker_dir)? {
        checkin.armed_len = line.len() as u64 + 1;
        checkin.write(worker_dir)?;
    }
    Ok(true)
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
mod tests {
    use super::*;

    const PROJECT: &str = "/private/tmp/niles-trust-probe/claude";
    const CLAUDE: &str = "Accessing workspace: /private/tmp/niles-trust-probe/claude\n\
        Quick safety check: Is this a project you created or one you trust?\n\
        ❯ No, exit\n  Yes, I trust this folder\nEnter to confirm · Esc to cancel\n";
    const CODEX: &str = "Folder access\n/private/tmp/niles-trust-probe/claude\n\
        Trust this folder? Codex can read, edit, and run files here.\n\
        › 1. Trust and continue\n  2. Quit\n";

    #[test]
    fn recognizes_verified_claude_default_no_and_reported_codex_prompts() {
        let project = Utf8Path::new(PROJECT);
        assert!(is_workspace_trust_prompt(CLAUDE, project));
        assert!(is_workspace_trust_prompt(
            &CLAUDE
                .replace("❯ No, exit", "  No, exit")
                .replace("  Yes, I trust this folder", "❯ Yes, I trust this folder"),
            project
        ));
        assert!(is_workspace_trust_prompt(CODEX, project));
        assert!(is_workspace_trust_prompt(
            &CODEX.replace(
                "Trust this folder?",
                "Do you trust the contents of this directory?"
            ),
            project
        ));
    }

    #[test]
    fn rejects_wrong_path_plain_prose_and_nonworkspace_approval() {
        let project = Utf8Path::new(PROJECT);
        assert!(!is_workspace_trust_prompt(
            &CLAUDE.replace(PROJECT, "/private/tmp/somewhere-else"),
            project
        ));
        assert!(!is_workspace_trust_prompt(
            &CLAUDE.replace(PROJECT, &format!("{PROJECT}-suffix")),
            project
        ));
        assert!(!is_workspace_trust_prompt(
            &format!("Notes for {PROJECT}: users may mention workspace trust in normal output"),
            project
        ));
        assert!(!is_workspace_trust_prompt(
            &format!("Command approval\n{PROJECT}\nRun this command?\n› 1. Yes\n  2. Quit\n"),
            project
        ));
        assert!(!is_workspace_trust_prompt(
            &format!("Login required\n{PROJECT}\nTrust this device?\n"),
            project
        ));
    }
}

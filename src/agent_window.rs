use std::fs;

use anyhow::{Context, Result};
use camino::Utf8Path;

use crate::{
    agents::{self, BriefDelivery},
    tmux::{self, SessionName, WindowTarget},
    wake,
};

pub(crate) fn worker_window_name(id: &str) -> String {
    format!("niles-{id}")
}

/// The files a worker window is launched from and reports into.
pub(crate) struct WorkerPaths<'a> {
    pub(crate) brief: &'a Utf8Path,
    pub(crate) launch: &'a Utf8Path,
    pub(crate) status: &'a Utf8Path,
}

pub(crate) fn launch_worker_window(
    session: &SessionName,
    window_name: &str,
    cwd: &Utf8Path,
    invocation: &agents::AgentInvocation,
    paths: &WorkerPaths<'_>,
) -> Result<WindowTarget> {
    let WorkerPaths {
        brief: brief_path,
        launch: launch_path,
        status: status_path,
    } = *paths;
    fs::write(
        launch_path,
        launch_script(invocation, brief_path, status_path),
    )
    .with_context(|| format!("failed to write {launch_path}"))?;
    let command = format!("sh {}", shell_quote(launch_path.as_str()));
    tmux::ensure_window_available(session, window_name)?;
    let target = WindowTarget::new(session.clone(), window_name.to_owned())?;
    tmux::new_window(session, window_name, cwd, &command)?;
    Ok(target)
}

/// Run, not `exec`ed, so the script outlives the agent and reports its exit.
fn launch_script(
    invocation: &agents::AgentInvocation,
    brief_path: &Utf8Path,
    status_path: &Utf8Path,
) -> String {
    let mut body = format!(
        "#!/bin/sh\nset -eu\nBRIEF={}\nSTATUS={}\n",
        shell_quote(brief_path.as_str()),
        shell_quote(status_path.as_str())
    );
    for (key, value) in &invocation.env {
        body.push_str(&format!("export {key}={}\n", shell_assignment_value(value)));
    }
    body.push_str("code=0\n");
    body.push_str(&shell_quote(&invocation.binary));
    for arg in &invocation.args {
        body.push_str(&format!(" {}", shell_quote(arg)));
    }
    match invocation.brief {
        BriefDelivery::Arg => body.push_str(" \"$(cat \"$BRIEF\")\""),
        BriefDelivery::Stdin => body.push_str(" < \"$BRIEF\""),
        BriefDelivery::Flag { path, .. } => body.push_str(&format!(" {path} \"$BRIEF\"")),
        // A worker's brief is the turn itself, with no second turn to follow it, so the flag
        // carries the brief alone — the lead's `<flag> <brief> <turn>` minus the turn.
        BriefDelivery::SystemPrompt(flag) => {
            body.push_str(&format!(" {flag} \"$(cat \"$BRIEF\")\""));
        }
    }
    // `|| code=$?` rather than a bare call: `set -e` would otherwise abort the script on a failing
    // agent, which is precisely the case the report below exists for.
    body.push_str(" || code=$?\n");
    // Literal halves single-quoted, `$code` double-quoted between them: the message expands the
    // exit status without the rest of it being subject to expansion.
    body.push_str("echo ");
    body.push_str(&shell_quote(&wake::line(
        wake::WakeKind::Closed,
        "agent exited (status ",
    )));
    body.push_str("\"$code\"");
    body.push_str(&shell_quote(")"));
    body.push_str(" >> \"$STATUS\"\n");

    body
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shell_assignment_value(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/'))
    {
        return value.to_owned();
    }

    shell_quote(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script_for(brief: BriefDelivery) -> String {
        let invocation = agents::AgentInvocation {
            binary: "codex".to_owned(),
            args: vec!["--flag".to_owned()],
            brief,
            env: Vec::new(),
            spec: agents::AgentSpec::parse("codex", &agents::ModelRoster::builtin().unwrap())
                .unwrap(),
        };
        launch_script(
            &invocation,
            Utf8Path::new("/w/brief.md"),
            Utf8Path::new("/w/status.log"),
        )
    }

    /// The agent is run, not `exec`ed, so something survives it to report the exit.
    #[test]
    fn the_launch_script_reports_the_agents_exit() {
        let script = script_for(BriefDelivery::Arg);

        assert!(
            !script.contains("exec "),
            "agent must not replace the shell:\n{script}"
        );
        // `|| code=$?` and not a bare call: `set -e` would abort before the report otherwise.
        assert!(script.contains("|| code=$?"), "{script}");
        assert!(
            script.contains(r#"echo 'closed: agent exited (status '"$code"')' >> "$STATUS""#),
            "the status must expand, and the rest of the line must not:\n{script}"
        );
    }

    #[test]
    fn the_exit_report_follows_a_stdin_prompt_too() {
        let script = script_for(BriefDelivery::Stdin);

        assert!(script.contains(r#"< "$BRIEF" || code=$?"#), "{script}");
        assert!(script.contains(">> \"$STATUS\""), "{script}");
    }

    #[test]
    fn a_query_file_prompt_hands_over_the_brief_path_not_its_contents() {
        let script = script_for(agents::profile_for("hermes").unwrap().worker_brief);

        assert!(
            script.contains(r#"--query-file "$BRIEF" || code=$?"#),
            "{script}"
        );
        // The brief must not be expanded into the command line: hermes reads the file itself, so
        // nothing in the brief can be taken for shell syntax.
        assert!(!script.contains("$(cat"), "{script}");
    }

    #[test]
    fn shell_quotes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}

use std::fs;

use anyhow::{Context, Result};
use camino::Utf8Path;

use crate::{
    agents,
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
    body.push_str(&shell_quote(invocation.binary));
    for arg in &invocation.args {
        body.push_str(&format!(" {}", shell_quote(arg)));
    }
    body.push_str(" \"$(cat \"$BRIEF\")\"");
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

    fn script_for() -> String {
        let invocation = agents::AgentInvocation {
            binary: "codex",
            args: vec!["--flag".to_owned()],
            system_prompt: agents::profile_for("codex").unwrap().system_prompt,
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
        let script = script_for();

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
    fn opening_briefs_reach_every_family() {
        use std::{os::unix::fs::PermissionsExt, process::Command};

        let models = agents::ModelRoster::builtin().unwrap();
        for profile in agents::profiles() {
            let temp = tempfile::tempdir().unwrap();
            let dir = Utf8Path::from_path(temp.path()).unwrap();
            let binary = dir.join(profile.binary);
            fs::write(
                &binary,
                "#!/bin/sh\nfor arg do :; done\nprintf '%s' \"$arg\"\n",
            )
            .unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            let brief = dir.join("brief.md");
            let text = "line one\n'$(exit 99)' \"quoted\"\nline two";
            fs::write(&brief, text).unwrap();
            let invocation =
                agents::invocation(profile.id, agents::InvocationDefaults::Worker, &models)
                    .unwrap();
            let launch = dir.join("launch.sh");
            fs::write(
                &launch,
                launch_script(&invocation, &brief, &dir.join("status.log")),
            )
            .unwrap();
            let output = Command::new("/bin/sh")
                .arg(&launch)
                .env("PATH", format!("{dir}:/usr/bin:/bin"))
                .output()
                .unwrap();
            assert!(output.status.success(), "{}: {output:?}", profile.id);
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                text,
                "{}",
                profile.id
            );
        }
    }

    #[test]
    fn shell_quotes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}

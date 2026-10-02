use super::{SessionName, WindowPresence, output, run, target};
use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use std::{env, os::unix::process::CommandExt, process::Command};

pub(crate) fn project_session(session: &SessionName) -> Result<Option<String>> {
    let name = target::exact(session.as_str());
    let probe = output(&["has-session", "-t", &name])?;
    if !probe.status.success() {
        let error = super::normalize_stderr(&probe.stderr);
        if target::is_missing_session_error(&error) {
            return Ok(None);
        }
        bail!("tmux has-session failed: {error}");
    }
    Ok(Some(
        super::display(&format!("{name}:"), "#{@niles-project}")?
            .trim_end()
            .to_owned(),
    ))
}

pub(crate) fn lead_running(session: &SessionName) -> Result<bool> {
    Ok(super::window_presence_in(session, "niles")? == WindowPresence::Live)
}

pub(crate) fn open_session(session: &SessionName, path: &Utf8Path) -> Result<()> {
    let executable = env::current_exe().context("failed to find niles executable")?;
    let executable = executable
        .to_str()
        .context("niles executable path is not UTF-8")?;
    let name = session.as_str();
    let window = format!("{}:=niles", target::exact(name));
    match project_session(session)? {
        Some(tag) if tag != path.as_str() => {
            bail!("tmux session {name} exists but isn't niles' session for {path}")
        }
        Some(_) => match super::window_presence_in(session, "niles")? {
            WindowPresence::Live => return Ok(()),
            WindowPresence::PaneExited => {
                return run(&[
                    "respawn-window",
                    "-k",
                    "-t",
                    &window,
                    "-c",
                    path.as_str(),
                    executable,
                    "lead",
                ]);
            }
            WindowPresence::Absent => run(&[
                "new-window",
                "-d",
                "-t",
                &format!("{}:", target::exact(name)),
                "-n",
                "niles",
                "-c",
                path.as_str(),
                executable,
                "lead",
                ";",
                "set-option",
                "-w",
                "-t",
                &window,
                "remain-on-exit",
                "failed",
            ])?,
        },
        None => run(&[
            "new-session",
            "-d",
            "-s",
            name,
            "-c",
            path.as_str(),
            "-n",
            "niles",
            executable,
            "lead",
            ";",
            "set-option",
            "-t",
            &format!("{}:", target::exact(name)),
            "@niles-project",
            path.as_str(),
            ";",
            "set-option",
            "-w",
            "-t",
            &window,
            "remain-on-exit",
            "failed",
        ])?,
    }
    Ok(())
}
pub(crate) fn switch_or_attach(session: &SessionName) -> Result<()> {
    let name = target::exact(session.as_str());
    if env::var_os("TMUX").is_some() {
        run(&["switch-client", "-t", &name])
    } else {
        let error = Command::new("tmux").args(["attach", "-t", &name]).exec();
        Err(error).context("failed to attach to tmux session")
    }
}

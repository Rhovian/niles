use super::{SessionName, TmuxTarget, WindowPresence, output, run, target};
use crate::agent_window::shell_quote;
use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use std::{env, os::unix::process::CommandExt, process::Command};

pub(super) fn has_session(session: &SessionName) -> Result<bool> {
    let probe = output(&["has-session", "-t", &target::exact(session.as_str())])?;
    if !probe.status.success() {
        let error = super::normalize_stderr(&probe.stderr);
        if target::is_missing_session_error(&error) {
            return Ok(false);
        }
        bail!("tmux has-session failed: {error}");
    }
    Ok(true)
}

pub(crate) fn kill_session(session: &SessionName) -> Result<()> {
    run(&["kill-session", "-t", &target::exact(session.as_str())])
}

pub(crate) fn project_session(session: &SessionName) -> Result<Option<String>> {
    if !has_session(session)? {
        return Ok(None);
    }
    let name = target::exact(session.as_str());
    Ok(Some(
        super::display(&format!("{name}:"), "#{@niles-project}")?
            .trim_end()
            .to_owned(),
    ))
}

pub(crate) fn lead_running(session: &SessionName) -> Result<bool> {
    Ok(super::window_presence_in(session, "niles")? == WindowPresence::Live)
}

pub(crate) struct Window {
    pub index: u32,
    pub name: String,
}

pub(crate) fn windows(session: &SessionName) -> Result<Vec<Window>> {
    let target = format!("{}:", target::exact(session.as_str()));
    let output = output(&[
        "list-windows",
        "-t",
        &target,
        "-F",
        "#{window_index}\t#{window_name}",
    ])?;
    if !output.status.success() {
        bail!(
            "tmux list-windows failed: {}",
            super::normalize_stderr(&output.stderr)
        );
    }
    let body = String::from_utf8(output.stdout).context("tmux windows are not UTF-8")?;
    let windows = body
        .lines()
        .map(|line| {
            let [index, name] = line.split('\t').collect::<Vec<_>>()[..] else {
                bail!("invalid tmux window line {line:?}");
            };
            Ok(Window {
                index: index
                    .parse()
                    .with_context(|| format!("invalid window index {index:?}"))?,
                name: name.to_owned(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(windows)
}

pub(crate) fn configure_status(session: &SessionName) -> Result<()> {
    let quoted = shell_quote(&executable()?).replace('#', "##");
    let target = format!("{}:", target::exact(session.as_str()));
    let projects = format!("#({quoted} status projects #{{session_name}})");
    let sessions = format!("#({quoted} status sessions #{{session_name}} #{{window_index}})");
    let options = [
        ("status", "2"),
        ("status-position", "top"),
        ("status-interval", "5"),
        ("status-format[0]", projects.as_str()),
        ("status-format[1]", sessions.as_str()),
    ];
    let args = options
        .into_iter()
        .flat_map(|(option, value)| [";", "set-option", "-t", target.as_str(), option, value])
        .skip(1)
        .collect::<Vec<_>>();
    run(&args)
}

pub(crate) fn open_session(session: &SessionName, path: &Utf8Path) -> Result<()> {
    let executable = executable()?;
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
                    &executable,
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
                &executable,
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
            &executable,
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

pub(super) fn executable() -> Result<String> {
    Ok(env::current_exe()
        .context("failed to find niles executable")?
        .to_str()
        .context("niles executable path is not UTF-8")?
        .to_owned())
}

/// Takes the client this process runs in, or the terminal it runs on, to `target`.
pub(crate) fn switch_or_attach(target: &TmuxTarget) -> Result<()> {
    if env::var_os("TMUX").is_some() {
        run(&["switch-client", "-t", target.as_str()])
    } else {
        let error = Command::new("tmux")
            .args(["attach", "-t", target.as_str()])
            .exec();
        Err(error).context("failed to attach to tmux session")
    }
}

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use std::env;

use super::{SessionName, TmuxTarget, WindowTarget, query, run, session};
use crate::agent_window::shell_quote;

/// The session that holds the explorer. `+` is outside a project name's charset, so no project
/// session can take this name.
const HOME_SESSION: &str = "niles+home";
const SHELL_LINES: &str = "8";
const VIEW_WIDTH: &str = "75%";
/// Marks the pane running the nested client, so it is found again from tmux alone.
const VIEW_OPTION: &str = "@niles-view";

/// Creates the home session unless it exists; an existing one is left as the operator arranged it.
pub(crate) fn open_home(cwd: &Utf8Path) -> Result<SessionName> {
    let home = SessionName::new(HOME_SESSION)?;
    if session::has_session(&home)? {
        return Ok(home);
    }
    let target = TmuxTarget::session(&home);
    let [width, height] = attaching_size()?;
    run(&[
        "new-session",
        "-d",
        "-x",
        &width,
        "-y",
        &height,
        "-s",
        HOME_SESSION,
        "-c",
        cwd.as_str(),
        &session::executable()?,
        "explorer",
        ";",
        "set-option",
        "-t",
        target.as_str(),
        "status",
        "off",
        ";",
        "set-option",
        "-p",
        "-t",
        target.as_str(),
        "remain-on-exit",
        "failed",
        ";",
        "split-window",
        "-d",
        "-f",
        "-v",
        "-l",
        SHELL_LINES,
        "-t",
        target.as_str(),
        "-c",
        cwd.as_str(),
    ])?;
    Ok(home)
}

/// The size of the client about to show the home session. A detached session otherwise starts at
/// tmux's default size and scales its panes on attach, stretching the shell past its lines.
fn attaching_size() -> Result<[String; 2]> {
    if env::var_os("TMUX").is_none() {
        let (width, height) =
            ratatui::crossterm::terminal::size().context("failed to read the terminal size")?;
        return Ok([width.to_string(), height.to_string()]);
    }
    let size = query(&["display-message", "-p", "#{client_width} #{client_height}"])?;
    match size.split_whitespace().collect::<Vec<_>>()[..] {
        [width, height] => Ok([width.to_owned(), height.to_owned()]),
        _ => bail!("unexpected tmux client size {size:?}"),
    }
}

/// Points the view pane's nested client at `window`, splitting the view off `explorer` when there
/// is none. Only that client moves: the operator's own client and the agent windows stay as they
/// are.
pub(crate) fn show_in_view(explorer: &TmuxTarget, window: &WindowTarget) -> Result<()> {
    let window = window.target_arg();
    let view = match view_pane(explorer)? {
        Some(view) => {
            run(&["switch-client", "-c", &view.tty, "-t", &window])?;
            view.id
        }
        None => {
            let socket = super::display(explorer.as_str(), "#{socket_path}")?;
            let attach = format!(
                "env -u TMUX tmux -S {} attach-session -t {}",
                shell_quote(socket.trim_end()),
                shell_quote(&window)
            );
            let id = query(&[
                "split-window",
                "-h",
                "-l",
                VIEW_WIDTH,
                "-t",
                explorer.as_str(),
                "-P",
                "-F",
                "#{pane_id}",
                &attach,
            ])?;
            let id = id.trim_end();
            run(&[
                "set-option",
                "-p",
                "-t",
                id,
                VIEW_OPTION,
                "1",
                ";",
                "set-option",
                "-p",
                "-t",
                id,
                "remain-on-exit",
                "off",
            ])?;
            id.to_owned()
        }
    };
    run(&["select-pane", "-t", &view])
}

struct ViewPane {
    id: String,
    tty: String,
}

fn view_pane(explorer: &TmuxTarget) -> Result<Option<ViewPane>> {
    let panes = query(&[
        "list-panes",
        "-t",
        explorer.as_str(),
        "-F",
        &format!("#{{{VIEW_OPTION}}}\t#{{pane_id}}\t#{{pane_tty}}"),
    ])?;
    panes
        .lines()
        .filter(|line| line.starts_with("1\t"))
        .map(|line| match line.split('\t').collect::<Vec<_>>()[..] {
            [_, id, tty] => Ok(ViewPane {
                id: id.to_owned(),
                tty: tty.to_owned(),
            }),
            _ => bail!("invalid tmux pane line {line:?}"),
        })
        .next()
        .transpose()
}

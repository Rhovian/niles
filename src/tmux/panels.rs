use anyhow::Result;

use super::{SessionName, TmuxTarget, WindowTarget, run, session};

const PANEL_SESSION: &str = "niles+panels";

/// Respawns the panel window `name` with `niles panel <name> <flags>`.
pub(crate) fn open_panel(name: &str, flags: &[&str]) -> Result<WindowTarget> {
    let session = SessionName::new(PANEL_SESSION)?;
    let target = TmuxTarget::session(&session);
    if !session::has_session(&session)? {
        run(&["new-session", "-d", "-s", PANEL_SESSION, "-n", "config"])?;
        run(&["new-window", "-d", "-t", target.as_str(), "-n", "telemetry"])?;
        run(&["new-window", "-d", "-t", target.as_str(), "-n", "help"])?;
        run(&["set-option", "-t", target.as_str(), "status", "off"])?;
        for name in ["config", "telemetry", "help"] {
            let window = WindowTarget::new(session.clone(), name)?;
            for (option, value) in [("remain-on-exit", "on"), ("remain-on-exit-format", "")] {
                super::set_window_option(&window, option, value)?;
            }
        }
    }
    let window = WindowTarget::new(session, name)?;
    let executable = session::executable()?;
    let target = window.target_arg();
    let mut command = vec![
        "respawn-window",
        "-k",
        "-t",
        &target,
        &executable,
        "panel",
        name,
    ];
    command.extend(flags);
    run(&command)?;
    Ok(window)
}

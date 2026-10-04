use anyhow::Result;

use super::{SessionName, TmuxTarget, WindowTarget, run, session};

const PANEL_SESSION: &str = "niles+panels";

pub(crate) fn open_panel(name: &str) -> Result<WindowTarget> {
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
    run(&[
        "respawn-window",
        "-k",
        "-t",
        &window.target_arg(),
        &session::executable()?,
        "panel",
        name,
    ])?;
    Ok(window)
}

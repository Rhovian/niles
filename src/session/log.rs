use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::{util, worker};

pub(crate) fn run(id: &str, decision: &str) -> Result<()> {
    if id != "-" {
        worker::validate_task_label(id)?;
    }
    if decision.trim().is_empty() || decision.chars().any(char::is_control) {
        bail!("decision must be non-empty and contain no control characters");
    }
    let workspace = util::current_dir_utf8()?;
    let lead =
        super::live_lead(&workspace)?.context("niles log requires the current lead session")?;
    let path = super::sessions_dir(&workspace)?
        .join(lead.id)
        .join("decisions.log");
    util::append_line(
        &path,
        &format!("{} {id} {decision}", Utc::now().format("%FT%TZ")),
    )
}

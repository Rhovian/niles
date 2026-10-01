use anyhow::{Result, bail};

use crate::store::ARCHIVE_DIR;

pub(super) fn validate_id(id: &str) -> Result<()> {
    validate_name("worker id", id)
}

pub(crate) fn validate_task_label(label: &str) -> Result<()> {
    validate_name("task label", label)
}

fn validate_name(kind: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("{kind} cannot be empty");
    }
    if value == ARCHIVE_DIR {
        bail!("{kind} '{ARCHIVE_DIR}' is reserved for closed worker archives");
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("{kind} may only contain ASCII letters, numbers, '-' and '_'");
    }
    Ok(())
}

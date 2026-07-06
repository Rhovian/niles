use anyhow::{Result, bail};

pub(super) fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        bail!("worker id cannot be empty");
    }
    if id == "archive" {
        bail!("worker id 'archive' is reserved for closed worker archives");
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("worker id may only contain ASCII letters, numbers, '-' and '_'");
    }
    Ok(())
}

pub(crate) fn validate_task_label(label: &str) -> Result<()> {
    if label.is_empty() {
        bail!("task label cannot be empty");
    }
    if label == "archive" {
        bail!("task label 'archive' is reserved for closed worker archives");
    }
    if !label
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("task label may only contain ASCII letters, numbers, '-' and '_'");
    }
    Ok(())
}

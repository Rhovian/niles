use anyhow::Result;
use camino::Utf8Path;

use crate::{util::read_optional_to_string, wake};

pub(crate) fn read_last_actionable_line(status: &Utf8Path) -> Result<Option<String>> {
    let Some(body) = read_optional_to_string(status, |path| format!("failed to read {path}"))?
    else {
        return Ok(None);
    };

    Ok(body
        .lines()
        .rev()
        .find(|line| wake::is_actionable_wake(line))
        .map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use camino::Utf8PathBuf;

    use super::*;

    #[test]
    fn reads_last_actionable_status_line_without_consuming() {
        let root = temp_dir("last-actionable");
        let status = root.join("status.log");
        fs::write(
            &status,
            "working: started\nnote: ignore me\ndone: first\nblocked: latest\n",
        )
        .unwrap();

        let line = read_last_actionable_line(&status).unwrap();

        assert_eq!(line.as_deref(), Some("blocked: latest"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn status_read_does_not_create_or_modify_ack_or_waiter_files() {
        let root = temp_dir("non-consuming");
        let status = root.join("status.log");
        let ack = status.with_extension("ack");
        let waiter = status.with_extension("waiter");
        let ack_log = status.with_extension("ack.log");
        fs::write(&status, "done: ready\n").unwrap();
        fs::write(&ack, "12\n").unwrap();
        let ack_before = fs::read_to_string(&ack).unwrap();

        let line = read_last_actionable_line(&status).unwrap();

        assert_eq!(line.as_deref(), Some("done: ready"));
        assert_eq!(fs::read_to_string(&ack).unwrap(), ack_before);
        assert!(!waiter.exists());
        assert!(!ack_log.exists());
        fs::remove_dir_all(root).unwrap();
    }

    fn temp_dir(label: &str) -> Utf8PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
            "niles-dashboard-status-{label}-{}-{nanos}",
            std::process::id()
        )))
        .unwrap();
        fs::create_dir_all(&path).unwrap();
        path
    }
}

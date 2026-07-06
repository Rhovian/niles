use anyhow::Result;
use camino::Utf8Path;

use crate::{util::read_optional_to_string, wake};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WakeState {
    Pending,
    Consumed,
    NoActionable,
    AckInvalid,
    NotApplicable,
}

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

pub(crate) fn read_wake_state(status: &Utf8Path) -> Result<WakeState> {
    let Some(body) = read_optional_to_string(status, |path| format!("failed to read {path}"))?
    else {
        return Ok(WakeState::NotApplicable);
    };

    let lines = body.lines().collect::<Vec<_>>();
    let Some((index, _)) = lines
        .iter()
        .enumerate()
        .rev()
        .find(|(_, line)| wake::is_actionable_wake(line))
    else {
        return Ok(WakeState::NoActionable);
    };

    let line_number = index + 1;
    let cursor = match read_ack_cursor(status, lines.len())? {
        Some(cursor) => cursor,
        None => return Ok(WakeState::AckInvalid),
    };
    Ok(wake_state_for_cursor(line_number, cursor))
}

fn read_ack_cursor(status: &Utf8Path, line_count: usize) -> Result<Option<usize>> {
    let path = status.with_extension("ack");
    let Some(body) = read_optional_to_string(&path, |path| format!("failed to read {path}"))?
    else {
        return Ok(Some(0));
    };

    match body.trim().parse::<usize>() {
        Ok(cursor) => Ok(Some(cursor.min(line_count))),
        Err(_) => Ok(None),
    }
}

fn wake_state_for_cursor(line_number: usize, cursor: usize) -> WakeState {
    if line_number <= cursor {
        WakeState::Consumed
    } else {
        WakeState::Pending
    }
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
    fn wake_state_missing_ack_classifies_actionable_or_no_actionable() {
        let root = temp_dir("wake-missing-ack");
        let actionable = root.join("actionable.log");
        let no_actionable = root.join("no-actionable.log");
        fs::write(&actionable, "working: started\ndone: ready\n").unwrap();
        fs::write(&no_actionable, "working: started\nnote: ignore\n").unwrap();

        assert_eq!(read_wake_state(&actionable).unwrap(), WakeState::Pending);
        assert_eq!(
            read_wake_state(&no_actionable).unwrap(),
            WakeState::NoActionable
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn wake_state_compares_ack_cursor_to_last_actionable_line() {
        let root = temp_dir("wake-cursor");
        let status = root.join("status.log");
        let ack = status.with_extension("ack");
        fs::write(&status, "done: first\nworking: middle\nblocked: latest\n").unwrap();
        fs::write(&ack, "2\n").unwrap();

        assert_eq!(read_wake_state(&status).unwrap(), WakeState::Pending);
        fs::write(&ack, "3\n").unwrap();
        assert_eq!(read_wake_state(&status).unwrap(), WakeState::Consumed);
        fs::write(&ack, "99\n").unwrap();
        assert_eq!(read_wake_state(&status).unwrap(), WakeState::Consumed);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn wake_state_unparseable_ack_is_display_state_not_error() {
        let root = temp_dir("wake-invalid-ack");
        let status = root.join("status.log");
        fs::write(&status, "done: ready\n").unwrap();
        fs::write(status.with_extension("ack"), "not-a-number\n").unwrap();

        assert_eq!(read_wake_state(&status).unwrap(), WakeState::AckInvalid);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn wake_state_missing_status_log_is_not_applicable() {
        let root = temp_dir("wake-missing-status");
        let status = root.join("status.log");

        assert_eq!(read_wake_state(&status).unwrap(), WakeState::NotApplicable);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn wake_state_read_does_not_create_or_modify_ack_or_waiter_files() {
        let root = temp_dir("wake-non-consuming");
        let status = root.join("status.log");
        let ack = status.with_extension("ack");
        let waiter = status.with_extension("waiter");
        let ack_log = status.with_extension("ack.log");
        fs::write(&status, "done: first\nblocked: latest\n").unwrap();
        fs::write(&ack, "1\n").unwrap();
        let ack_before = fs::read(&ack).unwrap();

        let state = read_wake_state(&status).unwrap();

        assert_eq!(state, WakeState::Pending);
        assert_eq!(fs::read(&ack).unwrap(), ack_before);
        assert!(!waiter.exists());
        assert!(!ack_log.exists());
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

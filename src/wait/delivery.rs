use super::Identity;
use super::cursor::{cursor_path, read_cursor, write_cursor};
use crate::{store, wake, worker};
use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, TryLockError};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use uuid::Uuid;

#[derive(Deserialize, Serialize)]
pub(super) struct Batch {
    token: String,
    receipts: Vec<Receipt>,
}

impl Batch {
    pub(super) fn publish(path: &Utf8Path, receipts: Vec<Receipt>) -> Result<String> {
        let token = Uuid::new_v4().to_string();
        store::write_json(
            path,
            &Self {
                token: token.clone(),
                receipts,
            },
        )?;
        Ok(token)
    }

    pub(super) fn acknowledge(path: &Utf8Path, workspace: &Utf8Path, token: &str) -> Result<()> {
        let Some(batch) = store::read_optional_json::<Self>(path)? else {
            return Ok(());
        };
        if batch.token == token {
            for receipt in batch.receipts {
                receipt.acknowledge(workspace)?;
            }
            fs::remove_file(path).with_context(|| format!("failed to remove {path}"))?;
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
pub(super) struct Receipt {
    pub(super) id: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) identity: Identity,
    pub(super) end: u64,
    pub(super) reset_from: Option<u64>,
}

impl Receipt {
    pub(super) fn acknowledge(self, workspace: &Utf8Path) -> Result<()> {
        worker::validate_id(&self.id)?;
        let Some(dir) = store::worker_location(workspace, &self.id)? else {
            return Ok(());
        };
        let path = cursor_path(&dir);
        let mut cursor = match File::options()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).with_context(|| format!("failed to open {path}")),
        };
        // Match polling: a sender holding the cursor must not block workspace takeover.
        match cursor.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(()),
            Err(TryLockError::Error(error)) => {
                return Err(error).with_context(|| format!("failed to lock {path}"));
            }
        }
        if worker::meta::read_meta_if_exists(&dir)?
            .is_none_or(|meta| meta.created_at != self.created_at)
        {
            return Ok(());
        }
        let log = wake::status_log_path(&dir);
        let current = match fs::symlink_metadata(&log) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).with_context(|| format!("failed to inspect {log}")),
        };
        if Identity::from(&current) == self.identity && current.len() >= self.end {
            let previous = read_cursor(&mut cursor, &path)?;
            // An observed truncation can invalidate the previous offset. A later valid offset
            // belongs to another acknowledgement or send and must stay in place.
            if previous < self.end
                || (self.reset_from == Some(previous) && previous > current.len())
            {
                write_cursor(&mut cursor, &path, self.end)?;
            }
        }
        Ok(())
    }
}

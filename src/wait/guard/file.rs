use std::{
    fs,
    io::{ErrorKind, Write},
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::util::read_optional_to_string;

use super::{GuardCheck, WaiterRegistration};

const DEFAULT_WAITER_FILE_NAME: &str = "status.waiter";

pub(super) enum RegistrationRead {
    Present(WaiterRegistration),
    Missing,
    Indeterminate { detail: String },
}

pub(super) enum RegistrationTokenCheck {
    Matches(WaiterRegistration),
    Missing,
    Changed,
    Indeterminate { detail: String },
}

pub(super) fn read_registration(path: &Utf8Path) -> RegistrationRead {
    let body = match read_optional_to_string(path, |path| format!("failed to read {path}")) {
        Ok(Some(body)) => body,
        Ok(None) => return RegistrationRead::Missing,
        Err(err) => {
            return RegistrationRead::Indeterminate {
                detail: format!("failed to read {path}: {err:#}"),
            };
        }
    };

    match serde_json::from_str(&body) {
        Ok(registration) => RegistrationRead::Present(registration),
        Err(err) => RegistrationRead::Indeterminate {
            detail: format!("failed to parse {path}: {err:#}"),
        },
    }
}

pub(super) fn verify_registration_token(path: &Utf8Path, token: &str) -> RegistrationTokenCheck {
    match read_registration(path) {
        RegistrationRead::Present(registration) if registration.token == token => {
            RegistrationTokenCheck::Matches(registration)
        }
        RegistrationRead::Present(_) => RegistrationTokenCheck::Changed,
        RegistrationRead::Missing => RegistrationTokenCheck::Missing,
        RegistrationRead::Indeterminate { detail } => {
            RegistrationTokenCheck::Indeterminate { detail }
        }
    }
}

pub(super) fn replace_registration_if_token_matches(
    path: &Utf8Path,
    registration: &WaiterRegistration,
    expected_token: &str,
) -> Result<GuardCheck> {
    let Some(parent) = path.parent() else {
        bail!("waiter registration path {path} has no parent");
    };
    let body = registration_body(registration)?;
    let waiter_file_name = match path.file_name() {
        Some(file_name) => file_name,
        None => DEFAULT_WAITER_FILE_NAME,
    };
    let temp_path = parent.join(format!(
        ".{}.{}.tmp",
        waiter_file_name,
        uuid::Uuid::new_v4()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .with_context(|| format!("failed to create temporary waiter registration {temp_path}"))?;
    file.write_all(body.as_bytes())
        .with_context(|| format!("failed to write temporary waiter registration {temp_path}"))?;
    file.sync_all()
        .with_context(|| format!("failed to sync temporary waiter registration {temp_path}"))?;
    drop(file);

    match verify_registration_token(path, expected_token) {
        RegistrationTokenCheck::Matches(_) => {}
        RegistrationTokenCheck::Missing => {
            let _ = fs::remove_file(&temp_path);
            return Ok(GuardCheck::Lost {
                detail: format!("waiter registration was removed/replaced while waiting: {path}"),
            });
        }
        RegistrationTokenCheck::Changed => {
            let _ = fs::remove_file(&temp_path);
            return Ok(GuardCheck::Lost {
                detail: format!("waiter registration was removed/replaced while waiting: {path}"),
            });
        }
        RegistrationTokenCheck::Indeterminate { detail } => {
            let _ = fs::remove_file(&temp_path);
            return Ok(GuardCheck::Indeterminate { detail });
        }
    }

    fs::rename(&temp_path, path)
        .with_context(|| format!("failed to replace waiter registration {path}"))?;
    Ok(GuardCheck::Verified)
}

pub(super) fn registration_body(registration: &WaiterRegistration) -> Result<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(registration)?))
}

pub(super) fn remove_waiter_file(path: &Utf8Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => {
            Err(err).with_context(|| format!("failed to remove waiter registration {path}"))
        }
    }
}

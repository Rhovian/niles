use std::{env, ffi::OsString, path::PathBuf};

use anyhow::{Context, Result};
use camino::Utf8PathBuf;

use crate::util::utf8_path;

pub(crate) const CODEX_HOME_ENV: &str = "CODEX_HOME";
pub(crate) const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
pub(crate) const DEFAULT_CODEX_HOME_SUFFIX: &str = ".codex";
pub(crate) const DEFAULT_CLAUDE_CONFIG_SUFFIX: &str = ".claude";

const HOME_ENV: &str = "HOME";

pub(crate) fn codex_home() -> Result<Utf8PathBuf> {
    resolve_home(
        CODEX_HOME_ENV,
        DEFAULT_CODEX_HOME_SUFFIX,
        "Codex home",
        |name| env::var_os(name),
    )
}

pub(crate) fn claude_home() -> Result<Utf8PathBuf> {
    resolve_home(
        CLAUDE_CONFIG_DIR_ENV,
        DEFAULT_CLAUDE_CONFIG_SUFFIX,
        "Claude config dir",
        |name| env::var_os(name),
    )
}

fn resolve_home(
    override_env: &str,
    default_suffix: &str,
    description: &str,
    mut get_env: impl FnMut(&str) -> Option<OsString>,
) -> Result<Utf8PathBuf> {
    if let Some(path) = get_env(override_env) {
        return utf8_path(PathBuf::from(path), description);
    }

    let home = get_env(HOME_ENV)
        .with_context(|| format!("{description} requires ${override_env} or $HOME"))?;
    utf8_path(PathBuf::from(home).join(default_suffix), description)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_env_wins() {
        let resolved = resolve_home(
            CODEX_HOME_ENV,
            DEFAULT_CODEX_HOME_SUFFIX,
            "Codex home",
            |name| match name {
                CODEX_HOME_ENV => Some(OsString::from("/tmp/codex-custom")),
                HOME_ENV => Some(OsString::from("/tmp/home")),
                _ => None,
            },
        )
        .unwrap();

        assert_eq!(resolved, Utf8PathBuf::from("/tmp/codex-custom"));
    }

    #[test]
    fn home_fallback_uses_named_suffix() {
        let resolved = resolve_home(
            CLAUDE_CONFIG_DIR_ENV,
            DEFAULT_CLAUDE_CONFIG_SUFFIX,
            "Claude config dir",
            |name| match name {
                HOME_ENV => Some(OsString::from("/tmp/home")),
                _ => None,
            },
        )
        .unwrap();

        assert_eq!(resolved, Utf8PathBuf::from("/tmp/home/.claude"));
    }

    #[test]
    fn missing_override_and_home_fails_loudly() {
        let err = resolve_home(
            CODEX_HOME_ENV,
            DEFAULT_CODEX_HOME_SUFFIX,
            "Codex home",
            |_| None,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("Codex home requires $CODEX_HOME or $HOME"));
    }
}

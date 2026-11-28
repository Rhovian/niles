use crate::theme::Theme;
use crate::{store, util};
use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use ratatui_themes::ThemeName;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::net::Ipv4Addr;

pub(crate) const DEFAULT_THEME: ThemeName = ThemeName::TokyoNight;
pub(crate) const DEFAULT_BINDINGS: bool = false;

/// `config.yaml` as written: a key it leaves out is `None`, so the config panel can tell an
/// absent key from one set to its builtin value.
#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct FileConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) theme: Option<ThemeName>,
    #[serde(skip_serializing_if = "tmux_is_empty")]
    pub(crate) tmux: FileTmux,
    #[serde(skip_serializing_if = "remote_is_empty")]
    pub(crate) remote: FileRemote,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct FileRemote {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) web: Option<FileWeb>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) issuers: BTreeMap<String, Vec<FileGrant>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) introspection: Option<FileIntrospection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mcp: Option<FileMcp>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileMcp {
    pub(crate) resource: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) interactive_issuer: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileWeb {
    pub(crate) bind: Ipv4Addr,
    pub(crate) resource: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileIntrospection {
    pub(crate) issuer: String,
    pub(crate) client_id: String,
    pub(crate) client_secret_env: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileGrant {
    pub(crate) subject: String,
    pub(crate) project: String,
    pub(crate) access: Access,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Access {
    Read,
    Operate,
}

fn remote_is_empty(remote: &FileRemote) -> bool {
    remote.web.is_none()
        && remote.issuers.is_empty()
        && remote.introspection.is_none()
        && remote.mcp.is_none()
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct FileTmux {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bindings: Option<bool>,
}

fn tmux_is_empty(tmux: &FileTmux) -> bool {
    tmux.bindings.is_none()
}

impl FileConfig {
    pub(crate) fn save(&self, path: &Utf8Path) -> Result<()> {
        fs::create_dir_all(path.parent().context("config path has no parent")?)?;
        store::write_yaml(path, self)
    }

    /// The file at `path`, or an empty config when there is no file.
    pub(crate) fn load(path: &Utf8Path) -> Result<Self> {
        match store::read_optional_yaml(path)? {
            Some(file) => Ok(file),
            None => Ok(Self::default()),
        }
    }
}

pub(crate) struct TmuxConfig {
    pub(crate) bindings: bool,
}

pub(crate) struct UserConfig {
    pub(crate) theme: Theme,
    pub(crate) tmux: TmuxConfig,
}

impl UserConfig {
    pub(crate) fn load() -> Result<Self> {
        Ok(Self::resolve(FileConfig::load(&path()?)?))
    }

    #[cfg(test)]
    pub(crate) fn parse(text: Option<&str>) -> Result<Self> {
        let file = match text {
            Some(text) => store::parse_yaml(text)?,
            None => FileConfig::default(),
        };
        Ok(Self::resolve(file))
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the parse boundary resolves each absent key to its named default"
    )]
    fn resolve(config: FileConfig) -> Self {
        Self {
            theme: Theme::new(config.theme.unwrap_or(DEFAULT_THEME)),
            tmux: TmuxConfig {
                bindings: config.tmux.bindings.unwrap_or(DEFAULT_BINDINGS),
            },
        }
    }
}

pub(crate) fn path() -> Result<Utf8PathBuf> {
    Ok(util::home_dir()?.join(".niles/config.yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::StyleKey;

    #[test]
    fn parses_named_palette_and_bindings() {
        let config = UserConfig::parse(Some("theme: nord\ntmux: {bindings: true}")).unwrap();
        assert_eq!(
            config.theme.style(StyleKey::Running).fg,
            Some(ThemeName::Nord.palette().success)
        );
        assert!(config.tmux.bindings);
    }

    #[test]
    fn missing_theme_defaults_to_tokyo_night() {
        for text in [None, Some("{}"), Some("tmux: {bindings: true}")] {
            let config = UserConfig::parse(text).unwrap();
            assert_eq!(
                config.theme.style(StyleKey::Bar).bg,
                Some(ThemeName::TokyoNight.palette().selection)
            );
        }
    }

    #[test]
    fn rejects_unknown_names_and_keys() {
        for text in [
            "theme: unknown",
            "unknown: {}",
            "tmux: {unknown: true}",
            "tmux: {bindings: invalid}",
            "theme: {styles: {waiting: bold}}",
        ] {
            assert!(UserConfig::parse(Some(text)).is_err(), "{text}");
        }
    }
}

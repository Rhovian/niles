use std::{env, fs};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use ratatui_themes::ThemeName;

use crate::theme::Theme;

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
        crate::store::write_yaml(path, self)
    }

    /// The file at `path`, or an empty config when there is no file.
    pub(crate) fn load(path: &Utf8Path) -> Result<Self> {
        match crate::store::read_optional_yaml(path)? {
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
            Some(text) => crate::store::parse_yaml(text)?,
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
    Ok(Utf8PathBuf::from(env::var("HOME").context("HOME is missing")?).join(".niles/config.yaml"))
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

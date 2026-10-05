use std::{env, fs, io::ErrorKind};

use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use serde::Deserialize;

use ratatui_themes::ThemeName;

use crate::theme::Theme;

const DEFAULT_THEME: ThemeName = ThemeName::TokyoNight;

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    theme: ThemeName,
    tmux: TmuxConfig,
}

impl Default for FileConfig {
    fn default() -> Self {
        Self {
            theme: DEFAULT_THEME,
            tmux: TmuxConfig::default(),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TmuxConfig {
    pub(crate) bindings: bool,
}

pub(crate) struct UserConfig {
    pub(crate) theme: Theme,
    pub(crate) tmux: TmuxConfig,
}

impl UserConfig {
    pub(crate) fn load() -> Result<Self> {
        let path = Utf8PathBuf::from(env::var("HOME").context("HOME is missing")?)
            .join(".niles/config.yaml");
        let text = match fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error).with_context(|| format!("failed to read {path}")),
        };
        Self::parse(text.as_deref()).with_context(|| format!("invalid config {path}"))
    }

    pub(crate) fn parse(text: Option<&str>) -> Result<Self> {
        let config = match text {
            Some(text) => serde_saphyr::from_str::<FileConfig>(text)?,
            None => FileConfig::default(),
        };
        Ok(Self {
            theme: Theme::new(config.theme),
            tmux: config.tmux,
        })
    }
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

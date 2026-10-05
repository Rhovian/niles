use std::{env, fs, io::ErrorKind};

use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use serde::Deserialize;

use crate::theme::{Overrides, Theme};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    theme: Overrides,
    tmux: TmuxConfig,
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
            theme: Theme::build(config.theme)?,
            tmux: config.tmux,
        })
    }
}

use anyhow::Result;
use ratatui::style::{Modifier, Style};
use ratatui_themes::{ThemeName, ThemePalette};

mod render;
pub(crate) use render::StyleRender;

pub(crate) const RUNNING: &str = "●";
pub(crate) const WAITING: &str = "⚠";
pub(crate) const IDLE: &str = "○";
pub(crate) const SPINNER: [&str; 8] = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];
pub(crate) const BRANCH: &str = "├─";
pub(crate) const LAST: &str = "└─";
pub(crate) const STEM: &str = ratatui::symbols::line::VERTICAL;
pub(crate) const EXPANDED: &str = "▾";
pub(crate) const COLLAPSED: &str = "▸";

#[derive(Clone, Copy)]
pub(crate) enum StyleKey {
    Running,
    Waiting,
    Idle,
    Lost,
    Selection,
    Pill,
    Heading,
    Muted,
    Accent,
    Guide,
    Bar,
}

#[derive(Clone, Copy)]
pub(crate) enum State {
    Running,
    Waiting,
    Idle,
}

pub(crate) struct Theme {
    palette: ThemePalette,
}

impl Theme {
    pub(crate) fn load() -> Result<Self> {
        Ok(crate::config::user::UserConfig::load()?.theme)
    }

    /// Test themes always emit colors, independent of the caller's environment.
    #[cfg(test)]
    pub(crate) fn parse(text: Option<&str>) -> Result<Self> {
        ratatui::crossterm::style::force_color_output(true);
        Ok(crate::config::user::UserConfig::parse(text)?.theme)
    }

    pub(crate) fn new(name: ThemeName) -> Self {
        Self {
            palette: name.palette(),
        }
    }

    pub(crate) fn style(&self, key: StyleKey) -> Style {
        let p = self.palette;
        let style = Style::new();
        match key {
            StyleKey::Running => style.fg(p.success),
            StyleKey::Waiting => style.fg(p.warning),
            StyleKey::Idle | StyleKey::Muted => style.fg(p.muted),
            StyleKey::Lost => style.fg(p.error),
            StyleKey::Guide => style.fg(p.muted).add_modifier(Modifier::DIM),
            StyleKey::Accent => style.fg(p.accent).add_modifier(Modifier::BOLD),
            StyleKey::Heading => style.add_modifier(Modifier::BOLD),
            StyleKey::Selection => style.bg(p.selection).add_modifier(Modifier::BOLD),
            StyleKey::Pill => style.fg(p.bg).bg(p.accent).add_modifier(Modifier::BOLD),
            StyleKey::Bar => style.fg(p.fg).bg(p.selection),
        }
    }

    pub(crate) fn state(&self, state: State) -> (&'static str, Style) {
        let (glyph, key) = match state {
            State::Running => (RUNNING, StyleKey::Running),
            State::Waiting => (WAITING, StyleKey::Waiting),
            State::Idle => (IDLE, StyleKey::Idle),
        };
        (glyph, self.style(key))
    }

    pub(crate) fn spinner(&self, frame: i64) -> &'static str {
        SPINNER[frame.rem_euclid(SPINNER.len() as i64) as usize]
    }
}

#[cfg(test)]
mod tests;

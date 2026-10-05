use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use ratatui::{
    backend::IntoCrossterm,
    crossterm::style::{Attribute, SetAttribute, SetBackgroundColor, SetForegroundColor},
    style::{Color, Modifier, Style as TuiStyle},
    text::Span,
};
use serde::Deserialize;

const DEFAULTS: &str = r##"
styles:
  running: 'fg=#5fd38d'
  waiting: 'fg=#f0a35e'
  idle: 'fg=#8a8f98'
  lost: 'fg=#e06c75'
  selection: 'bg=#1f2a44,bold'
  pill: 'fg=#1b1b1b,bg=#6b9cff,bold'
  heading: 'bold'
  muted: 'fg=#6c7086'
  accent: 'fg=#6b9cff,bold'
  guide: 'fg=#3b4252'
  bar: 'fg=#c0caf5,bg=#1a1b26'
glyphs:
  running: '●'
  waiting: '⚠'
  idle: '○'
  spinner: ['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷']
  branch: '├─'
  last: '└─'
  stem: '│'
  expanded: '▾'
  collapsed: '▸'
"##;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GlyphKey {
    Running,
    Waiting,
    Idle,
    Spinner,
    Branch,
    Last,
    Stem,
    Expanded,
    Collapsed,
}

#[derive(Clone, Copy)]
pub(crate) enum State {
    Running,
    Waiting,
    Idle,
}

impl State {
    fn tokens(self) -> (StyleKey, GlyphKey) {
        match self {
            Self::Running => (StyleKey::Running, GlyphKey::Running),
            Self::Waiting => (StyleKey::Waiting, GlyphKey::Waiting),
            Self::Idle => (StyleKey::Idle, GlyphKey::Idle),
        }
    }
}

pub(crate) struct Style {
    pub(crate) ratatui: TuiStyle,
    text: String,
}

impl Style {
    fn parse(key: &str, text: String) -> Result<Self> {
        let mut style = TuiStyle::new();
        for item in text.split(',') {
            if let Some(color) = item.strip_prefix("fg=") {
                style = style
                    .fg(parse_color(color)
                        .with_context(|| format!("{key}: bad color `{color}`"))?);
            } else if let Some(color) = item.strip_prefix("bg=") {
                style = style
                    .bg(parse_color(color)
                        .with_context(|| format!("{key}: bad color `{color}`"))?);
            } else {
                let modifier = match item {
                    "bold" => Modifier::BOLD,
                    "dim" => Modifier::DIM,
                    "italics" => Modifier::ITALIC,
                    "underscore" => Modifier::UNDERLINED,
                    "reverse" => Modifier::REVERSED,
                    _ => bail!("{key}: unknown attribute `{item}`"),
                };
                style = style.add_modifier(modifier);
            }
        }
        Ok(Self {
            ratatui: style,
            text,
        })
    }

    pub(crate) fn tmux_option(&self) -> &str {
        &self.text
    }

    pub(crate) fn tmux(&self) -> String {
        format!("#[{}]", self.text)
    }

    pub(crate) fn paint(&self, text: &str) -> String {
        let mut painted = String::new();
        if let Some(color) = self.ratatui.fg {
            painted.push_str(&SetForegroundColor(color.into_crossterm()).to_string());
        }
        if let Some(color) = self.ratatui.bg {
            painted.push_str(&SetBackgroundColor(color.into_crossterm()).to_string());
        }
        for (modifier, attribute) in [
            (Modifier::BOLD, Attribute::Bold),
            (Modifier::DIM, Attribute::Dim),
            (Modifier::ITALIC, Attribute::Italic),
            (Modifier::UNDERLINED, Attribute::Underlined),
            (Modifier::REVERSED, Attribute::Reverse),
        ] {
            if self.ratatui.add_modifier.contains(modifier) {
                painted.push_str(&SetAttribute(attribute).to_string());
            }
        }
        painted.push_str(text);
        painted.push_str(&SetAttribute(Attribute::Reset).to_string());
        painted
    }
}

fn parse_color(text: &str) -> Result<Color> {
    let named = match text {
        "default" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::Gray,
        "brightblack" => Color::DarkGray,
        "brightred" => Color::LightRed,
        "brightgreen" => Color::LightGreen,
        "brightyellow" => Color::LightYellow,
        "brightblue" => Color::LightBlue,
        "brightmagenta" => Color::LightMagenta,
        "brightcyan" => Color::LightCyan,
        "brightwhite" => Color::White,
        _ => {
            if let Some(index) = text
                .strip_prefix("colour")
                .or_else(|| text.strip_prefix("color"))
                && !index.is_empty()
                && index.bytes().all(|b| b.is_ascii_digit())
            {
                return Ok(Color::Indexed(index.parse()?));
            }
            if let Some(hex) = text.strip_prefix('#')
                && hex.len() == 6
                && hex.bytes().all(|b| b.is_ascii_hexdigit())
            {
                let rgb = u32::from_str_radix(hex, 16)?;
                return Ok(Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8));
            }
            bail!("unsupported color")
        }
    };
    Ok(named)
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Overrides {
    #[serde(default)]
    styles: BTreeMap<StyleKey, String>,
    #[serde(default)]
    glyphs: BTreeMap<GlyphKey, GlyphValue>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GlyphValue {
    Single(String),
    Frames(Vec<String>),
}

fn frames(key: GlyphKey, value: GlyphValue) -> Result<Vec<String>> {
    let name = format!("theme.glyphs.{}", format!("{key:?}").to_lowercase());
    let frames = match (key, value) {
        (GlyphKey::Spinner, GlyphValue::Frames(frames)) => frames,
        (key, GlyphValue::Single(value)) if key != GlyphKey::Spinner => {
            vec![value]
        }
        _ => bail!(
            "{name}: expected {}",
            if key == GlyphKey::Spinner {
                "a frame list"
            } else {
                "a glyph string"
            }
        ),
    };
    let width = match key {
        GlyphKey::Branch | GlyphKey::Last => 2,
        GlyphKey::Running
        | GlyphKey::Waiting
        | GlyphKey::Idle
        | GlyphKey::Spinner
        | GlyphKey::Stem
        | GlyphKey::Expanded
        | GlyphKey::Collapsed => 1,
    };
    if frames.is_empty()
        || frames
            .iter()
            .any(|glyph| glyph.is_empty() || Span::raw(glyph).width() != width)
    {
        bail!(
            "{name}: glyphs must be non-empty and display width {width}; spinner must have frames"
        );
    }
    Ok(frames)
}

pub(crate) struct Theme {
    styles: BTreeMap<StyleKey, Style>,
    glyphs: BTreeMap<GlyphKey, Vec<String>>,
}

impl Theme {
    pub(crate) fn load() -> Result<Self> {
        Ok(crate::config::user::UserConfig::load()?.theme)
    }

    #[cfg(test)]
    pub(crate) fn parse(text: Option<&str>) -> Result<Self> {
        Ok(crate::config::user::UserConfig::parse(text)?.theme)
    }

    pub(crate) fn build(overrides: Overrides) -> Result<Self> {
        let mut config: Overrides = serde_saphyr::from_str(DEFAULTS)?;
        config.styles.extend(overrides.styles);
        config.glyphs.extend(overrides.glyphs);
        let styles = config
            .styles
            .into_iter()
            .map(|(key, value)| {
                let name = format!("theme.styles.{}", format!("{key:?}").to_lowercase());
                Ok((key, Style::parse(&name, value)?))
            })
            .collect::<Result<_>>()?;
        let glyphs = config
            .glyphs
            .into_iter()
            .map(|(key, value)| Ok((key, frames(key, value)?)))
            .collect::<Result<_>>()?;
        Ok(Self { styles, glyphs })
    }

    pub(crate) fn style(&self, key: StyleKey) -> &Style {
        &self.styles[&key]
    }
    pub(crate) fn glyph(&self, key: GlyphKey) -> &str {
        &self.glyphs[&key][0]
    }
    pub(crate) fn state(&self, state: State) -> (&str, &Style) {
        let (style, glyph) = state.tokens();
        (self.glyph(glyph), self.style(style))
    }
    pub(crate) fn spinner(&self, frame: i64) -> &str {
        let frames = &self.glyphs[&GlyphKey::Spinner];
        &frames[frame.rem_euclid(frames.len() as i64) as usize]
    }
}

#[cfg(test)]
mod tests;

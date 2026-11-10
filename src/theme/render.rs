use ratatui::backend::IntoCrossterm;
use ratatui::crossterm::style::{Attribute, SetAttribute, SetBackgroundColor, SetForegroundColor};
use ratatui::style::{Color, Modifier, Style};

const MODIFIERS: [(Modifier, &str, Attribute); 2] = [
    (Modifier::BOLD, "bold", Attribute::Bold),
    (Modifier::DIM, "dim", Attribute::Dim),
];

pub(crate) trait StyleRender {
    fn tmux_option(&self) -> String;
    fn tmux(&self) -> String;
    fn paint(&self, text: &str) -> String;
}

impl StyleRender for Style {
    fn tmux_option(&self) -> String {
        let mut parts = Vec::new();
        for (slot, color) in [("fg", self.fg), ("bg", self.bg)] {
            let color = match color {
                Some(Color::Reset) | None => continue,
                Some(Color::Rgb(r, g, b)) => format!("#{r:02x}{g:02x}{b:02x}"),
                Some(Color::Indexed(index)) => format!("colour{index}"),
                Some(Color::Black) => "colour0".into(),
                Some(Color::Red) => "colour1".into(),
                Some(Color::Green) => "colour2".into(),
                Some(Color::Yellow) => "colour3".into(),
                Some(Color::Blue) => "colour4".into(),
                Some(Color::Magenta) => "colour5".into(),
                Some(Color::Cyan) => "colour6".into(),
                Some(Color::Gray) => "colour7".into(),
                Some(Color::DarkGray) => "colour8".into(),
                Some(Color::LightRed) => "colour9".into(),
                Some(Color::LightGreen) => "colour10".into(),
                Some(Color::LightYellow) => "colour11".into(),
                Some(Color::LightBlue) => "colour12".into(),
                Some(Color::LightMagenta) => "colour13".into(),
                Some(Color::LightCyan) => "colour14".into(),
                Some(Color::White) => "colour15".into(),
            };
            parts.push(format!("{slot}={color}"));
        }
        for (modifier, name, _) in MODIFIERS {
            if self.add_modifier.contains(modifier) {
                parts.push(name.into());
            }
        }
        parts.join(",")
    }

    fn tmux(&self) -> String {
        format!("#[{}]", self.tmux_option())
    }

    fn paint(&self, text: &str) -> String {
        let mut painted = String::new();
        if let Some(color) = self.fg {
            painted.push_str(&SetForegroundColor(color.into_crossterm()).to_string());
        }
        if let Some(color) = self.bg {
            painted.push_str(&SetBackgroundColor(color.into_crossterm()).to_string());
        }
        for (modifier, _, attribute) in MODIFIERS {
            if self.add_modifier.contains(modifier) {
                painted.push_str(&SetAttribute(attribute).to_string());
            }
        }
        painted.push_str(text);
        painted.push_str(&SetAttribute(Attribute::Reset).to_string());
        painted
    }
}

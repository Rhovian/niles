use ratatui::{
    backend::IntoCrossterm,
    crossterm::style::{Attribute, SetAttribute, SetBackgroundColor, SetForegroundColor},
    style::{Color, Modifier, Style},
};

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
            match color {
                Some(Color::Rgb(r, g, b)) => parts.push(format!("{slot}=#{r:02x}{g:02x}{b:02x}")),
                Some(Color::Reset) | None => {}
                Some(
                    Color::Black
                    | Color::Red
                    | Color::Green
                    | Color::Yellow
                    | Color::Blue
                    | Color::Magenta
                    | Color::Cyan
                    | Color::Gray
                    | Color::DarkGray
                    | Color::LightRed
                    | Color::LightGreen
                    | Color::LightYellow
                    | Color::LightBlue
                    | Color::LightMagenta
                    | Color::LightCyan
                    | Color::White
                    | Color::Indexed(_),
                ) => {
                    unreachable!("ratatui-themes palettes contain only RGB colors")
                }
            }
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

use ratatui::style::Color;

use super::*;

#[test]
fn palettes_are_rgb() {
    for name in ThemeName::all() {
        let p = name.palette();
        for color in [
            p.accent,
            p.secondary,
            p.bg,
            p.fg,
            p.muted,
            p.selection,
            p.error,
            p.warning,
            p.success,
            p.info,
        ] {
            assert!(matches!(color, Color::Rgb(..)), "{name:?}: {color:?}");
        }
    }
}

#[test]
fn tmux_renders_typed_colors_and_modifiers() {
    let style = Style::new()
        .fg(Color::Rgb(170, 187, 204))
        .bg(Color::Rgb(1, 2, 3))
        .add_modifier(Modifier::BOLD | Modifier::DIM);
    assert_eq!(style.tmux_option(), "fg=#aabbcc,bg=#010203,bold,dim");
    assert_eq!(style.tmux(), "#[fg=#aabbcc,bg=#010203,bold,dim]");
    assert_eq!(Style::new().fg(Color::Reset).tmux_option(), "");
}

#[test]
fn ansi_paint_uses_typed_colors_and_modifiers_and_resets() {
    let style = Style::new()
        .fg(Color::Rgb(170, 187, 204))
        .bg(Color::Rgb(1, 2, 3))
        .add_modifier(Modifier::BOLD | Modifier::DIM);
    let colors = if ratatui::crossterm::style::Colored::ansi_color_disabled_memoized() {
        "\x1b[m\x1b[m"
    } else {
        "\x1b[38;2;170;187;204m\x1b[48;2;1;2;3m"
    };
    assert_eq!(
        style.paint("text"),
        format!("{colors}\x1b[1m\x1b[2mtext\x1b[0m")
    );
}

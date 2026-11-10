use super::*;
use ratatui::crossterm::style;
use ratatui::style::Color;

#[test]
fn tmux_and_ansi_render_typed_colors_and_modifiers() {
    let style = Style::new()
        .fg(Color::Rgb(170, 187, 204))
        .bg(Color::Rgb(1, 2, 3))
        .add_modifier(Modifier::BOLD | Modifier::DIM);
    assert_eq!(style.tmux_option(), "fg=#aabbcc,bg=#010203,bold,dim");
    assert_eq!(style.tmux(), "#[fg=#aabbcc,bg=#010203,bold,dim]");
    assert_eq!(Style::new().fg(Color::Reset).tmux_option(), "");
    let names = "black red green yellow blue magenta cyan gray darkgray lightred lightgreen lightyellow lightblue lightmagenta lightcyan white";
    for (index, name) in names.split_whitespace().enumerate() {
        assert_eq!(
            Style::new().fg(name.parse().unwrap()).tmux_option(),
            format!("fg=colour{index}")
        );
    }
    assert_eq!(
        Style::new().bg(Color::Indexed(200)).tmux_option(),
        "bg=colour200"
    );
    style::force_color_output(true);
    assert_eq!(
        style.paint("text"),
        "\x1b[38;2;170;187;204m\x1b[48;2;1;2;3m\x1b[1m\x1b[2mtext\x1b[0m"
    );
}

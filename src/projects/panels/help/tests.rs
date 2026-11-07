use ratatui::{Terminal, backend::TestBackend};

use super::*;

#[test]
fn resizing_scales_the_whole_lockup_and_preserves_help() {
    let theme = Theme::parse(None).unwrap();
    let splash = Splash::load().unwrap();
    let mut terminal = Terminal::new(TestBackend::new(129, 36)).unwrap();
    for (first_run, width, height, help_row, first_char) in [
        (false, 129, 36, 23, "C"),
        (false, 129, 40, 27, "C"),
        (false, 91, 40, 23, "C"),
        (false, 37, 40, 10, "C"),
        (false, 129, 13, 0, "C"),
        (false, 129, 36, 23, "C"),
        (true, 129, 36, 24, "n"),
    ] {
        let text = help_text(first_run, &theme);
        terminal.backend_mut().resize(width, height);
        terminal
            .draw(|frame| draw(frame, &text, &splash, &theme))
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(0, help_row)].symbol(),
            first_char,
            "{width}x{height}"
        );
        if help_row > 0 {
            let art = splash.text(width, help_row - 1);
            for (y, line) in art.lines.iter().enumerate() {
                for (x, character) in line.to_string().chars().enumerate() {
                    assert_eq!(buffer[(x as u16, y as u16)].symbol(), character.to_string());
                }
            }
        }
        assert_help(buffer, 0, help_row, &text);
    }
}

fn assert_help(buffer: &ratatui::buffer::Buffer, x: u16, y: u16, text: &Text<'_>) {
    let mut rendered = String::new();
    for row in y..buffer.area.height {
        for column in x..buffer.area.width {
            rendered.push_str(buffer[(column, row)].symbol());
        }
    }
    let without_whitespace = |value: &str| {
        value
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
    };
    assert_eq!(
        without_whitespace(&rendered),
        without_whitespace(&text.to_string())
    );
}

#[test]
fn help_preserves_first_run_headings_and_state_styles() {
    let theme = Theme::parse(None).unwrap();
    let first = help_text(true, &theme);
    assert_eq!(first.lines[0].to_string(), "niles");
    assert_eq!(first.lines[0].style, theme.style(StyleKey::Heading));
    assert_eq!(first.lines[4].to_string(), "Get started");
    let keys = help_text(false, &theme);
    assert_eq!(keys.lines[9].to_string(), "  ⣾ running   ⚠ waiting");
    assert_eq!(keys.lines[9].spans[1].style, theme.state(State::Running).1);
    assert_eq!(keys.lines[9].spans[3].style, theme.state(State::Waiting).1);
}

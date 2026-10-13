use ratatui::{Terminal, backend::TestBackend};

use super::*;

#[test]
fn resizing_scales_the_whole_lockup_and_preserves_help() {
    let theme = Theme::parse(None).unwrap();
    let splash = Splash::load().unwrap();
    let text = help_text(false, &theme);
    let mut terminal = Terminal::new(TestBackend::new(129, 36)).unwrap();
    for (width, height, help_row) in [
        (129, 36, 24),
        (129, 40, 28),
        (91, 40, 23),
        (37, 40, 10),
        (129, 13, 0),
        (129, 36, 24),
    ] {
        terminal.backend_mut().resize(width, height);
        terminal
            .draw(|frame| draw(frame, &text, &splash, &theme))
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, help_row)].symbol(), "C", "{width}x{height}");
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

#[test]
fn first_run_help_stays_visible_with_the_scaled_lockup() {
    let theme = Theme::parse(None).unwrap();
    let splash = Splash::load().unwrap();
    let text = help_text(true, &theme);
    let mut terminal = Terminal::new(TestBackend::new(129, 36)).unwrap();
    terminal
        .draw(|frame| draw(frame, &text, &splash, &theme))
        .unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(0, 27)].symbol(), "n");
    assert_help(buffer, 0, 27, &text);
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

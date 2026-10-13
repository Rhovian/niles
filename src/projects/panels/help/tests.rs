use ratatui::{Terminal, backend::TestBackend};

use super::*;

#[test]
fn splash_requires_room_for_art_and_separator_in_both_dimensions() {
    let full = splash(51, 49).unwrap();
    assert!(full.contains(include_str!("butler.txt")));
    assert!(full.ends_with(WORDMARK));
    assert_eq!(splash(51, 48), Some(WORDMARK));
    assert_eq!(splash(50, 49), Some(WORDMARK));
    assert_eq!(splash(37, 7), Some(WORDMARK));
    assert_eq!(splash(36, 49), None);
    assert_eq!(splash(51, 6), None);
}

#[test]
fn resizing_reselects_splash_and_keeps_help_below_it() {
    let theme = Theme::parse(None).unwrap();
    let text = help_text(false, &theme);
    let mut terminal = Terminal::new(TestBackend::new(100, 70)).unwrap();
    for (width, height, help_row, splash_row) in [
        (100, 70, 49, Some(1)),
        (100, 24, 7, Some(0)),
        (36, 24, 0, None),
        (100, 70, 49, Some(1)),
    ] {
        terminal.backend_mut().resize(width, height);
        terminal.draw(|frame| draw(frame, &text, &theme)).unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, help_row)].symbol(), "C");
        if let Some(row) = splash_row {
            let x = if row == 1 { 17 } else { 0 };
            assert_eq!(buffer[(x, row)].symbol(), "█");
        }
    }
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

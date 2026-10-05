use super::*;

#[test]
fn styles_accept_the_whitelist_and_preserve_source() {
    let colors = [
        ("default", Color::Reset),
        ("black", Color::Black),
        ("red", Color::Red),
        ("green", Color::Green),
        ("yellow", Color::Yellow),
        ("blue", Color::Blue),
        ("magenta", Color::Magenta),
        ("cyan", Color::Cyan),
        ("white", Color::Gray),
        ("brightblack", Color::DarkGray),
        ("brightred", Color::LightRed),
        ("brightgreen", Color::LightGreen),
        ("brightyellow", Color::LightYellow),
        ("brightblue", Color::LightBlue),
        ("brightmagenta", Color::LightMagenta),
        ("brightcyan", Color::LightCyan),
        ("brightwhite", Color::White),
        ("colour0", Color::Indexed(0)),
        ("color255", Color::Indexed(255)),
        ("#aAbBcC", Color::Rgb(170, 187, 204)),
    ];
    for (text, color) in colors {
        let source = format!("fg={text},bg={text},bold,dim,italics,underscore,reverse");
        let style = Style::parse("theme.styles.waiting", source.clone()).unwrap();
        assert_eq!(style.ratatui.fg, Some(color));
        assert_eq!(style.ratatui.bg, Some(color));
        assert_eq!(
            style.ratatui.add_modifier,
            Modifier::BOLD
                | Modifier::DIM
                | Modifier::ITALIC
                | Modifier::UNDERLINED
                | Modifier::REVERSED
        );
        assert_eq!(style.tmux(), format!("#[{source}]"));
    }
}

#[test]
fn invalid_tokens_name_the_key() {
    for text in [
        "",
        "bold,",
        "blink",
        "italic",
        "fg=orange",
        "fg=color256",
        "fg=color+1",
        "fg=#abc",
        "#(touch /tmp/x)",
        "#[reverse]",
        "bold]",
        "bold, dim",
    ] {
        let error = Style::parse("theme.styles.waiting", text.into())
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("theme.styles.waiting"),
            "{error:#}"
        );
    }
    for value in ["''", "'界'", "[]"] {
        let key = if value == "[]" { "spinner" } else { "waiting" };
        let text = format!("theme:\n  glyphs:\n    {key}: {value}\n");
        let error = Theme::parse(Some(&text)).err().unwrap();
        assert!(
            error.to_string().contains(&format!("theme.glyphs.{key}")),
            "{error:#}"
        );
    }
    for text in [
        "unknown: {}",
        "tmux: {unknown: true}",
        "tmux: {bindings: invalid}",
        "theme: {unknown: {}}",
        "theme: {styles: {blink: bold}}",
        "theme: {glyphs: {unknown: x}}",
    ] {
        assert!(Theme::parse(Some(text)).is_err());
    }
}

#[test]
fn overrides_replace_one_whole_token() {
    let theme = Theme::parse(Some(
        "theme: {styles: {waiting: bold}, glyphs: {idle: '-'}}",
    ))
    .unwrap();
    let defaults = Theme::parse(None).unwrap();
    assert_eq!(theme.state(State::Waiting).1.ratatui.fg, None);
    assert_eq!(theme.state(State::Waiting).1.tmux(), "#[bold]");
    assert_eq!(
        theme.state(State::Running).1.tmux(),
        defaults.state(State::Running).1.tmux()
    );
    assert_eq!(
        theme.state(State::Waiting).0,
        defaults.state(State::Waiting).0
    );
    assert_eq!(theme.state(State::Idle).0, "-");
}

#[test]
fn guide_glyphs_require_the_default_width() {
    let error = Theme::parse(Some("theme: {glyphs: {branch: '+'}}"))
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("theme.glyphs.branch"));
    assert!(error.contains("width 2"));
}

#[test]
fn ansi_paint_uses_parsed_colors_and_modifiers_and_resets() {
    let text = "fg=#aabbcc,bg=color2,bold,dim,italics,underscore,reverse";
    let style = Style::parse("test", text.into()).unwrap();
    let colors = if ratatui::crossterm::style::Colored::ansi_color_disabled_memoized() {
        "\x1b[m\x1b[m"
    } else {
        "\x1b[38;2;170;187;204m\x1b[48;5;2m"
    };
    assert_eq!(
        style.paint("text"),
        format!("{colors}\x1b[1m\x1b[2m\x1b[3m\x1b[4m\x1b[7mtext\x1b[0m")
    );
}

#[test]
fn bar_option_text_is_the_validated_source() {
    let theme = Theme::parse(None).unwrap();
    assert_eq!(
        theme.style(StyleKey::Bar).tmux_option(),
        "fg=#c0caf5,bg=#1a1b26"
    );
}

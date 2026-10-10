use super::*;

#[test]
fn help_text_depends_on_whether_registry_is_empty() {
    let theme = Theme::parse(None).unwrap();
    assert_eq!(
        help_text(true, &theme),
        FIRST_RUN
            .replacen("niles", &theme.style(StyleKey::Heading).paint("niles"), 1)
            .replace(
                "Get started",
                &theme.style(StyleKey::Heading).paint("Get started")
            )
    );
    assert!(help_text(false, &theme).contains(&format!(
        "{} running   {} waiting",
        theme.state(State::Running).1.paint("⣾"),
        theme.state(State::Waiting).1.paint("⚠")
    )));
}

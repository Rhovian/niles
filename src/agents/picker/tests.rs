use super::*;
use crate::agents::ModelRoster;
use crate::agents::roster::Tier;
use crate::config::spec::ProjectConfig;
use crate::{test_support, workspace_manifest};
use columns::{Column, Columns, Pick};
use form::{Decision, Screen};
use ratatui::crossterm::event::KeyCode;

fn config() -> ProjectConfig {
    ProjectConfig {
        models: ModelRoster::builtin().unwrap(),
    }
}

fn columns(value: &str) -> Columns {
    Columns::new(&columns::families(&config()).unwrap(), value, &config()).unwrap()
}

fn keys(form: &mut Form, keys: impl IntoIterator<Item = KeyCode>) {
    for key in keys {
        form.key(key, &config()).unwrap();
    }
}

fn choose(form: &mut Form, value: &str) {
    let picker = match &mut form.screen {
        Screen::Editing { columns, .. } => Some(columns),
        Screen::Tiers | Screen::Entry { .. } | Screen::Review => None,
    }
    .expect("edit screen");
    **picker = columns(value);
    keys(form, [KeyCode::Enter; 3]);
}

fn render(form: &Form) -> String {
    let theme = Theme::parse(None).unwrap();
    test_support::render(140, 24, |frame| draw::form(frame, form, &theme, "/project"))
}

fn form() -> Form {
    Form::new(test_support::manifest(), &config()).unwrap()
}

fn entry(form: &mut Form) {
    form.selection
        .prefer(|row| matches!(row, form::EditRow::Entry));
    keys(form, [KeyCode::Right]);
}

#[test]
fn current_value_back_navigation_and_effort_completion() {
    for (value, enters) in [
        ("codex:gpt-6.1-sol:high", 2),
        ("claude:opus:high", 2),
        ("claude:haiku", 1),
    ] {
        let mut picker = columns(value);
        for back in [KeyCode::Left, KeyCode::Esc] {
            picker.key(KeyCode::Right);
            assert_eq!(picker.column, Column::Model);
            if enters == 2 {
                picker.key(KeyCode::Enter);
                assert_eq!(picker.column, Column::Effort);
                picker.key(back);
                assert_eq!(picker.column, Column::Model);
            }
            picker.key(back);
            assert_eq!(picker.column, Column::Family);
            assert!(matches!(picker.key(back), Pick::Cancelled));
        }
        for _ in 0..enters {
            assert!(matches!(picker.key(KeyCode::Enter), Pick::Pending));
        }
        assert!(
            matches!(picker.key(KeyCode::Enter), Pick::Selected(selected) if selected == value)
        );
    }
}

#[test]
fn menu_is_effective_roster_only_and_defaults_to_first_entry() {
    assert!(Columns::new(&[], "claude", &config()).is_err());
    let mut picker = columns("codex:gpt-5.4:high");
    let models = picker.models.as_ref().unwrap();
    assert!(!models.iter().any(|model| model.name == "gpt-5.4"));
    assert_eq!(models.current().name, "gpt-5.5");
    picker.key(KeyCode::Enter);
    for key in [KeyCode::Down, KeyCode::Up] {
        for _ in 0..100 {
            picker.key(key);
        }
        let models = picker.models.as_ref().unwrap();
        let edge = if key == KeyCode::Down {
            models.iter().last()
        } else {
            models.iter().next()
        };
        assert_eq!(models.current().name, edge.unwrap().name);
    }
}

#[test]
fn editing_primary_renders_hints_preserves_other_rows_and_can_be_restored() {
    let mut form = form();
    assert!(!render(&form).contains("s save"));
    assert!(render(&form).contains("c continue"));
    assert!(matches!(
        form.key(KeyCode::Char('c'), &config()).unwrap(),
        Some(Decision::Keep)
    ));
    keys(&mut form, [KeyCode::Char('s'), KeyCode::Right]);
    if let Screen::Editing { columns, .. } = &mut form.screen {
        columns.families.current_mut().installed = false;
    }
    let text = render(&form);
    assert!(text.contains("codex (not installed)") && text.contains("low medium high xhigh max"));
    assert!(!text.contains("cli default"));
    keys(&mut form, [KeyCode::Esc]);
    assert_eq!(form.draft, form.original);
    keys(&mut form, [KeyCode::Right]);
    choose(&mut form, "codex:gpt-6.1-sol:low");
    let mut expected = form.original.tiers.clone();
    expected.standard.primary = "codex:gpt-6.1-sol:low".into();
    assert_eq!(form.draft.tiers, expected);
    assert!(render(&form).contains("s save"));
    assert!(form.key(KeyCode::Char('c'), &config()).unwrap().is_none());
    keys(&mut form, [KeyCode::Right]);
    choose(&mut form, "codex:gpt-6.1-sol:medium");
    assert!(!form.can_save());
    keys(&mut form, [KeyCode::Char('s')]);
    assert!(matches!(form.screen, Screen::Tiers));
}

#[test]
fn setup_and_configuration_share_editing_but_enter_continues_setup() {
    for setup in [false, true] {
        let mut manifest = test_support::manifest();
        manifest.entry_agent = manifest.tiers.standard.primary.clone();
        let mut form = Form::new(manifest, &config()).unwrap();
        form.setup = setup;
        form.select_agent(Tier::Frontier, 0).unwrap();
        keys(&mut form, [KeyCode::Right]);
        assert!(matches!(
            form.screen,
            Screen::Editing {
                tier: Tier::Frontier,
                ..
            }
        ));
        choose(&mut form, "claude:fable:high");
        let draft = form.draft.clone();
        keys(&mut form, [KeyCode::Right, KeyCode::Enter, KeyCode::Esc]);
        assert!(matches!(form.screen, Screen::Editing { .. }));
        keys(&mut form, [KeyCode::Esc]);
        assert_eq!(form.draft, draft);
        if setup {
            assert_eq!(
                SetupTiers::key(&mut form, KeyCode::Esc, &config()).unwrap(),
                Some(SetupAction::Back)
            );
            assert!(matches!(
                form.key(KeyCode::Enter, &config()).unwrap(),
                Some(Decision::Save)
            ));
        } else {
            keys(&mut form, [KeyCode::Char('s')]);
            let decision = form.key(KeyCode::Enter, &config()).unwrap().unwrap();
            assert!(matches!(choice(&form, decision), Choice::Save(saved) if *saved == draft));
        }
        assert!(matches!(
            form.key(KeyCode::Char('q'), &config()).unwrap(),
            Some(Decision::Quit)
        ));
    }
    let mut form = form();
    form.setup = true;
    assert!(matches!(
        form.key(KeyCode::Enter, &config()).unwrap(),
        Some(Decision::Save)
    ));
}

#[test]
fn secondary_edit_reorder_boundaries_and_removal_preserve_primary() {
    let mut form = form();
    let original = form.draft.clone();
    keys(&mut form, [KeyCode::Char('a')]);
    choose(&mut form, "claude:fable:medium");
    let last = form.draft.tiers.standard.secondaries.len();
    form.select_agent(Tier::Standard, last).unwrap();
    keys(&mut form, [KeyCode::Right]);
    choose(&mut form, "claude:fable:high");
    assert_eq!(render(&form).matches("changed").count(), 1);
    let before = form.draft.tiers.standard.secondaries.clone();
    assert_eq!(before.last().unwrap(), "claude:fable:high");
    keys(&mut form, [KeyCode::Char(']')]);
    assert_eq!(form.draft.tiers.standard.secondaries, before);
    keys(&mut form, [KeyCode::Char('[')]);
    assert_ne!(form.draft.tiers.standard.secondaries, before);
    keys(&mut form, [KeyCode::Char(']')]);
    assert_eq!(form.draft.tiers.standard.secondaries, before);
    while !form.draft.tiers.standard.secondaries.is_empty() {
        form.select_agent(Tier::Standard, 1).unwrap();
        let before = form.draft.tiers.standard.secondaries.clone();
        keys(&mut form, [KeyCode::Char('[')]);
        assert_eq!(form.draft.tiers.standard.secondaries, before);
        keys(&mut form, [KeyCode::Char('d')]);
    }
    keys(&mut form, [KeyCode::Char('d')]);
    assert_eq!(
        form.draft.tiers.standard.primary,
        original.tiers.standard.primary
    );
    assert_eq!(form.draft.tiers.frontier, original.tiers.frontier);
    assert_eq!(form.draft.entry_agent, original.entry_agent);
    workspace_manifest::check(&form.draft, &config().models).unwrap();
}

#[test]
fn duplicate_primary_secondary_and_effort_aliases_are_refused() {
    for primary in ["codex:gpt-6.1-sol:medium", "codex:gpt-6.1-sol:med"] {
        let mut manifest = test_support::manifest();
        manifest.tiers.standard.primary = primary.into();
        let secondary = manifest.tiers.standard.secondaries.first().unwrap().clone();
        let mut form = Form::new(manifest, &config()).unwrap();
        for (key, value) in [
            (KeyCode::Char('a'), "codex:gpt-6.1-sol:medium"),
            (KeyCode::Right, secondary.as_str()),
        ] {
            keys(&mut form, [key]);
            choose(&mut form, value);
            assert_eq!(form.draft, form.original);
            assert!(render(&form).contains("Duplicate agent"));
        }
    }
}

#[test]
fn absent_tier_can_be_added_and_removed_entry_requires_explicit_choice() {
    let mut manifest = test_support::manifest();
    manifest.tiers.frontier = None;
    manifest.entry_agent = manifest.tiers.standard.primary.clone();
    let mut form = Form::new(manifest, &config()).unwrap();
    assert!(render(&form).contains("(absent"));
    form.select_agent(Tier::Frontier, 0).unwrap();
    keys(&mut form, [KeyCode::Right]);
    choose(&mut form, "claude:fable:high");
    assert!(
        form.draft
            .tiers
            .frontier
            .as_ref()
            .unwrap()
            .secondaries
            .is_empty()
    );
    form.select_agent(Tier::Standard, 0).unwrap();
    keys(&mut form, [KeyCode::Right]);
    choose(&mut form, "codex:gpt-6.1-sol:low");
    assert!(render(&form).contains("Entry agent is no longer listed"));
    keys(&mut form, [KeyCode::Char('s')]);
    assert!(matches!(form.screen, Screen::Tiers));
    assert!(form.error.contains("invalid entry_agent"));
    entry(&mut form);
    if let Screen::Entry { choices } = &mut form.screen {
        choices.prefer(|value| value == "claude:fable:high");
    }
    keys(&mut form, [KeyCode::Enter, KeyCode::Char('s')]);
    assert_eq!(form.draft.entry_agent, "claude:fable:high");
    assert!(matches!(form.screen, Screen::Review));
}

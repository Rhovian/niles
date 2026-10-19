use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyCode};

use super::*;
use crate::{agents::ModelRoster, config::spec::ProjectConfig};
use columns::Column;
use form::Screen;

fn config() -> ProjectConfig {
    ProjectConfig {
        agents: Default::default(),
        models: ModelRoster::builtin().unwrap(),
    }
}

fn columns(value: &str) -> Columns {
    let config = config();
    Columns::new(&columns::families(&config).unwrap(), value, &config).unwrap()
}

fn selected(pick: Pick) -> String {
    match pick {
        Pick::Selected(value) => value,
        Pick::Pending | Pick::Cancelled => panic!("pick did not complete"),
    }
}

#[test]
fn current_value_is_selected_and_back_undoes_every_level() {
    let mut picker = columns("codex:gpt-6.1-sol:high");
    assert_eq!(picker.families[picker.family].name, "codex");
    assert_eq!(
        picker.families[picker.family].models[picker.model].name,
        "gpt-6.1-sol"
    );
    assert_eq!(
        picker.families[picker.family].models[picker.model].efforts[picker.effort],
        "high"
    );
    for back in [KeyCode::Left, KeyCode::Esc] {
        picker.key(KeyCode::Right);
        picker.key(KeyCode::Enter);
        assert_eq!(picker.column, Column::Effort);
        picker.key(back);
        assert_eq!(picker.column, Column::Model);
        picker.key(back);
        assert_eq!(picker.column, Column::Family);
        assert!(matches!(picker.key(back), Pick::Cancelled));
    }
    picker.key(KeyCode::Enter);
    picker.key(KeyCode::Enter);
    assert_eq!(
        selected(picker.key(KeyCode::Enter)),
        "codex:gpt-6.1-sol:high"
    );
}

#[test]
fn picker_requires_exact_effort_and_skips_it_only_for_no_effort_models() {
    let mut picker = columns("claude:opus:high");
    picker.key(KeyCode::Enter);
    picker.key(KeyCode::Enter);
    assert_eq!(selected(picker.key(KeyCode::Enter)), "claude:opus:high");
    let mut picker = columns("claude:haiku");
    picker.key(KeyCode::Enter);
    assert_eq!(selected(picker.key(KeyCode::Enter)), "claude:haiku");
}

#[test]
fn menu_is_effective_roster_only_and_defaults_to_profile_model() {
    let mut picker = columns("codex:gpt-5.4:high");
    let family = &picker.families[picker.family];
    assert!(!family.models.iter().any(|model| model.name == "gpt-5.4"));
    assert_eq!(family.models[picker.model].name, "gpt-5.5");
    picker.key(KeyCode::Enter);
    for _ in 0..100 {
        picker.key(KeyCode::Down);
    }
    assert_eq!(
        picker.model,
        picker.families[picker.family].models.len() - 1
    );
}

#[test]
fn editing_standard_primary_preserves_secondaries_and_other_tiers() {
    let config = config();
    let mut form = Form::new(WorkspaceManifest::default(), &config).unwrap();
    form.selected = 1;
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Esc, &config).unwrap();
    assert_eq!(form.draft, form.original);
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Up, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    let mut expected = form.original.worker.clone();
    expected.standard.primary = "codex:gpt-6.1-sol:low".into();
    assert_eq!(form.draft.worker, expected);
    assert_eq!(form.draft.lead, form.original.lead);
}

#[test]
fn research_is_edited_in_its_own_binding() {
    let config = config();
    let mut form = Form::new(WorkspaceManifest::default(), &config).unwrap();
    for _ in 0..5 {
        form.key(KeyCode::Down, &config).unwrap();
    }
    form.key(KeyCode::Enter, &config).unwrap();
    assert!(render(&form).contains("ROLES / research"));
    for _ in 0..3 {
        form.key(KeyCode::Enter, &config).unwrap();
    }
    assert_eq!(form.draft.research, form.original.research);
    assert_eq!(form.draft.worker, form.original.worker);
}

#[test]
fn unchanged_draft_hides_and_ignores_save() {
    let config = config();
    let mut form = Form::new(WorkspaceManifest::default(), &config).unwrap();
    assert!(!render(&form).contains("s save"));
    assert!(form.key(KeyCode::Char('s'), &config).unwrap().is_none());
    assert!(matches!(form.screen, Screen::Roles));
}

#[test]
fn restoring_original_draft_hides_and_ignores_save() {
    let config = config();
    let mut form = Form::new(WorkspaceManifest::default(), &config).unwrap();
    form.draft.lead.standard.primary = "claude:sonnet:medium".into();
    assert!(render(&form).contains("s save"));
    form.key(KeyCode::Char('s'), &config).unwrap();
    assert!(matches!(form.screen, Screen::Review));
    form.key(KeyCode::Esc, &config).unwrap();
    form.draft = form.original.clone();
    assert!(!render(&form).contains("s save"));
    assert!(form.key(KeyCode::Char('s'), &config).unwrap().is_none());
    assert!(matches!(form.screen, Screen::Roles));
}

#[test]
fn edited_role_is_reviewed_and_saved() {
    let config = config();
    let mut form = Form::new(WorkspaceManifest::default(), &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Up, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    assert_eq!(form.draft.lead.standard.primary, "claude:sonnet:medium");
    assert!(render(&form).contains("s save"));
    form.key(KeyCode::Char('s'), &config).unwrap();
    assert!(matches!(form.screen, Screen::Review));
    assert!(matches!(
        form.key(KeyCode::Enter, &config).unwrap(),
        Some(Choice::Save(_))
    ));
}

#[test]
fn rendering_marks_uninstalled_families_and_shows_effort_hints() {
    let config = config();
    let mut form = Form::new(WorkspaceManifest::default(), &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    if let Screen::Editing { columns, .. } = &mut form.screen {
        columns.families[0].installed = false;
    }
    let text = render(&form);
    assert!(text.contains("codex (not installed)"), "{text}");
    assert!(text.contains("low medium high xhigh max"));
    assert!(!text.contains("cli default"));
}

fn render(form: &Form) -> String {
    let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
    let theme = Theme::parse(None).unwrap();
    terminal
        .draw(|frame| draw::form(frame, form, &theme, "/project"))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn replacement_explains_why_and_requires_save_or_quit() {
    let config = config();
    let mut form = Form::new(WorkspaceManifest::default(), &config).unwrap();
    form.replacement_reason =
        Some("Cannot parse manifest: missing field `design`\u{1b}".to_owned());
    let text = render(&form);
    assert!(
        text.contains("Cannot parse manifest: missing field `design`"),
        "{text}"
    );
    assert!(!text.contains('\u{1b}'), "{text}");
    assert!(!text.contains("continue"), "{text}");
    assert!(text.contains("s save"), "{text}");
    assert!(form.key(KeyCode::Char('c'), &config).unwrap().is_none());
    form.key(KeyCode::Char('s'), &config).unwrap();
    assert!(matches!(
        form.key(KeyCode::Enter, &config).unwrap(),
        Some(Choice::Save(_))
    ));
    assert!(matches!(
        form.key(KeyCode::Char('q'), &config).unwrap(),
        Some(Choice::Quit)
    ));
}

use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyCode};

use super::*;
use crate::{
    agents::ModelRoster,
    config::spec::ProjectConfig,
    test_support::project_config,
    workspace_manifest::{AgentGroup, RoleBinding},
};
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
fn cli_default_is_last_and_models_without_efforts_skip_the_column() {
    let mut picker = columns("claude:opus");
    picker.key(KeyCode::Enter);
    picker.key(KeyCode::Enter);
    let model = &picker.families[picker.family].models[picker.model];
    assert_eq!(picker.effort, model.efforts.len());
    assert_eq!(selected(picker.key(KeyCode::Enter)), "claude:opus");
    let mut picker = columns("claude:haiku");
    picker.key(KeyCode::Enter);
    assert_eq!(selected(picker.key(KeyCode::Enter)), "claude:haiku");
}

#[test]
fn custom_agents_follow_builtins_and_complete_immediately() {
    let config = project_config(
        "agents: {bot: {binary: /nonexistent/niles-test-bot}, zed: {binary: /bin/sh}}\n",
    )
    .unwrap();
    let families = columns::families(&config).unwrap();
    assert_eq!(
        families
            .iter()
            .map(|family| family.name.as_str())
            .collect::<Vec<_>>(),
        ["codex", "claude", "hermes", "pi", "bot", "zed"]
    );
    assert!(!families[4].installed);
    assert!(families[5].installed);
    let mut picker = Columns::new(&families, "bot", &config).unwrap();
    assert_eq!(selected(picker.key(KeyCode::Enter)), "bot");
    assert!(
        !columns("claude")
            .families
            .iter()
            .any(|family| family.name == "lead")
    );
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

fn grouped() -> WorkspaceManifest {
    WorkspaceManifest {
        worker: RoleBinding(vec![
            AgentGroup {
                when: Some("first".to_owned()),
                models: vec!["codex".to_owned()],
                efforts: None,
            },
            AgentGroup {
                when: Some("second".to_owned()),
                models: vec!["claude".to_owned()],
                efforts: None,
            },
        ]),
        ..WorkspaceManifest::default()
    }
}

#[test]
fn cancelling_group_replacement_keeps_groups_and_other_picks() {
    let config = config();
    let mut form = Form::new(grouped(), &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    for _ in 0..3 {
        form.key(KeyCode::Enter, &config).unwrap();
    }
    assert_eq!(form.draft.lead, "claude:opus");
    let lead = form.draft.lead.clone();
    form.key(KeyCode::Down, &config).unwrap();
    assert!(render(&form).contains("2 groups (hand-edited)"));
    form.key(KeyCode::Enter, &config).unwrap();
    assert!(render(&form).contains("drops its hand-edited groups"));
    form.key(KeyCode::Esc, &config).unwrap();
    assert_eq!(form.draft.worker, form.original.worker);
    assert_eq!(form.draft.lead, lead);
    form.key(KeyCode::Enter, &config).unwrap();
    for _ in 0..3 {
        form.key(KeyCode::Enter, &config).unwrap();
    }
    assert!(form.draft.worker.scalar().is_some());
    form.key(KeyCode::Char('s'), &config).unwrap();
    assert!(matches!(form.screen, Screen::Review));
    assert!(render(&form).contains("changed"));
    form.key(KeyCode::Esc, &config).unwrap();
    assert!(matches!(form.screen, Screen::Roles));
    assert!(matches!(
        form.key(KeyCode::Char('c'), &config).unwrap(),
        Some(Choice::Keep)
    ));
    assert!(matches!(
        form.key(KeyCode::Char('q'), &config).unwrap(),
        Some(Choice::Quit)
    ));
}

#[test]
fn design_is_shown_but_cannot_be_replaced_with_a_single_agent() {
    let config = config();
    let mut form = Form::new(WorkspaceManifest::default(), &config).unwrap();
    form.selected = Role::ALL
        .iter()
        .position(|role| *role == Role::Design)
        .unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    assert!(matches!(form.screen, Screen::Roles));
    assert_eq!(form.draft.design, form.original.design);
    assert!(!render(&form).contains("↵ edit"));
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
    assert_eq!(
        form.draft.research,
        "codex:gpt-6-luna:medium".to_owned().into()
    );
    assert_eq!(form.draft.worker, form.original.worker);
}

#[test]
fn edited_role_is_reviewed_and_saved() {
    let config = config();
    let mut form = Form::new(grouped(), &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Up, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    assert_eq!(form.draft.lead, "claude:opus:max");
    form.key(KeyCode::Char('s'), &config).unwrap();
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
    assert!(text.contains("cli default"));
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

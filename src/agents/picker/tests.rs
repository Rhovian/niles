use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyCode};

use super::*;
use crate::{
    agents::{self, ModelRoster},
    config::spec::ProjectConfig,
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

fn project_config(body: &str) -> anyhow::Result<ProjectConfig> {
    let root = crate::test_support::temp_test_path("picker-config");
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join("niles.yaml"), body)?;
    let result = crate::config::spec::load_project_config_from(&root);
    std::fs::remove_dir_all(root)?;
    result
}

fn columns(value: &str, reviewer: bool) -> Columns {
    let config = config();
    Columns::new(
        &columns::families(&config).unwrap(),
        value,
        reviewer,
        &config,
    )
    .unwrap()
}

fn selected(pick: Pick) -> String {
    match pick {
        Pick::Selected(value) => value,
        Pick::Pending | Pick::Cancelled => panic!("pick did not complete"),
    }
}

#[test]
fn current_value_is_selected_and_back_undoes_every_level() {
    let mut picker = columns("codex:gpt-6.1-sol:high", false);
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
    let mut picker = columns("claude:opus", false);
    picker.key(KeyCode::Enter);
    picker.key(KeyCode::Enter);
    let model = &picker.families[picker.family].models[picker.model];
    assert_eq!(picker.effort, model.efforts.len());
    assert_eq!(selected(picker.key(KeyCode::Enter)), "claude:opus");
    let mut picker = columns("claude:haiku", false);
    picker.key(KeyCode::Enter);
    assert_eq!(selected(picker.key(KeyCode::Enter)), "claude:haiku");
}

#[test]
fn custom_agents_follow_builtins_and_lead_completes_immediately() {
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
    let mut picker = Columns::new(&families, "bot", false, &config).unwrap();
    assert_eq!(selected(picker.key(KeyCode::Enter)), "bot");
    let mut picker = columns("lead", true);
    assert_eq!(selected(picker.key(KeyCode::Right)), "lead");
    assert!(
        !columns("claude", false)
            .families
            .iter()
            .any(|family| family.name == "lead")
    );
}

#[test]
fn menu_is_effective_roster_only_and_defaults_to_profile_model() {
    let mut picker = columns("codex:gpt-5.4:high", false);
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
fn every_builtin_preset_is_valid_and_launchable() {
    let config = config();
    let presets = presets::load(&config).unwrap();
    assert_eq!(presets.len(), 3);
    for preset in presets {
        for (index, value) in preset.values.unwrap().iter().enumerate() {
            if index == 2 && value == "lead" {
                continue;
            }
            agents::invocation(
                value,
                None,
                agents::InvocationDefaults::Worker,
                &config.models,
            )
            .unwrap();
        }
    }
}

#[test]
fn invalid_presets_are_visible_with_reasons_and_cannot_be_applied() {
    let config = project_config("models: {claude: {opus: {efforts: [low]}}}").unwrap();
    let mut form = Form::new(WorkspaceManifest::default(), &config).unwrap();
    assert!(
        form.presets[0]
            .values
            .as_ref()
            .unwrap_err()
            .contains("unsupported claude effort")
    );
    form.key(KeyCode::Char('p'), &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    assert!(matches!(form.screen, Screen::Presets { .. }));
    assert_eq!(form.draft, form.original);
    let text = render(&form);
    assert!(text.contains("all claude"));
    assert!(text.contains("unsupported claude effort"));
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
    form.key(KeyCode::Down, &config).unwrap();
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
        form.key(KeyCode::Char('q'), &config).unwrap(),
        Action::Quit
    ));
}

#[test]
fn preset_can_be_edited_then_reviewed_and_saved() {
    let config = config();
    let mut form = Form::new(grouped(), &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Down, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    assert_eq!(form.draft.lead, "codex:gpt-6.1-sol:medium");
    assert!(form.draft.worker.scalar().is_some());
    form.key(KeyCode::Down, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Down, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    assert_eq!(form.draft.lead, "codex:gpt-6.1-sol:high");
    form.key(KeyCode::Char('s'), &config).unwrap();
    assert!(matches!(
        form.key(KeyCode::Enter, &config).unwrap(),
        Action::Save
    ));
}

#[test]
fn rendering_marks_uninstalled_families_and_shows_effort_hints() {
    let config = config();
    let mut form = Form::new(WorkspaceManifest::default(), &config).unwrap();
    form.key(KeyCode::Down, &config).unwrap();
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

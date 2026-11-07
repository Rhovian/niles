use crate::agents::roster::Tier;
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::{agents::ModelRoster, config::spec::ProjectConfig};
use columns::{Column, Columns, Pick};
use form::{Decision, Screen};

fn config() -> ProjectConfig {
    ProjectConfig {
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
    for (value, enters) in [("claude:opus:high", 2), ("claude:haiku", 1)] {
        let mut picker = columns(value);
        for _ in 0..enters {
            picker.key(KeyCode::Enter);
        }
        assert_eq!(selected(picker.key(KeyCode::Enter)), value);
    }
}

#[test]
fn menu_is_effective_roster_only_and_defaults_to_first_entry() {
    let picker = columns("claude");
    let family = &picker.families[picker.family];
    assert_eq!(family.models[picker.model].name, family.models[0].name);
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
    let mut form = Form::new(crate::test_support::manifest(), &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Esc, &config).unwrap();
    assert_eq!(form.draft, form.original);
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    form.key(KeyCode::Up, &config).unwrap();
    form.key(KeyCode::Enter, &config).unwrap();
    let mut expected = form.original.tiers.clone();
    expected.standard.primary = "codex:gpt-6.1-sol:low".into();
    assert_eq!(form.draft.tiers, expected);
    assert!(form.key(KeyCode::Char('c'), &config).unwrap().is_none());
    form.draft.entry_agent = form.draft.tiers.standard.primary.clone();
    form.key(KeyCode::Char('s'), &config).unwrap();
    assert!(matches!(form.screen, Screen::Review));
    let decision = form.key(KeyCode::Enter, &config).unwrap().unwrap();
    let Choice::Save(saved) = choice(&form, decision) else {
        panic!("save the reviewed draft")
    };
    assert_eq!(*saved, form.draft);
}

fn assert_save_hidden(form: &mut Form) {
    assert!(!render(form).contains("s save"));
    assert!(form.key(KeyCode::Char('s'), &config()).unwrap().is_none());
    assert!(matches!(form.screen, Screen::Tiers));
}

#[test]
fn unchanged_and_restored_drafts_hide_and_ignore_save() {
    let config = config();
    let mut form = Form::new(crate::test_support::manifest(), &config).unwrap();
    assert_save_hidden(&mut form);
    assert!(render(&form).contains("c continue"));
    assert!(matches!(
        form.key(KeyCode::Char('c'), &config).unwrap(),
        Some(Decision::Keep)
    ));

    form.draft.tiers.standard.primary = "claude:sonnet:medium".into();
    assert!(render(&form).contains("s save"));
    form.draft.entry_agent = form.draft.tiers.standard.primary.clone();
    form.key(KeyCode::Char('s'), &config).unwrap();
    assert!(matches!(form.screen, Screen::Review));
    form.key(KeyCode::Esc, &config).unwrap();
    form.draft = form.original.clone();
    assert_save_hidden(&mut form);
}

#[test]
fn rendering_marks_uninstalled_families_and_shows_effort_hints() {
    let config = config();
    let mut form = Form::new(crate::test_support::manifest(), &config).unwrap();
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
    let theme = Theme::parse(None).unwrap();
    crate::test_support::render(140, 24, |frame| draw::form(frame, form, &theme, "/project"))
}

#[test]
fn picker_rows_follow_project_tiers() {
    let mut manifest = crate::test_support::manifest();
    let binding = manifest.bindings.values().next().unwrap().clone();
    manifest.bindings = ["zeta", "beta", "alpha"]
        .into_iter()
        .map(|name| (name.parse().unwrap(), binding.clone()))
        .collect();
    manifest.entry = "zeta".parse().unwrap();
    let mut form = Form::new(manifest, &config()).unwrap();
    select_agent(&mut form, Tier::Frontier, 0);
    form.key(KeyCode::Enter, &config()).unwrap();
    assert!(matches!(&form.screen, Screen::Editing { tier, .. } if *tier == Tier::Frontier));
    for _ in 0..3 {
        form.key(KeyCode::Enter, &config()).unwrap();
    }
    assert_eq!(form.draft, form.original);
}

#[test]
fn setup_accepts_unchanged_detected_tiers_with_enter() {
    let config = config();
    let manifest = crate::test_support::manifest();
    let mut form = Form::new(manifest.clone(), &config).unwrap();
    form.setup = true;
    assert!(!form.can_save());
    assert!(matches!(
        form.key(KeyCode::Enter, &config).unwrap(),
        Some(Decision::Save)
    ));
}

#[test]
fn right_arrow_edits_in_setup_and_after_setup() {
    let config = config();
    for setup in [false, true] {
        let mut form = Form::new(crate::test_support::manifest(), &config).unwrap();
        form.setup = setup;
        form.key(KeyCode::Right, &config).unwrap();
        assert!(matches!(form.screen, Screen::Editing { .. }));
    }
}

fn choose(form: &mut Form, value: &str) {
    let Screen::Editing { tier, index, .. } = &form.screen else {
        panic!("edit screen")
    };
    form.screen = Screen::Editing {
        tier: *tier,
        index: *index,
        columns: columns(value),
    };
    for _ in 0..3 {
        form.key(KeyCode::Enter, &config()).unwrap();
    }
}

fn select_agent(form: &mut Form, tier: Tier, index: usize) {
    form.selected = form
        .rows()
        .iter()
        .position(|row| {
            matches!(row,
        form::Row::Agent { tier: t, index: i } if *t == tier && *i == index)
        })
        .unwrap();
}

#[test]
fn secondaries_can_be_added_edited_reordered_and_all_removed() {
    let mut form = Form::new(crate::test_support::manifest(), &config()).unwrap();
    let original = form.draft.clone();
    form.key(KeyCode::Char('a'), &config()).unwrap();
    choose(&mut form, "claude:fable:medium");
    assert_eq!(
        form.draft.tiers.standard.secondaries.last().unwrap(),
        "claude:fable:medium"
    );
    let index = form.draft.tiers.standard.secondaries.len();
    select_agent(&mut form, Tier::Standard, index);
    form.key(KeyCode::Right, &config()).unwrap();
    choose(&mut form, "claude:fable:high");
    let before = form.draft.tiers.standard.secondaries.clone();
    form.key(KeyCode::Char('['), &config()).unwrap();
    let mut expected = before.clone();
    expected.swap(index - 1, index - 2);
    assert_eq!(form.draft.tiers.standard.secondaries, expected);
    form.key(KeyCode::Char(']'), &config()).unwrap();
    assert_eq!(form.draft.tiers.standard.secondaries, before);
    while !form.draft.tiers.standard.secondaries.is_empty() {
        select_agent(&mut form, Tier::Standard, 1);
        form.key(KeyCode::Char('d'), &config()).unwrap();
    }
    assert_eq!(
        form.draft.tiers.standard.primary,
        original.tiers.standard.primary
    );
    assert_eq!(form.draft.tiers.frontier, original.tiers.frontier);
    assert_eq!(form.draft.entry_agent, original.entry_agent);
    crate::workspace_manifest::check(&form.draft, &config().models).unwrap();
}

#[test]
fn duplicate_primary_or_secondary_including_effort_aliases_is_refused() {
    let mut form = Form::new(crate::test_support::manifest(), &config()).unwrap();
    let primary = form.draft.tiers.standard.primary.clone();
    form.key(KeyCode::Char('a'), &config()).unwrap();
    choose(&mut form, &primary);
    assert_eq!(form.draft, form.original);
    assert!(render(&form).contains("Duplicate agent"));
    let secondary = form.draft.tiers.standard.secondaries[0].clone();
    form.key(KeyCode::Right, &config()).unwrap();
    choose(&mut form, &secondary);
    assert_eq!(form.draft, form.original);
    form.draft.tiers.standard.primary = "codex:gpt-6.1-sol:med".into();
    form.original = form.draft.clone();
    form.key(KeyCode::Char('a'), &config()).unwrap();
    choose(&mut form, &primary);
    assert_eq!(form.draft, form.original);
    assert!(form.error.contains("Duplicate agent"));
}

#[test]
fn absent_tiers_and_an_independent_entry_agent_are_editable() {
    let mut manifest = crate::test_support::manifest();
    manifest.tiers.frontier = None;
    let mut form = Form::new(manifest, &config()).unwrap();
    assert!(render(&form).contains("(absent"));
    select_agent(&mut form, Tier::Frontier, 0);
    form.key(KeyCode::Right, &config()).unwrap();
    choose(&mut form, "claude:fable:high");
    assert_eq!(
        form.draft.tiers.frontier.as_ref().unwrap().primary,
        "claude:fable:high"
    );
    assert!(
        form.draft
            .tiers
            .frontier
            .as_ref()
            .unwrap()
            .secondaries
            .is_empty()
    );
    let old_entry = form.draft.entry_agent.clone();
    form.selected = form.rows().len() - 1;
    form.key(KeyCode::Right, &config()).unwrap();
    let selected = form
        .entry_choices()
        .iter()
        .position(|agent| *agent == "claude:fable:high")
        .unwrap();
    form.screen = Screen::Entry { selected };
    form.key(KeyCode::Enter, &config()).unwrap();
    assert_ne!(form.draft.entry_agent, old_entry);
    assert_eq!(form.draft.entry_agent, "claude:fable:high");
    crate::workspace_manifest::check(&form.draft, &config().models).unwrap();
}

#[test]
fn removing_entry_requires_an_explicit_new_entry_choice_before_save() {
    let mut form = Form::new(crate::test_support::manifest(), &config()).unwrap();
    form.draft.entry_agent = form.draft.tiers.standard.primary.clone();
    let old_entry = form.draft.entry_agent.clone();
    form.key(KeyCode::Right, &config()).unwrap();
    choose(&mut form, "claude:fable:high");
    assert_eq!(form.draft.entry_agent, old_entry);
    assert!(render(&form).contains("Entry agent is no longer listed"));
    form.key(KeyCode::Char('s'), &config()).unwrap();
    assert!(matches!(form.screen, Screen::Tiers));
    assert!(form.error.contains("invalid entry_agent"));
    form.selected = form.rows().len() - 1;
    form.key(KeyCode::Right, &config()).unwrap();
    form.key(KeyCode::Enter, &config()).unwrap();
    form.key(KeyCode::Char('s'), &config()).unwrap();
    assert!(matches!(form.screen, Screen::Review));
}

#[test]
fn setup_back_leaves_columns_before_leaving_table_and_keeps_edits() {
    let mut form = Form::new(crate::test_support::manifest(), &config()).unwrap();
    form.setup = true;
    form.key(KeyCode::Right, &config()).unwrap();
    choose(&mut form, "claude:fable:high");
    let draft = form.draft.clone();
    form.key(KeyCode::Right, &config()).unwrap();
    form.key(KeyCode::Enter, &config()).unwrap();
    form.key(KeyCode::Esc, &config()).unwrap();
    assert!(matches!(form.screen, Screen::Editing { .. }));
    form.key(KeyCode::Esc, &config()).unwrap();
    assert!(matches!(form.screen, Screen::Tiers));
    assert_eq!(
        SetupTiers::key(&mut form, KeyCode::Esc, &config()).unwrap(),
        Some(SetupAction::Back)
    );
    assert_eq!(form.draft, draft);
    assert!(matches!(
        form.key(KeyCode::Char('q'), &config()).unwrap(),
        Some(Decision::Quit)
    ));
}

#[test]
fn changing_one_secondary_marks_only_that_row_changed() {
    let mut form = Form::new(crate::test_support::manifest(), &config()).unwrap();
    form.draft.tiers.standard.secondaries[0] = "claude:fable:medium".into();
    assert_eq!(render(&form).matches("changed").count(), 1);
}

use super::*;
use crate::projects::registry::{Entry, ProjectName};
use crate::projects::rows::{Row, State};
use crate::projects::status::Segment;
use crate::projects::windows::{SessionAgents, WindowKind};
use crate::theme::State as ThemeState;

const PANEL_LABELS: [&str; 8] = [
    "CONFIG",
    "├─ global",
    "└─ ▸ projects",
    "TELEMETRY",
    "├─ today",
    "├─ 7 days",
    "└─ 30 days",
    "HELP",
];

pub(in crate::projects::explorer) fn project(
    name: &str,
    state: State,
    agents: Option<SessionAgents>,
) -> Project {
    let entry = Entry {
        name: ProjectName::parse(name).unwrap(),
        path: "/tmp".into(),
    };
    let row = Row { entry, state };
    Project { row, agents }
}

fn window(name: &str, kind: WindowKind, label: &str) -> AgentWindow {
    let segment = Segment {
        label: label.into(),
        state: Some(ThemeState::Running),
        tokens: None,
        age: None,
    };
    AgentWindow {
        name: name.into(),
        kind,
        segment,
    }
}

fn lead_only() -> SessionAgents {
    SessionAgents {
        windows: vec![window("niles", WindowKind::Lead, "start")],
        lost: Vec::new(),
    }
}

fn with_panels<'a>(labels: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    labels.into_iter().chain(PANEL_LABELS).collect()
}

fn projects() -> Vec<Project> {
    let agents = SessionAgents {
        windows: vec![
            window("niles", WindowKind::Lead, "start"),
            window(
                "parse",
                WindowKind::Worker("parse".into(), "worker".parse().unwrap()),
                "parse",
            ),
        ],
        lost: vec![("gone".into(), "worker".parse().unwrap())],
    };
    vec![
        project("api", State::Running, Some(agents)),
        project("old", State::NotRunning, None),
    ]
}

pub(in crate::projects::explorer) fn tree() -> Tree {
    let mut tree = Tree::default();
    tree.replace(projects());
    tree.down();
    tree
}

pub(in crate::projects::explorer) fn labels(tree: &Tree) -> Vec<String> {
    let now = DateTime::from_timestamp_millis(0).unwrap();
    let items = tree.items();
    let theme = Theme::parse(None).unwrap();
    items
        .into_iter()
        .map(|item| tree.label(item, now, &theme, 36).to_string())
        .collect()
}

#[test]
fn collapsed_projects_and_folders_survive_refresh() {
    let mut tree = tree();
    tree.down();
    tree.down();
    tree.collapse();
    tree.collapse();
    tree.replace(projects());
    assert_eq!(tree.items().len(), 11);
    tree.expand();
    assert_eq!(tree.items().len(), 12);
    tree.down();
    tree.expand();
    assert_eq!(tree.items().len(), 14);
}

fn selected_window(tree: &Tree, project: &str, window: &str) -> bool {
    tree.selected().map(Item::key) == Some(Key::Window(project, window))
}

#[test]
fn cycling_projects_wraps_over_running_project_rows() {
    let mut tree = tree();
    let lead = lead_only();
    let mut projects = projects();
    projects.push(project("web", State::Running, Some(lead)));
    tree.replace(projects);
    assert!(tree.cycle_projects(-1));
    assert!(tree.selected().map(Item::key) == Some(Key::Project("web")));
    assert!(tree.cycle_projects(1));
    assert!(tree.selected().map(Item::key) == Some(Key::Project("api")));
    tree.replace(Vec::new());
    assert!(!tree.cycle_projects(1));
}

#[test]
fn cycling_windows_wraps_within_the_selected_project() {
    let mut tree = tree();
    assert!(tree.cycle_windows(1));
    assert!(selected_window(&tree, "api", "parse"));
    assert!(
        !tree
            .collapsed_folders
            .contains(&("api".into(), "worker".parse().unwrap()))
    );
    assert!(tree.cycle_windows(1));
    assert!(tree.selected().map(Item::key) == Some(Key::Project("api")));
    assert!(tree.cycle_windows(-1));
    assert!(selected_window(&tree, "api", "parse"));
    assert!(tree.cycle_windows(1));
    tree.select(Key::Project("old"));
    assert!(!tree.cycle_windows(1));
}

#[test]
fn projects_without_agents_do_not_expand() {
    let mut tree = tree();
    tree.select(Key::Project("old"));
    tree.expand();
    assert_eq!(tree.items().len(), 14);
    tree.down();
    assert_eq!(tree.cursor(), 6);
}

#[test]
fn refresh_keeps_the_selection_by_name() {
    let mut tree = tree();
    tree.select(Key::Project("old"));
    let mut projects = projects();
    projects.reverse();
    tree.replace(projects);
    assert!(matches!(
        tree.selected(),
        Some(Item::Project(project)) if project.row.entry.name.as_str() == "old"
    ));
    assert_eq!(tree.cursor(), 1);
    tree.replace(Vec::new());
    assert!(matches!(
        tree.selected(),
        Some(Item::Panel(Heading::Config))
    ));
}
#[test]
fn project_scopes_expand_under_config_and_collapse_to_their_folder() {
    let mut tree = tree();
    tree.select(Key::ProjectScopes);
    tree.expand();
    let labels = labels(&tree);
    let config = labels.iter().position(|label| label == "CONFIG").unwrap();
    assert_eq!(
        labels
            .iter()
            .skip(config)
            .take(5)
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "CONFIG",
            "├─ global",
            "└─ ▾ projects",
            "   ├─ api",
            "   └─ old"
        ]
    );
    tree.select(Key::Scope(Some("old")));
    tree.collapse();
    assert!(tree.selected().map(Item::key) == Some(Key::ProjectScopes));
    assert!(
        !tree
            .items()
            .iter()
            .any(|item| matches!(item, Item::Scope(Some(_))))
    );
}

#[test]
fn panels_keep_selection_and_cycle_only_to_windows() {
    let mut tree = tree();
    let panels = [
        Item::Panel(Heading::Config),
        Item::Scope(None),
        Item::ProjectScopes,
        Item::Panel(Heading::Telemetry),
    ]
    .into_iter()
    .chain(Range::ALL.map(Item::Range))
    .chain([Item::Panel(Heading::Help)]);
    for panel in panels {
        tree.cursor = tree
            .items()
            .iter()
            .position(|item| item.key() == panel.key())
            .unwrap();
        tree.replace(projects());
        assert!(tree.selected().map(Item::key) == Some(panel.key()));
        assert!(!tree.cycle_windows(1));
        assert!(tree.cycle_projects(1));
        assert!(tree.selected().map(Item::key) == Some(Key::Project("api")));
    }
}

#[test]
fn waiting_project_shows_warning_and_spinner() {
    let agents = lead_only();
    let waiting_none = project("wait", State::Waiting(None), Some(agents));
    let mut tree = Tree::default();
    tree.replace(vec![waiting_none]);
    assert_eq!(labels(&tree), with_panels(["PROJECTS", "└─   ⚠ wait"]));
    let now0 = DateTime::from_timestamp_millis(0).unwrap();
    let now120 = DateTime::from_timestamp_millis(120).unwrap();
    let now960 = DateTime::from_timestamp_millis(960).unwrap();
    let theme = Theme::parse(None).unwrap();
    assert_eq!(spinner(now0, &theme), "⣾");
    assert_eq!(spinner(now120, &theme), "⣽");
    assert_eq!(spinner(now960, &theme), "⣾");
}

#[test]
fn exact_labels_with_two_folders_and_a_lost_reviewer() {
    let mut projects = projects();
    let agents = projects.first_mut().unwrap().agents.as_mut().unwrap();
    let reviewer = WindowKind::Worker("review-parse".into(), "reviewer".parse().unwrap());
    agents
        .windows
        .push(window("review-parse", reviewer, "review-parse"));
    agents
        .lost
        .push(("gone".into(), "reviewer".parse().unwrap()));
    agents.windows.first_mut().unwrap().segment.age = Some("3m".into());
    let web = SessionAgents {
        windows: Vec::new(),
        lost: Vec::new(),
    };
    projects.insert(
        1,
        project(
            "web",
            State::Waiting(Some(DateTime::from_timestamp(-180, 0).unwrap())),
            Some(web),
        ),
    );
    let mut tree = Tree::default();
    tree.replace(projects);
    tree.select(Key::Window("api", "parse"));
    tree.collapse();
    tree.select(Key::Folder("api", &"reviewer".parse().unwrap()));
    assert_eq!(
        labels(&tree),
        with_panels([
            "PROJECTS",
            "├─ ▾ ⣾ api                        3m",
            "│  ├─ ▾ reviewer 2",
            "│  │  ├─ ● review-parse",
            "│  │  └─ gone window lost",
            "│  └─ ▸ worker 2",
            "├─   ⚠ web",
            "└─     old",
        ])
    );
    let mut refreshed = self::projects();
    let agents = refreshed.first_mut().unwrap().agents.as_mut().unwrap();
    agents
        .lost
        .push(("gone".into(), "reviewer".parse().unwrap()));
    tree.replace(refreshed);
    assert!(
        tree.selected().map(Item::key) == Some(Key::Folder("api", &"reviewer".parse().unwrap()))
    );
}

#[test]
fn collapsing_children_moves_to_their_parent() {
    let mut tree = tree();
    tree.down();
    tree.collapse();
    assert_eq!(tree.cursor(), 1);
    assert_eq!(tree.items().len(), 11);
    tree.up();
    assert_eq!(tree.cursor(), 0);

    tree = self::tree();
    assert!(tree.cycle_windows(1));
    tree.collapse();
    assert!(tree.selected().map(Item::key) == Some(Key::Folder("api", &"worker".parse().unwrap())));
    assert!(
        tree.collapsed_folders
            .contains(&("api".into(), "worker".parse().unwrap()))
    );
}

#[test]
fn worker_rows_show_activity_and_right_aligned_tokens() {
    let tree = tree();
    let theme = Theme::parse(None).unwrap();
    let now = DateTime::UNIX_EPOCH;
    let project = tree.projects.first().unwrap();
    let mut worker = window(
        "parse",
        WindowKind::Worker("parse".into(), "worker".parse().unwrap()),
        "parse",
    );
    worker.segment.tokens = Some(12_000);
    for (state, glyph) in [
        (Some(ThemeState::Running), "●"),
        (Some(ThemeState::Idle), "○"),
        (None, "○"),
    ] {
        worker.segment.state = state;
        let item = Item::Member(project, &"worker".parse().unwrap(), Member::Window(&worker));
        let line = tree.label(item, now, &theme, 36);
        let prefix = format!("│     ├─ {glyph} parse");
        let expected = if state.is_some() {
            format!("{prefix}{}12k", " ".repeat(36 - prefix.chars().count() - 3))
        } else {
            prefix
        };
        assert_eq!(line.to_string(), expected);
        if state.is_some() {
            assert_eq!(line.width(), 36);
            assert!(
                tree.label(item, now, &theme, 10)
                    .to_string()
                    .ends_with("parse 12k")
            );
        }
    }
}

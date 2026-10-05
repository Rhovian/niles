use super::*;
use crate::projects::{
    registry::{Entry, ProjectName},
    status::Segment,
    windows::Role,
};

fn project(name: &str, state: State, agents: Option<SessionAgents>) -> Project {
    let entry = Entry {
        name: ProjectName::parse(name).unwrap(),
        path: "/tmp".into(),
    };
    let row = Row { entry, state };
    Project { row, agents }
}

fn window(index: u32, name: &str, role: Role, label: &str) -> AgentWindow {
    let segment = Segment {
        label: label.into(),
        model: Some("opus".into()),
        state: Some(ThemeState::Running),
        tokens: None,
        age: None,
        highlighted: false,
    };
    AgentWindow {
        index,
        name: name.into(),
        role,
        segment,
    }
}

fn projects() -> Vec<Project> {
    let agents = SessionAgents {
        windows: vec![
            window(0, "niles", Role::Lead, "lead"),
            window(
                1,
                "parse",
                Role::Worker("parse".into(), WorkerRole::Worker),
                "parse",
            ),
        ],
        lost: vec![("gone".into(), WorkerRole::Worker)],
    };
    vec![
        project("api", State::Running, Some(agents)),
        project("old", State::NotRunning, None),
    ]
}

fn tree() -> Tree {
    let mut tree = Tree::default();
    tree.replace(projects());
    tree.down();
    tree
}

fn labels(tree: &Tree) -> Vec<String> {
    let now = DateTime::from_timestamp_millis(0).unwrap();
    let items = tree.items();
    let theme = Theme::parse(None).unwrap();
    items
        .into_iter()
        .map(|item| tree.label(item, now, &theme).to_string())
        .collect()
}

#[test]
fn running_projects_expand_to_role_folders() {
    let mut tree = tree();
    assert_eq!(
        labels(&tree),
        [
            "PROJECTS",
            "  ▸ api  ⣾",
            "    old",
            "CONFIG",
            "TELEMETRY",
            "HELP"
        ]
    );
    tree.expand();
    assert_eq!(
        labels(&tree),
        [
            "PROJECTS",
            "  ▾ api  ⣾",
            "    └─ ▸ workers 2",
            "    old",
            "CONFIG",
            "TELEMETRY",
            "HELP",
        ]
    );
}

fn selected_window(tree: &Tree, project: &str, window: &str) -> bool {
    tree.selected().map(Item::key) == Some(Key::Window(project, window))
}

#[test]
fn cycling_projects_wraps_over_running_project_rows() {
    let mut tree = tree();
    let lead = SessionAgents {
        windows: vec![window(0, "niles", Role::Lead, "lead")],
        lost: Vec::new(),
    };
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
    assert!(tree.folders.contains(&("api".into(), WorkerRole::Worker)));
    assert!(tree.cycle_windows(1));
    assert!(tree.selected().map(Item::key) == Some(Key::Project("api")));
    assert!(tree.cycle_windows(-1));
    assert!(selected_window(&tree, "api", "parse"));
    assert!(tree.cycle_windows(1));
    for _ in 0..4 {
        tree.down();
    }
    assert!(!tree.cycle_windows(1));
}

#[test]
fn projects_without_agents_do_not_expand() {
    let mut tree = tree();
    tree.down();
    tree.expand();
    assert_eq!(tree.items().len(), 6);
    tree.down();
    assert_eq!(tree.cursor(), 3);
}

#[test]
fn collapsing_a_child_moves_to_its_project() {
    let mut tree = tree();
    tree.expand();
    tree.down();
    tree.collapse();
    assert_eq!(tree.cursor(), 1);
    assert_eq!(tree.items().len(), 6);
    tree.up();
    assert_eq!(tree.cursor(), 0);
}

#[test]
fn refresh_keeps_the_selection_by_name() {
    let mut tree = tree();
    tree.down();
    let mut projects = projects();
    projects.reverse();
    tree.replace(projects);
    assert!(matches!(
        tree.selected(),
        Some(Item::Project(project)) if project.row.entry.name.as_str() == "old"
    ));
    assert_eq!(tree.cursor(), 1);
    tree.replace(Vec::new());
    assert!(matches!(tree.selected(), Some(Item::Panel(Panel::Config))));
}
#[test]
fn panels_keep_selection_and_cycle_only_to_windows() {
    let mut tree = tree();
    for panel in [Panel::Config, Panel::Telemetry, Panel::Help] {
        tree.cursor = tree
            .items()
            .iter()
            .position(|item| item.key() == Key::Panel(panel))
            .unwrap();
        tree.replace(projects());
        assert!(matches!(tree.selected(), Some(Item::Panel(selected)) if selected == panel));
        assert!(!tree.cycle_windows(1));
        assert!(tree.cycle_projects(1));
        assert!(tree.selected().map(Item::key) == Some(Key::Project("api")));
    }
}

#[test]
fn waiting_project_shows_warning_and_spinner() {
    let agents = SessionAgents {
        windows: vec![window(0, "niles", Role::Lead, "lead")],
        lost: Vec::new(),
    };
    let waiting_none = project("wait", State::Waiting(None), Some(agents));
    let mut tree = Tree::default();
    tree.replace(vec![waiting_none]);
    assert_eq!(
        labels(&tree),
        vec!["PROJECTS", "  ▸ wait  ⚠", "CONFIG", "TELEMETRY", "HELP",]
    );
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
    let agents = projects[0].agents.as_mut().unwrap();
    let reviewer = Role::Worker("review-parse".into(), WorkerRole::Reviewer);
    agents
        .windows
        .push(window(2, "review-parse", reviewer, "review-parse"));
    agents.lost.push(("gone".into(), WorkerRole::Reviewer));
    let web = SessionAgents {
        windows: Vec::new(),
        lost: Vec::new(),
    };
    projects.insert(1, project("web", State::Waiting(None), Some(web)));
    let mut tree = Tree::default();
    tree.replace(projects);
    tree.down();
    tree.expand();
    tree.down();
    tree.down();
    tree.expand();
    assert_eq!(
        labels(&tree),
        [
            "PROJECTS",
            "  ▾ api  ⣾",
            "    ├─ ▸ workers 2",
            "    └─ ▾ reviewers 2",
            "          ├─ review-parse",
            "          └─ gone window lost",
            "  ▸ web  ⚠",
            "    old",
            "CONFIG",
            "TELEMETRY",
            "HELP"
        ]
    );
    let mut refreshed = self::projects();
    let agents = refreshed[0].agents.as_mut().unwrap();
    agents.lost.push(("gone".into(), WorkerRole::Reviewer));
    tree.replace(refreshed);
    assert!(tree.selected().map(Item::key) == Some(Key::Folder("api", WorkerRole::Reviewer)));
}

#[test]
fn collapsing_a_member_moves_to_its_folder() {
    let mut tree = tree();
    assert!(tree.cycle_windows(1));
    tree.collapse();
    assert!(tree.selected().map(Item::key) == Some(Key::Folder("api", WorkerRole::Worker)));
    assert!(!tree.folders.contains(&("api".into(), WorkerRole::Worker)));
}

#[test]
fn state_suffixes_carry_theme_styles() {
    let mut tree = tree();
    tree.projects[0].row.state = State::Waiting(None);
    let theme = Theme::parse(None).unwrap();
    let now = Utc::now();
    let line = tree.label(Item::Project(&tree.projects[0]), now, &theme);
    assert_eq!(
        line.spans.last().unwrap().style,
        theme.state(ThemeState::Waiting).1.ratatui
    );
    let line = tree.label(
        Item::Member(&tree.projects[0], WorkerRole::Worker, Member::Lost("gone")),
        now,
        &theme,
    );
    assert_eq!(line.spans.last().unwrap().content, "window lost");
    assert_eq!(
        line.spans.last().unwrap().style,
        theme.style(StyleKey::Lost).ratatui
    );
}

#[test]
fn headings_guides_and_counts_carry_theme_styles() {
    let mut tree = tree();
    tree.expand();
    let theme = Theme::parse(None).unwrap();
    let now = Utc::now();
    let heading = tree.label(Item::Header, now, &theme);
    assert_eq!(
        heading.spans[0].style,
        theme.style(StyleKey::Heading).ratatui
    );
    let folder = tree.label(tree.items()[2], now, &theme);
    for span in [&folder.spans[1], &folder.spans[3]] {
        assert_eq!(span.style, theme.style(StyleKey::Guide).ratatui);
    }
    assert_eq!(
        folder.spans.last().unwrap().style,
        theme.style(StyleKey::Muted).ratatui
    );
}

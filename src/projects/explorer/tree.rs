use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::projects::{
    rows::{self, Row, State},
    windows::{AgentWindow, SessionAgents, age},
};

pub(super) struct Project {
    pub row: Row,
    /// Present exactly when the project's lead is running.
    pub agents: Option<SessionAgents>,
}

#[derive(Clone, Copy)]
pub(super) enum Item<'a> {
    Project(&'a Project),
    Window(&'a Project, &'a AgentWindow),
    Lost(&'a Project, &'a str),
}

/// What the cursor is on, by name, so a refresh that reorders rows keeps the selection.
#[derive(PartialEq, Eq)]
enum Key<'a> {
    Project(&'a str),
    Window(&'a str, &'a str),
    Lost(&'a str, &'a str),
}

impl<'a> Item<'a> {
    pub fn project(self) -> &'a Project {
        match self {
            Item::Project(project) | Item::Window(project, _) | Item::Lost(project, _) => project,
        }
    }

    fn key(self) -> Key<'a> {
        let name = self.project().row.entry.name.as_str();
        match self {
            Item::Project(_) => Key::Project(name),
            Item::Window(_, window) => Key::Window(name, &window.name),
            Item::Lost(_, id) => Key::Lost(name, id),
        }
    }
}

#[derive(Default)]
pub(super) struct Tree {
    projects: Vec<Project>,
    expanded: HashSet<String>,
    cursor: usize,
}

impl Tree {
    pub fn replace(&mut self, projects: Vec<Project>) {
        let old = std::mem::replace(&mut self.projects, projects);
        let selected = Tree::items_of(&old, &self.expanded)
            .get(self.cursor)
            .map(|item| item.key());
        let items = self.items();
        self.cursor = match selected.and_then(|key| items.iter().position(|item| item.key() == key))
        {
            Some(index) => index,
            // The selected row is gone; stay at the same height rather than jump to the top.
            None => self.cursor.min(items.len().saturating_sub(1)),
        };
    }

    pub fn items(&self) -> Vec<Item<'_>> {
        Tree::items_of(&self.projects, &self.expanded)
    }

    fn items_of<'a>(projects: &'a [Project], expanded: &HashSet<String>) -> Vec<Item<'a>> {
        let mut items = Vec::new();
        for project in projects {
            items.push(Item::Project(project));
            if let Some(agents) = &project.agents
                && expanded.contains(project.row.entry.name.as_str())
            {
                items.extend(agents.windows.iter().map(|w| Item::Window(project, w)));
                items.extend(agents.lost.iter().map(|id| Item::Lost(project, id)));
            }
        }
        items
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selected(&self) -> Option<Item<'_>> {
        self.items().get(self.cursor).copied()
    }

    pub fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn down(&mut self) {
        if self.cursor + 1 < self.items().len() {
            self.cursor += 1;
        }
    }

    pub fn expand(&mut self) {
        if let Some(Item::Project(project)) = self.selected()
            && project.agents.is_some()
        {
            let name = project.row.entry.name.as_str().to_owned();
            self.expanded.insert(name);
        }
    }

    /// Collapses the selected project, or the project of the selected child and moves onto it.
    pub fn collapse(&mut self) {
        let Some(item) = self.selected() else {
            return;
        };
        let name = item.project().row.entry.name.as_str().to_owned();
        let parent = self
            .items()
            .iter()
            .position(|other| other.key() == Key::Project(&name));
        if let Some(parent) = parent {
            self.cursor = parent;
        }
        self.expanded.remove(&name);
    }

    pub fn label(&self, item: Item<'_>, now: DateTime<Utc>) -> String {
        match item {
            Item::Project(project) => {
                let name = project.row.entry.name.as_str();
                let marker = match &project.agents {
                    Some(_) if self.expanded.contains(name) => "▾",
                    Some(_) => "▸",
                    None => " ",
                };
                let state = match project.row.state {
                    State::Missing => "missing".to_owned(),
                    State::NotRunning => "not running".to_owned(),
                    State::Running => "running".to_owned(),
                    State::Waiting(Some(since)) => format!("waiting {}", age(now, since)),
                    State::Waiting(None) => "waiting".to_owned(),
                };
                let tokens = match project.row.lead_tokens {
                    Some(n) => format!(" · lead {}", rows::abbreviate(n)),
                    None => String::new(),
                };
                format!("{marker} {name}  {state}{tokens}")
            }
            Item::Window(_, window) => format!("    {}", window.segment.text()),
            Item::Lost(_, id) => format!("    {id} window lost"),
        }
    }
}

#[cfg(test)]
mod tests {
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
        let row = Row {
            entry,
            state,
            lead_tokens: Some(12_000),
        };
        Project { row, agents }
    }

    fn window(index: u32, name: &str, role: Role, label: &str) -> AgentWindow {
        let segment = Segment {
            label: label.into(),
            model: Some("opus".into()),
            glyph: Some("●"),
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
                window(1, "parse", Role::Worker("parse".into()), "parse"),
            ],
            lost: vec!["gone".into()],
        };
        vec![
            project("api", State::Running, Some(agents)),
            project("old", State::NotRunning, None),
        ]
    }

    fn tree() -> Tree {
        let mut tree = Tree::default();
        tree.replace(projects());
        tree
    }

    fn labels(tree: &Tree) -> Vec<String> {
        let now = Utc::now();
        let items = tree.items();
        items
            .into_iter()
            .map(|item| tree.label(item, now))
            .collect()
    }

    #[test]
    fn running_projects_expand_to_windows_and_lost_workers() {
        let mut tree = tree();
        assert_eq!(
            labels(&tree),
            ["▸ api  running · lead 12k", "  old  not running · lead 12k"]
        );
        tree.expand();
        assert_eq!(
            labels(&tree),
            [
                "▾ api  running · lead 12k",
                "    lead opus ●",
                "    parse opus ●",
                "    gone window lost",
                "  old  not running · lead 12k",
            ]
        );
        tree.down();
        tree.down();
        assert!(matches!(
            tree.selected(),
            Some(Item::Window(_, window)) if matches!(&window.role, Role::Worker(id) if id == "parse")
        ));
    }

    #[test]
    fn projects_without_agents_do_not_expand() {
        let mut tree = tree();
        tree.down();
        tree.expand();
        assert_eq!(tree.items().len(), 2);
        tree.down();
        assert_eq!(tree.cursor(), 1);
    }

    #[test]
    fn collapsing_a_child_moves_to_its_project() {
        let mut tree = tree();
        tree.expand();
        for _ in 0..3 {
            tree.down();
        }
        tree.collapse();
        assert_eq!(tree.cursor(), 0);
        assert_eq!(tree.items().len(), 2);
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
        assert_eq!(tree.cursor(), 0);
        tree.replace(Vec::new());
        assert!(tree.selected().is_none());
    }
}

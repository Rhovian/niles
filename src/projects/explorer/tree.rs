use std::collections::HashSet;
use std::time::Duration;

use crate::theme::Theme;
use chrono::{DateTime, Utc};

use crate::projects::{
    panels::{Panel, Range},
    rows::Row,
    windows::{AgentWindow, Role, SessionAgents},
};
use crate::worker::WorkerRole;

pub(super) struct Project {
    pub row: Row,
    /// Present exactly when the project's lead is running.
    pub agents: Option<SessionAgents>,
}

impl Project {
    fn members(&self, role: WorkerRole) -> impl Iterator<Item = Item<'_>> {
        self.agents.iter().flat_map(move |agents| {
            let windows = agents
                .windows
                .iter()
                .filter(move |w| matches!(w.role, Role::Worker(_, r) if r == role))
                .map(move |w| Item::Member(self, role, Member::Window(w)));
            let lost = agents
                .lost
                .iter()
                .filter(move |(_, r)| *r == role)
                .map(|(id, r)| Item::Member(self, *r, Member::Lost(id)));
            windows.chain(lost)
        })
    }

    fn roles(&self) -> impl DoubleEndedIterator<Item = WorkerRole> + '_ {
        [
            WorkerRole::Worker,
            WorkerRole::Reviewer,
            WorkerRole::Security,
            WorkerRole::Research,
            WorkerRole::Design,
        ]
        .into_iter()
        .filter(|role| self.members(*role).next().is_some())
    }
}

pub(super) const SPINNER_FRAME: Duration = Duration::from_millis(120);

fn spinner(now: DateTime<Utc>, theme: &Theme) -> &str {
    theme.spinner(now.timestamp_millis() / SPINNER_FRAME.as_millis() as i64)
}

/// A panel's row; CONFIG and TELEMETRY open their first child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Heading {
    Config,
    Telemetry,
    Help,
}

impl Heading {
    pub fn panel(self) -> Panel {
        match self {
            Self::Config => Panel::Config { project: None },
            Self::Telemetry => Panel::Telemetry {
                range: Range::Today,
            },
            Self::Help => Panel::Help,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Item<'a> {
    Header,
    Panel(Heading),
    /// A config scope: the global one under CONFIG, or a project's under
    /// [`Item::ProjectScopes`].
    Scope(Option<&'a Project>),
    /// The folder of project scopes under CONFIG, closed by default.
    ProjectScopes,
    /// A child of TELEMETRY, which is never collapsed.
    Range(Range),
    Project(&'a Project),
    Folder(&'a Project, WorkerRole),
    Member(&'a Project, WorkerRole, Member<'a>),
}

#[derive(Clone, Copy)]
pub(super) enum Member<'a> {
    Window(&'a AgentWindow),
    Lost(&'a str),
}

/// What the cursor is on, by name, so a refresh that reorders rows keeps the selection.
#[derive(PartialEq, Eq)]
enum Key<'a> {
    Header,
    Panel(Heading),
    Scope(Option<&'a str>),
    ProjectScopes,
    Range(Range),
    Project(&'a str),
    Folder(&'a str, WorkerRole),
    Window(&'a str, &'a str),
    Lost(&'a str, &'a str),
}

impl<'a> Item<'a> {
    pub fn project(self) -> Option<&'a Project> {
        match self {
            Item::Project(project) | Item::Folder(project, _) | Item::Member(project, _, _) => {
                Some(project)
            }
            Item::Header
            | Item::Panel(_)
            | Item::Scope(_)
            | Item::ProjectScopes
            | Item::Range(_) => None,
        }
    }

    fn key(self) -> Key<'a> {
        match self {
            Item::Header => Key::Header,
            Item::Panel(heading) => Key::Panel(heading),
            Item::Scope(project) => {
                Key::Scope(project.map(|project| project.row.entry.name.as_str()))
            }
            Item::ProjectScopes => Key::ProjectScopes,
            Item::Range(range) => Key::Range(range),
            Item::Project(project) => Key::Project(project.row.entry.name.as_str()),
            Item::Folder(project, role) => Key::Folder(project.row.entry.name.as_str(), role),
            Item::Member(project, _, Member::Window(window)) => {
                Key::Window(project.row.entry.name.as_str(), &window.name)
            }
            Item::Member(project, _, Member::Lost(id)) => {
                Key::Lost(project.row.entry.name.as_str(), id)
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Tree {
    projects: Vec<Project>,
    collapsed: HashSet<String>,
    collapsed_folders: HashSet<(String, WorkerRole)>,
    project_scopes_expanded: bool,
    cursor: usize,
}

impl Tree {
    pub fn replace(&mut self, projects: Vec<Project>) {
        let old = std::mem::replace(&mut self.projects, projects);
        let selected = self.items_of(&old).get(self.cursor).map(|item| item.key());
        let items = self.items();
        self.cursor = match selected.and_then(|key| items.iter().position(|item| item.key() == key))
        {
            Some(index) => index,
            // The selected row is gone; stay at the same height rather than jump to the top.
            None => self.cursor.min(items.len().saturating_sub(1)),
        };
    }

    pub fn items(&self) -> Vec<Item<'_>> {
        self.items_of(&self.projects)
    }

    fn items_of<'a>(&self, projects: &'a [Project]) -> Vec<Item<'a>> {
        let mut items = vec![Item::Header];
        for project in projects {
            items.push(Item::Project(project));
            if project.agents.is_some() && !self.collapsed.contains(project.row.entry.name.as_str())
            {
                for role in project.roles() {
                    items.push(Item::Folder(project, role));
                    if !self
                        .collapsed_folders
                        .contains(&(project.row.entry.name.as_str().to_owned(), role))
                    {
                        items.extend(project.members(role));
                    }
                }
            }
        }
        items.extend([
            Item::Panel(Heading::Config),
            Item::Scope(None),
            Item::ProjectScopes,
        ]);
        if self.project_scopes_expanded {
            items.extend(projects.iter().map(|project| Item::Scope(Some(project))));
        }
        items.push(Item::Panel(Heading::Telemetry));
        items.extend(Range::ALL.map(Item::Range));
        items.push(Item::Panel(Heading::Help));
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
        match self.selected() {
            Some(Item::Project(project)) if project.agents.is_some() => {
                let name = project.row.entry.name.as_str().to_owned();
                self.collapsed.remove(&name);
            }
            Some(Item::Folder(project, role)) => {
                self.collapsed_folders
                    .remove(&(project.row.entry.name.as_str().to_owned(), role));
            }
            Some(Item::ProjectScopes) => self.project_scopes_expanded = true,
            Some(
                Item::Project(_)
                | Item::Header
                | Item::Panel(_)
                | Item::Scope(_)
                | Item::Range(_)
                | Item::Member(..),
            )
            | None => {}
        }
    }

    /// Cycles running projects, landing on their project rows.
    pub fn cycle_projects(&mut self, steps: isize) -> bool {
        let current = self
            .selected()
            .and_then(Item::project)
            .map(|p| p.row.entry.name.as_str());
        let projects: Vec<&str> = self
            .projects
            .iter()
            .filter(|p| p.agents.is_some())
            .map(|p| p.row.entry.name.as_str())
            .collect();
        let index = projects.iter().position(|&p| Some(p) == current);
        let Some(project) = wrapped(&projects, index, steps).map(|p| (*p).to_owned()) else {
            return false;
        };
        self.select(Key::Project(&project));
        true
    }

    /// Cycles the lead and worker windows; off either, lands on the lead.
    pub fn cycle_windows(&mut self, steps: isize) -> bool {
        let Some(project) = self.selected().and_then(Item::project) else {
            return false;
        };
        let name = project.row.entry.name.as_str();
        let Some(agents) = &project.agents else {
            return false;
        };
        let mut windows = vec![None];
        windows.extend(
            agents
                .windows
                .iter()
                .filter_map(|window| match window.role {
                    Role::Worker(_, role) => Some(Some((window.name.as_str(), role))),
                    Role::Lead | Role::Plain => None,
                }),
        );
        let current = self.selected().map(Item::key);
        let index = windows.iter().position(|&window| {
            current == Some(window.map_or(Key::Project(name), |(w, _)| Key::Window(name, w)))
        });
        let Some(window) = wrapped(&windows, index, steps) else {
            return false;
        };
        let project = name.to_owned();
        let window = window.map(|(w, role)| (w.to_owned(), role));
        match window {
            Some((window, role)) => {
                self.collapsed.remove(&project);
                self.collapsed_folders.remove(&(project.clone(), role));
                self.select(Key::Window(&project, &window));
            }
            None => self.select(Key::Project(&project)),
        }
        true
    }

    fn select(&mut self, key: Key<'_>) {
        #[expect(clippy::expect_used, reason = "the navigation target is visible")]
        let cursor = self
            .items()
            .iter()
            .position(|item| item.key() == key)
            .expect("the target row is listed");
        self.cursor = cursor;
    }

    pub fn collapse(&mut self) {
        let Some(item) = self.selected() else {
            return;
        };
        if let Item::ProjectScopes | Item::Scope(Some(_)) = item {
            self.project_scopes_expanded = false;
            self.select(Key::ProjectScopes);
            return;
        }
        let Some(project) = item.project() else {
            return;
        };
        let name = project.row.entry.name.as_str().to_owned();
        match item {
            Item::Member(_, role, _) => {
                self.collapsed_folders.insert((name.clone(), role));
                self.select(Key::Folder(&name, role));
            }
            Item::Project(_) | Item::Folder(..) => {
                self.collapsed.insert(name.clone());
                self.select(Key::Project(&name));
            }
            Item::Header
            | Item::Panel(_)
            | Item::Scope(_)
            | Item::ProjectScopes
            | Item::Range(_) => {}
        }
    }
}

/// The item `steps` along from `index`, wrapping at the ends; the first when there is no `index`.
fn wrapped<T>(items: &[T], index: Option<usize>, steps: isize) -> Option<&T> {
    let index = index.map_or(0, |index| {
        (index.cast_signed() + steps)
            .rem_euclid(items.len().cast_signed())
            .cast_unsigned()
    });
    items.get(index)
}

mod render;

#[cfg(test)]
mod tests;

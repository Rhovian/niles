use std::collections::HashSet;
use std::time::Duration;

use crate::theme::{GlyphKey, State as ThemeState, StyleKey, Theme};
use chrono::{DateTime, Utc};
use ratatui::text::{Line, Span};

use crate::projects::{
    panels::Panel,
    rows::{Row, State},
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
        ]
        .into_iter()
        .filter(|role| self.members(*role).next().is_some())
    }
}

pub(super) const SPINNER_FRAME: Duration = Duration::from_millis(120);

fn spinner(now: DateTime<Utc>, theme: &Theme) -> &str {
    theme.spinner(now.timestamp_millis() / SPINNER_FRAME.as_millis() as i64)
}

#[derive(Clone, Copy)]
pub(super) enum Item<'a> {
    Header,
    Panel(Panel),
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
    Panel(Panel),
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
            Item::Header | Item::Panel(_) => None,
        }
    }

    fn key(self) -> Key<'a> {
        match self {
            Item::Header => Key::Header,
            Item::Panel(panel) => Key::Panel(panel),
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
    expanded: HashSet<String>,
    folders: HashSet<(String, WorkerRole)>,
    cursor: usize,
}

impl Tree {
    pub fn replace(&mut self, projects: Vec<Project>) {
        let old = std::mem::replace(&mut self.projects, projects);
        let selected = Tree::items_of(&old, &self.expanded, &self.folders)
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
        Tree::items_of(&self.projects, &self.expanded, &self.folders)
    }

    fn items_of<'a>(
        projects: &'a [Project],
        expanded: &HashSet<String>,
        folders: &HashSet<(String, WorkerRole)>,
    ) -> Vec<Item<'a>> {
        let mut items = vec![Item::Header];
        for project in projects {
            items.push(Item::Project(project));
            if project.agents.is_some() && expanded.contains(project.row.entry.name.as_str()) {
                for role in project.roles() {
                    items.push(Item::Folder(project, role));
                    if folders.contains(&(project.row.entry.name.as_str().to_owned(), role)) {
                        items.extend(project.members(role));
                    }
                }
            }
        }
        items.extend([
            Item::Panel(Panel::Config),
            Item::Panel(Panel::Telemetry),
            Item::Panel(Panel::Help),
        ]);
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
                self.expanded
                    .insert(project.row.entry.name.as_str().to_owned());
            }
            Some(Item::Folder(project, role)) => {
                self.folders
                    .insert((project.row.entry.name.as_str().to_owned(), role));
            }
            Some(Item::Project(_) | Item::Header | Item::Panel(_) | Item::Member(..)) | None => {}
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
                self.expanded.insert(project.clone());
                self.folders.insert((project.clone(), role));
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
        let Some(project) = item.project() else {
            return;
        };
        let name = project.row.entry.name.as_str().to_owned();
        match item {
            Item::Member(_, role, _) => {
                self.folders.remove(&(name.clone(), role));
                self.select(Key::Folder(&name, role));
            }
            Item::Project(_) | Item::Folder(..) => {
                self.expanded.remove(&name);
                self.select(Key::Project(&name));
            }
            Item::Header | Item::Panel(_) => {}
        }
    }

    pub fn label<'a>(&self, item: Item<'_>, now: DateTime<Utc>, theme: &'a Theme) -> Line<'a> {
        let guide = |glyph| Span::styled(glyph, theme.style(StyleKey::Guide).ratatui);
        match item {
            Item::Header => Line::from(Span::styled(
                "PROJECTS",
                theme.style(StyleKey::Heading).ratatui,
            )),
            Item::Panel(panel) => Line::from(Span::styled(
                panel.name().to_uppercase(),
                theme.style(StyleKey::Heading).ratatui,
            )),
            Item::Project(project) => {
                let name = project.row.entry.name.as_str();
                let marker = match &project.agents {
                    Some(_) if self.expanded.contains(name) => theme.glyph(GlyphKey::Expanded),
                    Some(_) => theme.glyph(GlyphKey::Collapsed),
                    None => " ",
                };
                let mut line = Line::from(vec![
                    Span::raw("  "),
                    guide(marker),
                    Span::raw(format!(" {name}")),
                ]);
                let suffix = match project.row.state {
                    State::Missing => {
                        Some(Span::styled("missing", theme.style(StyleKey::Lost).ratatui))
                    }
                    State::NotRunning => None,
                    State::Running => Some(Span::styled(
                        spinner(now, theme),
                        theme.state(ThemeState::Running).1.ratatui,
                    )),
                    State::Waiting(_) => {
                        let (glyph, style) = theme.state(ThemeState::Waiting);
                        Some(Span::styled(glyph, style.ratatui))
                    }
                };
                if let Some(suffix) = suffix {
                    line.spans.extend([Span::raw("  "), suffix]);
                }
                line
            }
            Item::Folder(project, role) => {
                let branch = if project.roles().next_back() == Some(role) {
                    theme.glyph(GlyphKey::Last)
                } else {
                    theme.glyph(GlyphKey::Branch)
                };
                let name = project.row.entry.name.as_str().to_owned();
                let marker = if self.folders.contains(&(name, role)) {
                    theme.glyph(GlyphKey::Expanded)
                } else {
                    theme.glyph(GlyphKey::Collapsed)
                };
                let plural = match role {
                    WorkerRole::Worker => "workers",
                    WorkerRole::Reviewer => "reviewers",
                    WorkerRole::Security => "security",
                    WorkerRole::Research => "research",
                };
                let count = project.members(role).count();
                Line::from(vec![
                    Span::raw("    "),
                    guide(branch),
                    Span::raw(" "),
                    guide(marker),
                    Span::raw(format!(" {plural} ")),
                    Span::styled(count.to_string(), theme.style(StyleKey::Muted).ratatui),
                ])
            }
            Item::Member(project, role, member) => {
                let stem = if project.roles().next_back() == Some(role) {
                    " "
                } else {
                    theme.glyph(GlyphKey::Stem)
                };
                let branch = if project.members(role).last().map(Item::key) == Some(item.key()) {
                    theme.glyph(GlyphKey::Last)
                } else {
                    theme.glyph(GlyphKey::Branch)
                };
                let mut line = Line::from(vec![
                    Span::raw("    "),
                    guide(stem),
                    Span::raw("     "),
                    guide(branch),
                    Span::raw(" "),
                ]);
                match member {
                    Member::Window(window) => {
                        line.spans.push(Span::raw(window.segment.label.clone()))
                    }
                    Member::Lost(id) => line.spans.extend([
                        Span::raw(format!("{id} ")),
                        Span::styled("window lost", theme.style(StyleKey::Lost).ratatui),
                    ]),
                }
                line
            }
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

#[cfg(test)]
mod tests;

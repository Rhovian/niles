use super::{Item, Member, Tree, spinner};
use crate::{
    projects::{panels::Range, rows::State, windows::Role},
    theme::{self, State as ThemeState, StyleKey, Theme},
    worker::WorkerRole,
};
use chrono::{DateTime, Utc};
use ratatui::text::{Line, Span};

impl Tree {
    pub fn label<'a>(
        &self,
        item: Item<'_>,
        now: DateTime<Utc>,
        theme: &'a Theme,
        width: u16,
    ) -> Line<'a> {
        let guide = |glyph| Span::styled(glyph, theme.style(StyleKey::Guide));
        let last = item.project().is_some_and(|project| {
            self.projects
                .last()
                .is_some_and(|last| last.row.entry.name == project.row.entry.name)
        });
        let stem = if last { " " } else { theme::STEM };
        match item {
            Item::Header => Line::from(Span::styled("PROJECTS", theme.style(StyleKey::Heading))),
            Item::Panel(heading) => Line::from(Span::styled(
                heading.panel().name().to_uppercase(),
                theme.style(StyleKey::Heading),
            )),
            Item::Scope(None) => Line::from(vec![guide(theme::BRANCH), Span::raw(" global")]),
            Item::Scope(Some(project)) => {
                let last = self
                    .projects
                    .last()
                    .is_some_and(|last| last.row.entry.name == project.row.entry.name);
                Line::from(vec![
                    Span::raw("   "),
                    guide(if last { theme::LAST } else { theme::BRANCH }),
                    Span::raw(format!(" {}", project.row.entry.name.as_str())),
                ])
            }
            Item::ProjectScopes => {
                let marker = if self.projects.is_empty() {
                    " "
                } else if self.project_scopes_expanded {
                    theme::EXPANDED
                } else {
                    theme::COLLAPSED
                };
                Line::from(vec![
                    guide(theme::LAST),
                    Span::raw(" "),
                    guide(marker),
                    Span::raw(" projects"),
                ])
            }
            Item::Range(range) => Line::from(vec![
                guide(if range == Range::Month {
                    theme::LAST
                } else {
                    theme::BRANCH
                }),
                Span::raw(format!(" {}", range.label())),
            ]),
            Item::Project(project) => {
                let name = project.row.entry.name.as_str();
                let marker = if project.roles().next().is_none() {
                    " "
                } else if self.collapsed.contains(name) {
                    theme::COLLAPSED
                } else {
                    theme::EXPANDED
                };
                let (glyph, style) = match project.row.state {
                    State::Running => (spinner(now, theme), theme.state(ThemeState::Running).1),
                    State::Waiting(_) => theme.state(ThemeState::Waiting),
                    State::Starting => theme.state(ThemeState::Starting),
                    State::Missing | State::NotRunning => (" ", theme.style(StyleKey::Muted)),
                };
                let mut line = Line::from(vec![
                    guide(if last { theme::LAST } else { theme::BRANCH }),
                    Span::styled(format!(" {marker} "), theme.style(StyleKey::Guide)),
                    Span::styled(glyph, style),
                    Span::raw(format!(" {name}")),
                ]);
                if let State::Missing = project.row.state {
                    line.spans.extend([
                        Span::raw("  "),
                        Span::styled("missing", theme.style(StyleKey::Lost)),
                    ]);
                }
                let runtime = project
                    .agents
                    .iter()
                    .flat_map(|agents| &agents.windows)
                    .find(|window| matches!(window.role, Role::Lead))
                    .and_then(|lead| lead.segment.age.as_deref());
                if let Some(runtime) = runtime {
                    let padding = usize::from(width).saturating_sub(line.width() + runtime.len());
                    line.spans.push(Span::styled(
                        format!("{}{runtime}", " ".repeat(padding)),
                        theme.style(StyleKey::Muted),
                    ));
                }
                line
            }
            Item::Folder(project, role) => {
                let branch = if project.roles().next_back() == Some(role) {
                    theme::LAST
                } else {
                    theme::BRANCH
                };
                let name = project.row.entry.name.as_str().to_owned();
                let marker = if self.collapsed_folders.contains(&(name, role)) {
                    theme::COLLAPSED
                } else {
                    theme::EXPANDED
                };
                let plural = match role {
                    WorkerRole::Worker => "workers",
                    WorkerRole::Reviewer => "reviewers",
                    WorkerRole::Security => "security",
                    WorkerRole::Research => "research",
                    WorkerRole::Design => "designers",
                };
                let count = project.members(role).count();
                Line::from(vec![
                    guide(stem),
                    Span::raw("  "),
                    guide(branch),
                    Span::raw(" "),
                    guide(marker),
                    Span::raw(format!(" {plural} ")),
                    Span::styled(count.to_string(), theme.style(StyleKey::Muted)),
                ])
            }
            Item::Member(project, role, member) => {
                let folder_stem = if project.roles().next_back() == Some(role) {
                    " "
                } else {
                    theme::STEM
                };
                let branch = if project.members(role).last().map(Item::key) == Some(item.key()) {
                    theme::LAST
                } else {
                    theme::BRANCH
                };
                let mut line = Line::from(vec![
                    guide(stem),
                    Span::raw("  "),
                    guide(folder_stem),
                    Span::raw("  "),
                    guide(branch),
                    Span::raw(" "),
                ]);
                match member {
                    Member::Window(window) => {
                        let state = match window.segment.state {
                            Some(ThemeState::Running) => ThemeState::Running,
                            Some(ThemeState::Starting) => ThemeState::Starting,
                            Some(ThemeState::Idle | ThemeState::Waiting) | None => ThemeState::Idle,
                        };
                        let (glyph, style) = theme.state(state);
                        line.spans.extend([
                            Span::styled(glyph, style),
                            Span::raw(format!(" {}", window.segment.label)),
                        ]);
                        if let Some(tokens) = window
                            .segment
                            .tokens
                            .filter(|_| window.segment.state.is_some())
                        {
                            let tokens = crate::projects::rows::abbreviate(tokens);
                            let padding = usize::from(width)
                                .saturating_sub(line.width() + tokens.len())
                                .max(1);
                            line.spans.push(Span::styled(
                                format!("{}{tokens}", " ".repeat(padding)),
                                theme.style(StyleKey::Muted),
                            ));
                        }
                    }
                    Member::Lost(id) => line.spans.extend([
                        Span::raw(format!("{id} ")),
                        Span::styled("window lost", theme.style(StyleKey::Lost)),
                    ]),
                }
                line
            }
        }
    }
}

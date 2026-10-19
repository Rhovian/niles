use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Row, Table, TableState},
};

use super::{
    Role,
    columns::{Column, Columns},
    form::{Form, Screen},
};
use crate::{
    theme::{StyleKey, Theme},
    workspace_manifest::clamp,
};

pub(super) fn form(frame: &mut Frame, form: &Form, theme: &Theme, root: &str) {
    let title = match &form.screen {
        Screen::Editing { role, .. } => format!("NILES / ROLES / {}", role.name()),
        Screen::Roles | Screen::Review => "NILES / ROLES".to_owned(),
    };
    let (mut body, footer) = chrome(frame, theme, &format!("{title} — {}", clamp(root, 120)));
    if let Some(reason) = &form.save_reason {
        let [notice, rest] =
            Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(body);
        frame.render_widget(
            Paragraph::new(clamp(reason, 400))
                .wrap(ratatui::widgets::Wrap { trim: true })
                .style(theme.style(StyleKey::Accent)),
            notice,
        );
        body = rest;
    }
    let mut warning = None;
    let hints = match &form.screen {
        Screen::Roles => {
            roles(frame, body, form, theme, false);
            if form.can_save() {
                "↑↓ role · ↵ edit standard primary · s save"
            } else {
                "↑↓ role · ↵ edit standard primary"
            }
        }
        Screen::Review => {
            roles(frame, body, form, theme, true);
            "↵ save · esc back"
        }
        Screen::Editing { columns, .. } => {
            columns.draw(frame, body, theme);
            warning = Some("Editing the standard primary; other tiers and secondaries are kept.");
            COLUMN_KEYS
        }
    };
    let muted = theme.style(StyleKey::Muted);
    let mut hints = Line::styled(hints, muted);
    if !form.can_save() {
        hints.spans.extend([
            Span::styled(" · ", muted),
            Span::styled("c", theme.style(StyleKey::Running)),
            Span::styled(" continue", muted),
        ]);
    }
    hints.spans.extend([
        Span::styled(" · ", muted),
        Span::styled("q", theme.style(StyleKey::Lost)),
        Span::styled(" quit", muted),
    ]);
    footer_text(frame, footer, theme, warning, hints);
}

pub(super) fn single(frame: &mut Frame, columns: &Columns, role: Role, theme: &Theme) {
    let (body, footer) = chrome(frame, theme, &format!("CONFIG / {}", role.name()));
    columns.draw(frame, body, theme);
    let hints = Line::styled(COLUMN_KEYS, theme.style(StyleKey::Muted));
    footer_text(frame, footer, theme, None, hints);
}

const COLUMN_KEYS: &str = "↑↓ move · →/↵ pick · ←/esc back";

fn chrome(frame: &mut Frame, theme: &Theme, title: &str) -> (Rect, Rect) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(3),
    ])
    .areas(frame.area());
    frame.render_widget(
        Paragraph::new(title).style(theme.style(StyleKey::Heading)),
        header,
    );
    (body, footer)
}

fn footer_text(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    warning: Option<&str>,
    hints: Line<'_>,
) {
    let mut lines = Vec::new();
    if let Some(warning) = warning {
        lines.push(Line::styled(warning, theme.style(StyleKey::Accent)));
    }
    lines.push(hints);
    let rule = Block::new()
        .borders(Borders::TOP)
        .border_style(theme.style(StyleKey::Guide));
    frame.render_widget(Paragraph::new(Text::from(lines)).block(rule), area);
}

fn roles(frame: &mut Frame, area: Rect, form: &Form, theme: &Theme, review: bool) {
    let mut rows = Vec::new();
    for role in Role::ALL {
        let changed = role.changed(&form.original, &form.draft);
        let value = clamp(&role.value(&form.draft), 120);
        let status = if changed { "changed" } else { "kept" };
        let row = Row::new([role.name().to_owned(), value, status.to_owned()]);
        rows.push(if changed {
            row.style(theme.style(StyleKey::Accent))
        } else {
            row
        });
    }
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Fill(1),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new([if review { "REVIEW" } else { "ROLE" }, "AGENT", ""])
            .style(theme.style(StyleKey::Heading)),
    )
    .row_highlight_style(theme.style(StyleKey::Selection));
    let mut state = TableState::default().with_selected((!review).then_some(form.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

impl Columns {
    pub(super) fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let [families, models, efforts] = Layout::horizontal([
            Constraint::Percentage(25),
            Constraint::Percentage(50),
            Constraint::Percentage(25),
        ])
        .areas(area);
        let mut render_list = |area, title, items, selected, column| {
            list(
                frame,
                area,
                title,
                items,
                selected,
                self.column == column,
                theme,
            );
        };
        let items = self
            .families
            .iter()
            .map(|family| {
                if family.installed {
                    ListItem::new(clamp(&family.name, 60))
                } else {
                    ListItem::new(format!("{} (not installed)", clamp(&family.name, 60)))
                        .style(theme.style(StyleKey::Muted))
                }
            })
            .collect();
        render_list(families, "FAMILY", items, self.family, Column::Family);
        let family = &self.families[self.family];
        if family.models.is_empty() {
            return;
        }
        let items = family
            .models
            .iter()
            .map(|model| {
                ListItem::new(Line::from(vec![
                    Span::raw(model.name.as_str()),
                    Span::styled(
                        format!("  {}", model.efforts.join(" ")),
                        theme.style(StyleKey::Muted),
                    ),
                ]))
            })
            .collect();
        render_list(models, "MODEL", items, self.model, Column::Model);
        let model = &family.models[self.model];
        if model.efforts.is_empty() {
            return;
        }
        let items = model
            .efforts
            .iter()
            .map(|effort| ListItem::new(effort.as_str()))
            .collect();
        render_list(efforts, "EFFORT", items, self.effort, Column::Effort);
    }
}

fn list(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: Vec<ListItem<'_>>,
    selected: usize,
    active: bool,
    theme: &Theme,
) {
    let heading = if active {
        StyleKey::Accent
    } else {
        StyleKey::Heading
    };
    let block = Block::new().title(Line::styled(title, theme.style(heading)));
    let list = List::new(items)
        .block(block)
        .highlight_style(theme.style(if active {
            StyleKey::Selection
        } else {
            StyleKey::Heading
        }));
    frame.render_stateful_widget(
        list,
        area,
        &mut ListState::default().with_selected(Some(selected)),
    );
}

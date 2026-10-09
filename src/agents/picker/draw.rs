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
        Screen::Roles | Screen::Presets { .. } | Screen::Review => "NILES / ROLES".to_owned(),
    };
    let (body, footer) = chrome(frame, theme, &format!("{title} — {}", clamp(root, 120)));
    let mut warning = None;
    let hints = match &form.screen {
        Screen::Roles => {
            roles(frame, body, form, theme, false);
            "↑↓ role · ↵ edit · p preset · s save"
        }
        Screen::Review => {
            roles(frame, body, form, theme, true);
            "↵ save · esc back"
        }
        Screen::Presets { selected } => {
            let items = form
                .presets
                .iter()
                .map(|preset| match &preset.values {
                    Ok(_) => ListItem::new(preset.name.as_str()),
                    Err(reason) => {
                        ListItem::new(format!("{} — {}", preset.name, clamp(reason, 120)))
                            .style(theme.style(StyleKey::Muted))
                    }
                })
                .collect();
            list(frame, body, "PRESETS", items, *selected, true, theme);
            "↑↓ preset · ↵ apply · esc back"
        }
        Screen::Editing { role, columns } => {
            columns.draw(frame, body, theme);
            if role.groups(&form.draft).is_some() {
                warning =
                    Some("Replacing this role drops its hand-edited groups; esc back keeps them.");
            }
            COLUMN_KEYS
        }
    };
    let mut hints = key_hints(theme, hints);
    let muted = theme.style(StyleKey::Muted);
    hints.spans.extend([
        Span::styled(" · ", muted),
        Span::styled("c", theme.style(StyleKey::Running)),
        Span::styled(" continue · ", muted),
        Span::styled("q", theme.style(StyleKey::Lost)),
        Span::styled(" quit", muted),
    ]);
    footer_text(frame, footer, theme, warning, hints);
}

pub(super) fn single(frame: &mut Frame, columns: &Columns, role: Role, theme: &Theme) {
    let (body, footer) = chrome(frame, theme, &format!("CONFIG / {}", role.name()));
    columns.draw(frame, body, theme);
    footer_text(frame, footer, theme, None, key_hints(theme, COLUMN_KEYS));
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

fn key_hints<'a>(theme: &Theme, hints: &'a str) -> Line<'a> {
    Line::styled(hints, theme.style(StyleKey::Muted))
}

fn roles(frame: &mut Frame, area: Rect, form: &Form, theme: &Theme, review: bool) {
    let mut rows = Vec::new();
    if !review {
        rows.push(Row::new([
            "PRESET".to_owned(),
            "choose a preset".to_owned(),
            String::new(),
        ]));
    }
    for role in Role::ALL {
        let changed = role.changed(&form.original, &form.draft);
        let value = match role.groups(&form.draft) {
            Some(count) => format!("{count} groups (hand-edited)"),
            None => clamp(&role.value(&form.draft), 120),
        };
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
        list(
            frame,
            families,
            "FAMILY",
            items,
            self.family,
            self.column == Column::Family,
            theme,
        );
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
        list(
            frame,
            models,
            "MODEL",
            items,
            self.model,
            self.column == Column::Model,
            theme,
        );
        let model = &family.models[self.model];
        if model.efforts.is_empty() {
            return;
        }
        let items = model
            .efforts
            .iter()
            .map(|effort| ListItem::new(effort.as_str()))
            .chain(std::iter::once(ListItem::new("cli default")))
            .collect();
        list(
            frame,
            efforts,
            "EFFORT",
            items,
            self.effort,
            self.column == Column::Effort,
            theme,
        );
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

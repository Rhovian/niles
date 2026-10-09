use chrono::{DateTime, Local, Utc};
use ratatui::style::Style;

use super::{
    Range,
    aggregate::{ContextRow, Dashboard},
    collect::EventKind,
};
use crate::{
    projects::{
        panels::{Cell, table},
        rows::abbreviate,
    },
    theme::{StyleKey, StyleRender, Theme},
};

/// Section pairs sit side by side from this pane width, and stack below it.
const SIDE_BY_SIDE: usize = 120;
const GAP: usize = 4;
const BAR_WIDTH: u64 = 12;
const CONTEXT_WARN_PERCENT: u64 = 80;
const CONTEXT_ROWS: usize = 5;
const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// A section's lines, each with its width on screen.
struct Block(Vec<(String, usize)>);

impl Block {
    fn new(title: &str, theme: &Theme) -> Self {
        Self(vec![(
            theme.style(StyleKey::Heading).paint(title),
            title.chars().count(),
        )])
    }

    fn line(&mut self, text: &str, style: Style) {
        self.0.push((style.paint(text), text.chars().count()));
    }

    fn table(&mut self, rows: &[Vec<Cell>]) {
        let (lines, width) = table(rows);
        self.0.extend(lines.into_iter().map(|line| (line, width)));
    }

    fn width(&self) -> usize {
        self.0.iter().map(|(_, width)| *width).fold(0, usize::max)
    }
}

pub(super) fn dashboard(dashboard: &Dashboard<'_>, theme: &Theme, width: usize) -> String {
    let window = &dashboard.window;
    let start = window.start.with_timezone(&Local);
    let mut lines = vec![
        format!(
            "{}   sessions {} · spawns {} · wakes {}",
            theme.style(StyleKey::Heading).paint(&format!(
                "since {} {}",
                start.format(time_format(window.range)),
                start.format("%Z")
            )),
            dashboard.sessions,
            dashboard.spawns,
            dashboard.wakes
        ),
        String::new(),
    ];
    lines.extend(pair(
        projects(dashboard, theme),
        models(dashboard, theme),
        width,
    ));
    lines.push(String::new());
    lines.extend(chart(dashboard, theme).0.into_iter().map(|(line, _)| line));
    lines.push(String::new());
    lines.extend(pair(
        context(dashboard, theme),
        events(dashboard, theme),
        width,
    ));
    lines.join("\n") + "\n"
}

fn pair(left: Block, right: Block, width: usize) -> Vec<String> {
    if width < SIDE_BY_SIDE {
        let mut lines = left.0.into_iter().map(|(line, _)| line).collect::<Vec<_>>();
        lines.push(String::new());
        lines.extend(right.0.into_iter().map(|(line, _)| line));
        return lines;
    }
    let column = left.width() + GAP;
    let mut left = left.0.into_iter();
    let mut right = right.0.into_iter();
    let mut lines = Vec::new();
    loop {
        let line = match (left.next(), right.next()) {
            (None, None) => return lines,
            (Some((line, _)), None) => line,
            (left, Some((right, _))) => {
                let (line, width) = left.unwrap_or_else(|| (String::new(), 0));
                format!("{line}{}{right}", " ".repeat(column - width))
            }
        };
        lines.push(line);
    }
}

/// `part` of `whole`, scaled to `scale`; nothing of nothing.
fn share(part: u64, whole: u64, scale: u64) -> u64 {
    match whole {
        0 => 0,
        _ => part * scale / whole,
    }
}

fn bar(filled: u64) -> String {
    let filled = filled.min(BAR_WIDTH) as usize;
    "█".repeat(filled) + &" ".repeat(BAR_WIDTH as usize - filled)
}

fn projects(dashboard: &Dashboard<'_>, theme: &Theme) -> Block {
    let total = dashboard.projects.iter().map(|(_, tokens)| tokens).sum();
    let mut rows = dashboard
        .projects
        .iter()
        .map(|(name, tokens)| {
            vec![
                Cell::left(*name, theme.style(StyleKey::Accent)),
                Cell::left(
                    bar(share(*tokens, total, BAR_WIDTH)),
                    theme.style(StyleKey::Bar),
                ),
                Cell::right(abbreviate(*tokens), theme.style(StyleKey::Heading)),
                Cell::right(
                    format!("{}%", share(*tokens, total, 100)),
                    theme.style(StyleKey::Muted),
                ),
            ]
        })
        .collect::<Vec<_>>();
    rows.push(vec![
        Cell::left("total", theme.style(StyleKey::Heading)),
        Cell::left("", Style::new()),
        Cell::right(abbreviate(total), theme.style(StyleKey::Heading)),
        Cell::left("", Style::new()),
    ]);
    let mut block = Block::new("TOKENS BY PROJECT", theme);
    block.table(&rows);
    block
}

fn models(dashboard: &Dashboard<'_>, theme: &Theme) -> Block {
    let mut block = Block::new("BY MODEL", theme);
    if dashboard.models.is_empty() {
        block.line("no sessions", theme.style(StyleKey::Muted));
        return block;
    }
    let rows = dashboard
        .models
        .iter()
        .map(|row| {
            let roles = row
                .roles
                .iter()
                .map(|(role, count)| match (*role, count) {
                    (_, 1) | ("security" | "research", _) => format!("{count} {role}"),
                    _ => format!("{count} {role}s"),
                })
                .collect::<Vec<_>>()
                .join(" · ");
            vec![
                Cell::left(row.agent, Style::new()),
                Cell::left(roles, theme.style(StyleKey::Muted)),
                Cell::right(abbreviate(row.tokens), theme.style(StyleKey::Heading)),
            ]
        })
        .collect::<Vec<_>>();
    block.table(&rows);
    block
}

fn chart(dashboard: &Dashboard<'_>, theme: &Theme) -> Block {
    let mut block = Block::new("TOKENS", theme);
    let window = &dashboard.window;
    let peak = dashboard.bins.iter().copied().max();
    let Some((index, peak)) = dashboard
        .bins
        .iter()
        .position(|tokens| Some(*tokens) == peak)
        .zip(peak)
        .filter(|(_, peak)| *peak > 0)
    else {
        block.line("no tokens", theme.style(StyleKey::Muted));
        return block;
    };
    let strip = dashboard
        .bins
        .iter()
        .map(|tokens| match share(*tokens, peak, 8) {
            0 if *tokens == 0 => ' ',
            level => LEVELS[(level.max(1) - 1) as usize],
        })
        .collect::<String>();
    block.line(&strip, theme.style(StyleKey::Bar));
    let (stride, format) = match window.range {
        Range::Today => (16, "%H:%M"),
        Range::Week => (12, "%a"),
        Range::Month => (21, "%-d %b"),
    };
    let mut axis = String::new();
    for bin in (0..window.count).step_by(stride) {
        let label = local(window.bin_start(bin)).format(format).to_string();
        let column = bin.cast_unsigned() as usize;
        if axis.chars().count() <= column {
            axis += &" ".repeat(column - axis.chars().count());
            axis += &label;
        }
    }
    block.line(&axis, theme.style(StyleKey::Muted));
    let at = local(window.bin_start(index as i32)).format(time_format(window.range));
    block.line(
        &format!("peak {} at {at}", abbreviate(peak)),
        theme.style(StyleKey::Muted),
    );
    block
}

fn context(dashboard: &Dashboard<'_>, theme: &Theme) -> Block {
    let mut block = Block::new("CONTEXT", theme);
    if dashboard.context.is_empty() {
        block.line("no live sessions", theme.style(StyleKey::Muted));
        return block;
    }
    let rows = dashboard
        .context
        .iter()
        .take(CONTEXT_ROWS)
        .map(|row| context_row(row, theme))
        .collect::<Vec<_>>();
    block.table(&rows);
    let more = dashboard.context.len().saturating_sub(CONTEXT_ROWS);
    if more > 0 {
        block.line(&format!("+ {more} more"), theme.style(StyleKey::Muted));
    }
    block
}

fn context_row(row: &ContextRow<'_>, theme: &Theme) -> Vec<Cell> {
    let warn = row
        .percent
        .is_some_and(|percent| percent >= CONTEXT_WARN_PERCENT);
    let style = |key| theme.style(if warn { StyleKey::Waiting } else { key });
    let session = row.session;
    vec![
        Cell::left(row.project, style(StyleKey::Muted)),
        Cell::left(
            if session.role == "lead" {
                "lead"
            } else {
                &session.id
            },
            style(StyleKey::Accent),
        ),
        Cell::left(&session.agent, style(StyleKey::Muted)),
        match row.percent {
            Some(percent) => Cell::left(
                bar(share(percent, 100, BAR_WIDTH)),
                theme.style(StyleKey::Bar),
            ),
            None => Cell::left("", Style::new()),
        },
        Cell::right(abbreviate(row.prompt_tokens), style(StyleKey::Heading)),
        Cell::right(
            row.percent
                .map_or_else(String::new, |percent| format!("{percent}%")),
            style(StyleKey::Muted),
        ),
    ]
}

fn events(dashboard: &Dashboard<'_>, theme: &Theme) -> Block {
    let mut block = Block::new("EVENTS", theme);
    if dashboard.events.is_empty() {
        block.line("no events", theme.style(StyleKey::Muted));
        return block;
    }
    let rows = dashboard
        .events
        .iter()
        .map(|event| {
            vec![
                Cell::left(
                    local(event.at)
                        .format(time_format(dashboard.window.range))
                        .to_string(),
                    theme.style(StyleKey::Muted),
                ),
                Cell::left(event.project, theme.style(StyleKey::Muted)),
                Cell::left(event.id, theme.style(StyleKey::Accent)),
                Cell::left(
                    match event.kind {
                        EventKind::Spawned => "spawned".to_owned(),
                        EventKind::Reported(kind) => format!("reported ({kind})"),
                        EventKind::Closed => "closed".to_owned(),
                    },
                    Style::new(),
                ),
            ]
        })
        .collect::<Vec<_>>();
    block.table(&rows);
    block
}

fn local(at: DateTime<Utc>) -> DateTime<Local> {
    at.with_timezone(&Local)
}

/// How a time in the window is shown: within a day, the hour is enough.
fn time_format(range: Range) -> &'static str {
    match range {
        Range::Today => "%H:%M",
        Range::Week => "%a %H:%M",
        Range::Month => "%-d %b %H:%M",
    }
}

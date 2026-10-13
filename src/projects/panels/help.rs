use anyhow::Result;
use ratatui::{
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout},
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};

use crate::{
    theme::{State, StyleKey, Theme},
    tmux,
};

const WORDMARK: &str = include_str!("help/wordmark.txt");

const FIRST_RUN: &str = "niles

Coordinates coding agents from different model families in tmux.

Get started
  r    register a project directory, in the explorer on the left
  ↵    start its lead and show it here

Key bindings for the home view: https://github.com/Rhovian/niles/blob/main/docs/setup.md
";

const KEYS: &str = "Choose a project on the left.

  ↵      open                 → ←    expand / collapse
  [ ]    previous / next project
  ; '    previous / next window
  r      register a project
  c      close a worker or project
  ?      this help            esc    back

  {running} running   {waiting} waiting

With the bindings in docs/setup.md, M-[ M-] M-; M-' do the same from any pane.
";

pub(super) fn run(first_run: bool, theme: &Theme) -> Result<()> {
    let text = help_text(first_run, theme);
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|frame| draw(frame, &text, theme))?;
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && key.code == KeyCode::Esc
            {
                tmux::focus_explorer()?;
            }
        }
    })();
    ratatui::restore();
    result
}

fn help_text(first_run: bool, theme: &Theme) -> Text<'static> {
    let source = if first_run { FIRST_RUN } else { KEYS };
    Text::from(
        source
            .lines()
            .map(|line| match line {
                "niles" | "Get started" => Line::styled(line, theme.style(StyleKey::Heading)),
                "  {running} running   {waiting} waiting" => {
                    let (waiting, style) = theme.state(State::Waiting);
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(theme.spinner(0), theme.state(State::Running).1),
                        Span::raw(" running   "),
                        Span::styled(waiting, style),
                        Span::raw(" waiting"),
                    ])
                }
                _ => Line::raw(line),
            })
            .collect::<Vec<_>>(),
    )
}

fn draw(frame: &mut Frame, text: &Text<'_>, theme: &Theme) {
    let area = frame.area();
    let help = Paragraph::new(text.clone()).wrap(Wrap { trim: false });
    let available = usize::from(area.height).saturating_sub(help.line_count(area.width));
    let art = splash(area.width, available);
    let height = art.map_or(0, |art| art.lines().count() + 1);
    // Every selected splash fits the frame, whose height is a u16.
    debug_assert!(height <= usize::from(area.height));
    let height = height as u16;
    let [banner, body] =
        Layout::vertical([Constraint::Length(height), Constraint::Fill(1)]).areas(area);
    if let Some(art) = art {
        frame.render_widget(
            Paragraph::new(art).style(theme.style(StyleKey::Heading)),
            banner,
        );
    }
    frame.render_widget(help, body);
}

// ratatui-widgets 0.3.2 Paragraph::line_count accounts for wrapping, but neither it nor
// Layout selects among alternative artwork: https://docs.rs/ratatui-widgets/0.3.2/ratatui_widgets/paragraph/struct.Paragraph.html#method.line_count
fn splash(width: u16, available_rows: usize) -> Option<&'static str> {
    [
        concat!(
            include_str!("help/butler.txt"),
            "\n",
            include_str!("help/wordmark.txt")
        ),
        WORDMARK,
    ]
    .into_iter()
    .find(|art| {
        art.lines().count() < available_rows
            && art
                .lines()
                .all(|line| line.chars().count() <= usize::from(width))
    })
}

#[cfg(test)]
mod tests;

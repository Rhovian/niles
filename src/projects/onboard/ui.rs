use crate::selection::Selection;
use crate::theme::{StyleKey, Theme};
use anyhow::{Context, Result};
use camino::Utf8Path;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{List, ListState, Paragraph, Wrap};

pub(super) fn screen<T>(
    mut run: impl FnMut(&mut ratatui::DefaultTerminal) -> Result<T>,
) -> Result<T> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}
pub(super) fn key() -> Result<KeyCode> {
    loop {
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            return Ok(key.code);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Next,
    Back,
    Quit,
}

// crossterm 0.29.0 event::read supplies key events, not selection or checklist state.
pub(super) struct Options {
    rows: Selection<((String, String), bool)>,
    checklist: bool,
}
impl Options {
    pub fn new(rows: Vec<(String, String)>, checklist: bool) -> Result<Self> {
        Ok(Self {
            rows: Selection::new(rows.into_iter().map(|row| (row, false)))
                .context("a picker needs at least one option")?,
            checklist,
        })
    }
    #[expect(clippy::wildcard_enum_match_arm, reason = "ignore unbound picker keys")]
    pub fn key(&mut self, key: KeyCode) -> Option<Action> {
        match key {
            KeyCode::Esc | KeyCode::Left => Some(Action::Back),
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Enter => {
                if !self.checklist {
                    self.rows.current_mut().1 = true;
                }
                Some(Action::Next)
            }
            KeyCode::Up | KeyCode::Down => {
                self.rows.move_focus(key == KeyCode::Down);
                None
            }
            KeyCode::Char(' ') if self.checklist => {
                let checked = &mut self.rows.current_mut().1;
                *checked = !*checked;
                None
            }
            _ => None,
        }
    }
    pub fn pick(&mut self, header: &str, question: &str) -> Result<Action> {
        let theme = Theme::load()?;
        let mut state = ListState::default();
        screen(|terminal| {
            loop {
                let rows = self
                    .rows
                    .iter()
                    .map(|((name, description), checked)| {
                        let mark = if *checked { "[x] " } else { "[ ] " };
                        format!(
                            "{}{name} — {description}",
                            if self.checklist { mark } else { "" }
                        )
                    })
                    .collect::<Vec<_>>();
                let hint = if self.checklist {
                    " · space toggle"
                } else {
                    ""
                };
                terminal.draw(|frame| {
                    let question = Paragraph::new(format!("{header}\n\n{question}"))
                        .wrap(Wrap { trim: false });
                    // ratatui-widgets 0.3.2 Paragraph::line_count returns usize; a header
                    // larger than the terminal is clipped before converting to coordinates.
                    let height = question
                        .line_count(frame.area().width)
                        .min(usize::from(frame.area().height));
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "height is bounded by the terminal u16 height"
                    )]
                    let height = height as u16;
                    let [heading, body, footer] = Layout::vertical([
                        Constraint::Length(height.saturating_add(1)),
                        Constraint::Fill(1),
                        Constraint::Length(2),
                    ])
                    .areas(frame.area());
                    frame.render_widget(question, heading);
                    state.select(Some(self.rows.position()));
                    frame.render_stateful_widget(
                        List::new(rows)
                            .highlight_symbol("› ")
                            .highlight_style(theme.style(StyleKey::Selection)),
                        body,
                        &mut state,
                    );
                    frame.render_widget(
                        Paragraph::new(format!(
                            "↑↓ move{hint} · Enter continue · ←/Esc back · q quit"
                        )),
                        footer,
                    );
                })?;
                if let Some(accepted) = self.key(key()?) {
                    return Ok(accepted);
                }
            }
        })
    }
    pub fn replace_rows(&mut self, rows: Vec<(String, String)>) -> Result<()> {
        let mut next = Selection::new(rows.into_iter().map(|row| {
            let checked = self
                .rows
                .iter()
                .any(|((name, _), checked)| *name == row.0 && *checked);
            (row, checked)
        }))
        .context("a picker needs at least one option")?;
        next.prefer(|(row, _)| row.0 == self.rows.current().0.0);
        self.rows = next;
        Ok(())
    }
    pub fn chosen(&self) -> &(String, String) {
        debug_assert!(!self.checklist);
        &self.rows.current().0
    }
    pub fn selected(&self) -> impl Iterator<Item = &str> {
        self.rows
            .iter()
            .filter(|(_, checked)| *checked)
            .map(|((name, _), _)| name.as_str())
    }
}

pub(super) fn confirm(options: &mut Options, header: &str, text: &str) -> Result<Action> {
    let action = options.pick(header, text)?;
    Ok(
        if action == Action::Next && options.chosen().0 == "Cancel" {
            Action::Quit
        } else {
            action
        },
    )
}

pub(super) fn register(root: &Utf8Path) -> Result<Option<super::registry::Entry>> {
    let mut prompt = super::register::prompt::Prompt::new();
    let mut error = String::new();
    screen(|terminal| {
        loop {
            terminal.draw(|frame| {
                frame.render_widget(
                    Paragraph::new(format!(
                        "Setting up Niles — register a project\n\n{}\n{error}\n\nEsc cancel",
                        prompt.text(root)
                    )),
                    frame.area(),
                )
            })?;
            let action = prompt.key(key()?, root);
            match action {
                Ok(super::register::prompt::Action::Pending) => error.clear(),
                Ok(super::register::prompt::Action::Cancel) => return Ok(None),
                Ok(super::register::prompt::Action::Register { path, name }) => {
                    match super::register::register(&name, &path) {
                        Ok(entry) => return Ok(Some(entry)),
                        Err(problem) => error = format!("{problem:#}"),
                    }
                }
                Err(problem) => error = format!("{problem:#}"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::super::defaults;
    use super::*;
    use crate::store;

    #[test]
    fn checklist_rebuild_keeps_other_checks_and_cursor() {
        use ratatui::crossterm::event::KeyCode;
        let rows: Vec<(String, String)> = store::parse_yaml(defaults::SKILLS).unwrap();
        let mut options = Options::new(rows.clone(), true).unwrap();
        options.key(KeyCode::Down);
        options.key(KeyCode::Char(' '));
        options
            .replace_rows(
                rows.into_iter()
                    .filter(|(name, _)| name != "lead")
                    .collect(),
            )
            .unwrap();
        assert_eq!(options.selected().collect::<Vec<_>>(), ["design"]);
        assert_eq!(options.rows.position(), 0);
        options.key(KeyCode::Char(' '));
        assert_eq!(options.selected().count(), 0);
    }
}

use anyhow::Result;
use camino::Utf8Path;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout},
    widgets::{List, ListState, Paragraph, Wrap},
};

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

// crossterm 0.29.0 event::read supplies key events, not selection or checklist state.
pub(super) struct Options {
    rows: Vec<(String, String)>,
    checked: Vec<bool>,
    cursor: usize,
    checklist: bool,
}
impl Options {
    pub fn new(rows: Vec<(String, String)>, checklist: bool) -> Self {
        debug_assert!(!rows.is_empty(), "a picker needs at least one option");
        Self {
            checked: vec![false; rows.len()],
            rows,
            cursor: 0,
            checklist,
        }
    }
    #[expect(clippy::wildcard_enum_match_arm, reason = "ignore unbound picker keys")]
    pub fn key(&mut self, key: KeyCode) -> Option<bool> {
        match key {
            KeyCode::Esc => Some(false),
            KeyCode::Enter => {
                if !self.checklist {
                    self.checked[self.cursor] = true;
                }
                Some(true)
            }
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                None
            }
            KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(self.rows.len() - 1);
                None
            }
            KeyCode::Char(' ') if self.checklist => {
                self.checked[self.cursor] = !self.checked[self.cursor];
                None
            }
            _ => None,
        }
    }
    pub fn pick(&mut self, header: &str, question: &str) -> Result<bool> {
        let theme = crate::theme::Theme::load()?;
        let mut state = ListState::default();
        screen(|terminal| {
            loop {
                let rows = self
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(i, (name, description))| {
                        let mark = if self.checked[i] { "[x] " } else { "[ ] " };
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
                    state.select(Some(self.cursor));
                    frame.render_stateful_widget(
                        List::new(rows)
                            .highlight_symbol("› ")
                            .highlight_style(theme.style(crate::theme::StyleKey::Selection)),
                        body,
                        &mut state,
                    );
                    frame.render_widget(
                        Paragraph::new(format!("↑↓ move{hint} · Enter continue · Esc cancel")),
                        footer,
                    );
                })?;
                if let Some(accepted) = self.key(key()?) {
                    return Ok(accepted);
                }
            }
        })
    }
    pub fn chosen(&self) -> &(String, String) {
        debug_assert!(!self.checklist);
        &self.rows[self.cursor]
    }
    pub fn selected(&self) -> impl Iterator<Item = &str> {
        self.rows
            .iter()
            .zip(&self.checked)
            .filter(|(_, checked)| **checked)
            .map(|((name, _), _)| name.as_str())
    }
}
pub(super) fn confirm(header: &str, text: &str) -> Result<bool> {
    let mut options = Options::new(
        vec![
            ("Continue".into(), "Accept and continue".into()),
            (
                "Cancel".into(),
                "Leave setup without writing a manifest".into(),
            ),
        ],
        false,
    );
    Ok(options.pick(header, text)? && options.chosen().0 == "Continue")
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

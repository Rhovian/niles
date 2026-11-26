use super::super::lifecycle;
use super::tree::{Item, Member};
use super::{
    Closing, Explorer, LEAD_WINDOW, Mode, State, StyleKey, Theme, registry, shown, start_project,
    tmux,
};
use camino::Utf8PathBuf;
use ratatui::Frame;
use ratatui::layout::Constraint::{Fill, Length};
use ratatui::layout::{Direction, Layout, Position, Rect};
use ratatui::widgets::{Block, Clear, List, ListState, Padding};
use std::{cmp::max_by_key, mem::replace};

const DEREGISTER: &str = "deregister";

#[derive(PartialEq, Eq)]
pub(super) enum Action {
    View(registry::Entry, String),
    Open(registry::Entry),
    Close(Closing),
    CloseWorker(String, Utf8PathBuf),
    Deregister(registry::Entry),
}

impl Action {
    fn options(item: Item<'_>) -> Option<Vec<Self>> {
        let project = item.project()?;
        let entry = &project.row.entry;
        let view = |window: &str| Self::View(entry.clone(), window.to_owned());
        let deregister = || Self::Deregister(entry.clone());
        if let Item::Member(_, _, member) = item {
            let (id, window) = match member {
                Member::Lost(id) => (id, None),
                Member::Window(window) => (window.kind.worker_id()?, Some(window.name.as_str())),
            };
            let close = Self::CloseWorker(id.to_owned(), entry.path.clone());
            return Some(window.into_iter().map(view).chain([close]).collect());
        }
        if !matches!(item, Item::Project(_)) {
            return None;
        }
        Some(match project.row.state {
            State::Missing => vec![deregister()],
            State::NotRunning => vec![Self::Open(entry.clone()), deregister()],
            State::Running | State::Starting | State::Waiting(_) => {
                let close = Self::Close(Closing::project(project)?);
                vec![view(LEAD_WINDOW), close, deregister()]
            }
        })
    }

    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::View(..) => "view",
            Self::Open(_) => "open",
            Self::Close(_) | Self::CloseWorker(..) => "close",
            Self::Deregister(_) => DEREGISTER,
        }
    }
}

pub(super) enum MenuInput {
    Enter,
    Click(Position),
}

pub(super) struct Menu {
    actions: Vec<Action>,
    origin: Option<Position>,
    area: Rect,
    state: ListState,
}

impl Menu {
    pub(super) fn matches(&self, item: Item<'_>) -> bool {
        Action::options(item).is_some_and(|actions| self.actions == actions)
    }

    pub(super) fn move_highlight(&mut self, forward: bool) {
        let last = self.actions.len().saturating_sub(1);
        let current = self.state.selected();
        self.state.select(Some(if forward {
            current.map_or(0, |index| index.saturating_add(1).min(last))
        } else {
            current.map_or(last, |index| index.saturating_sub(1))
        }));
    }

    pub(super) fn highlighted(&self) -> Option<&Action> {
        self.actions.get(self.state.selected()?)
    }

    pub(super) fn action_at(&self, at: Position) -> Option<&Action> {
        if !self.area.contains(at) {
            return None;
        }
        let row = usize::from(at.y.saturating_sub(self.area.y));
        self.actions.get(self.state.offset().saturating_add(row))
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the row is bounded by list height and menu labels and action counts are small"
    )]
    pub(super) fn draw(&mut self, frame: &mut Frame, list: Rect, row: usize, theme: &Theme) {
        let row = row.min(usize::from(list.height)) as u16;
        let x = self.origin.map_or(list.right(), |at| at.x);
        let y = self.origin.map_or(list.y.saturating_add(row), |at| at.y);
        let width = DEREGISTER.len().saturating_add(2) as u16;
        let popup = Rect::new(x, y, width, self.actions.len() as u16);
        // In a one-row list, place the menu beside the opening cell to keep the footer visible.
        let (axis, offset) = if list.height == 1 {
            (Direction::Horizontal, x.saturating_sub(list.x))
        } else {
            (Direction::Vertical, y.saturating_sub(list.y))
        };
        let [before, _, after] =
            Layout::new(axis, [Length(offset), Length(1), Fill(1)]).areas(list);
        self.area = max_by_key(popup.clamp(before), popup.clamp(after), |r| r.area());
        let widget = List::new(self.actions.iter().map(Action::label))
            .block(Block::new().padding(Padding::horizontal(1)))
            .style(theme.style(StyleKey::Bar))
            .highlight_style(theme.style(StyleKey::Selection));
        frame.render_widget(Clear, self.area);
        frame.render_stateful_widget(widget, self.area, &mut self.state);
    }
}

impl Explorer {
    pub(super) fn activate(&mut self, origin: Option<Position>) {
        if let Some(actions) = self.tree.selected().and_then(Action::options) {
            self.mode = Mode::Menu(Menu {
                actions,
                origin,
                area: Rect::default(),
                state: ListState::default(),
            });
        } else {
            self.footer = shown(self.open());
        }
    }

    pub(super) fn pick_menu(&mut self, input: MenuInput) -> bool {
        let mode = replace(&mut self.mode, Mode::Browse);
        if let Mode::Menu(menu) = mode {
            let action = match input {
                MenuInput::Enter => menu.highlighted(),
                MenuInput::Click(at) => menu.action_at(at),
            };
            match action {
                Some(action) => self.perform_menu_action(action),
                None if matches!(input, MenuInput::Enter) => self.mode = Mode::Menu(menu),
                None => {}
            }
            return true;
        }
        self.mode = mode;
        false
    }

    fn perform_menu_action(&mut self, action: &Action) {
        let view = |target| tmux::show_in_view(&self.pane, &target).map(|()| None);
        let outcome = match action {
            Action::Close(closing) => {
                self.tree.cycle_projects(1);
                self.mode = Mode::Close(closing.clone());
                return;
            }
            Action::CloseWorker(id, path) => lifecycle::niles_in(path, &["close", id]).map(Some),
            Action::View(entry, window) => entry.window(window).and_then(view),
            Action::Open(entry) => start_project(entry, &self.theme).and_then(view),
            Action::Deregister(entry) => registry::deregister(entry)
                .and_then(|()| tmux::close_view(&self.pane))
                .map(|()| Some(format!("deregistered {}", entry.name.as_str()))),
        };
        self.footer = shown(outcome);
        if !matches!(action, Action::View(..)) {
            self.refresh();
        }
    }
}

#[cfg(test)]
impl Menu {
    pub(super) fn labels(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.actions.iter().map(Action::label)
    }
}

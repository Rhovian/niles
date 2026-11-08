use super::{Explorer, Mode, State, tree::Item};
use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind},
    layout::Position,
};

const PROJECT_DISCLOSURE_COLUMN: u16 = 2;

impl Explorer {
    pub(super) fn event(&mut self, event: Event) -> anyhow::Result<()> {
        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
        {
            self.key(key.code)?;
        } else if let Event::Mouse(mouse) = event
            && matches!(self.mode, Mode::Browse)
            && self
                .list_area
                .contains(Position::new(mouse.column, mouse.row))
        {
            if mouse.kind == MouseEventKind::ScrollUp {
                self.tree.up();
            } else if mouse.kind == MouseEventKind::ScrollDown {
                self.tree.down();
            } else if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                // ratatui-widgets 0.3.2 list/state.rs exposes the first visible item,
                // but no hit testing. Each explorer label occupies exactly one line.
                let row = self.list_state.offset() + usize::from(mouse.row - self.list_area.y);
                if self.tree.select_row(row) {
                    self.footer = None;
                    match self.tree.selected() {
                        Some(Item::Folder(..) | Item::ProjectScopes) => self.tree.toggle_branch(),
                        Some(Item::Project(_))
                            if mouse.column - self.list_area.x == PROJECT_DISCLOSURE_COLUMN =>
                        {
                            self.tree.toggle_branch();
                        }
                        Some(Item::Project(project))
                            if matches!(project.row.state, State::NotRunning) => {}
                        _ => self.key(KeyCode::Enter)?,
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use ratatui::{backend::TestBackend, crossterm::event::MouseEvent};

    fn left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: event::KeyModifiers::NONE,
        }
    }

    #[test]
    fn clicks_select_visible_rows_without_starting_projects() {
        for (offset, row, state) in [
            (0, 1, State::Missing),
            (1, 0, State::Missing),
            (0, 1, State::NotRunning),
        ] {
            let stopped = matches!(state, State::NotRunning);
            let mut explorer = super::super::tests::explorer();
            let project = tree::tests::project("sample", state, None);
            explorer.tree.replace(vec![project]);
            explorer.list_area = ratatui::layout::Rect::new(2, 2, 36, 6);
            *explorer.list_state.offset_mut() = offset;
            let click = Event::Mouse(left_click(37, 2 + row));
            explorer.event(click).unwrap();
            assert_eq!(explorer.tree.cursor(), 1);
            let clicked_footer = explorer.footer.clone();
            if stopped {
                assert!(clicked_footer.is_none());
            } else {
                explorer.key(KeyCode::Enter).unwrap();
                assert_eq!(explorer.footer, clicked_footer);
            }
        }
    }

    #[test]
    fn clicks_toggle_folders_and_project_disclosures() {
        let mut explorer = super::super::tests::explorer();
        explorer.tree = tree::tests::tree();
        explorer.list_area.height = 30;
        let branches: [(fn(Item<'_>) -> bool, u16); 3] = [
            (|item| matches!(item, Item::Project(_)), 2),
            (|item| matches!(item, Item::Folder(..)), 0),
            (|item| matches!(item, Item::ProjectScopes), 0),
        ];
        for (matches, column) in branches {
            let row = explorer.tree.items().into_iter().position(matches).unwrap();
            let click = Event::Mouse(left_click(column, u16::try_from(row).unwrap()));
            let count = explorer.tree.items().len();
            explorer.event(click.clone()).unwrap();
            assert_ne!(explorer.tree.items().len(), count);
            explorer.event(click).unwrap();
            assert_eq!(explorer.tree.items().len(), count);
            assert_eq!(explorer.tree.cursor(), row);
        }
    }

    #[test]
    fn clicks_ignore_blank_space_other_buttons_and_prompts() {
        let mut explorer = super::super::tests::explorer();
        explorer.list_area.height = 20;
        let click = left_click(0, 1);
        for (kind, column, row) in [
            (click.kind, 0, explorer.list_area.bottom()),
            (click.kind, 0, explorer.tree.items().len() as u16),
            (click.kind, explorer.list_area.right(), 1),
            (MouseEventKind::Down(MouseButton::Right), 0, 1),
            (MouseEventKind::Up(MouseButton::Left), 0, 1),
        ] {
            let mut mouse = left_click(column, row);
            mouse.kind = kind;
            explorer.event(Event::Mouse(mouse)).unwrap();
            assert_eq!(explorer.tree.cursor(), 0);
        }
        explorer.key(KeyCode::Char('r')).unwrap();
        explorer.event(Event::Mouse(click)).unwrap();
        assert_eq!(explorer.tree.cursor(), 0);
    }

    #[test]
    fn wheel_is_bounded_and_preserves_viewport_when_reversing() {
        let mut explorer = super::super::tests::explorer();
        let mut terminal = ratatui::Terminal::new(TestBackend::new(40, 6)).unwrap();
        let mut wheel = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: event::KeyModifiers::NONE,
        };
        for _ in 0..20 {
            explorer.event(Event::Mouse(wheel)).unwrap();
            terminal.draw(|frame| explorer.draw(frame)).unwrap();
        }
        assert_eq!(explorer.tree.cursor(), explorer.tree.items().len() - 1);
        let offset = explorer.list_state.offset();
        assert!(offset > 0);
        wheel.kind = MouseEventKind::ScrollUp;
        explorer.event(Event::Mouse(wheel)).unwrap();
        terminal.draw(|frame| explorer.draw(frame)).unwrap();
        assert_eq!(explorer.list_state.offset(), offset);
        for _ in 0..20 {
            explorer.event(Event::Mouse(wheel)).unwrap();
        }
        assert_eq!(explorer.tree.cursor(), 0);
    }
}

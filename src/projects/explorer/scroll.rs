use super::tree::Item;
use super::{Explorer, MenuInput, Mode};
use ratatui::crossterm::event::{Event, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::layout::Position;

impl Explorer {
    pub(super) fn event(&mut self, event: Event) {
        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
        {
            self.key(key.code);
        } else if let Event::Mouse(mouse) = event {
            let at = Position::new(mouse.column, mouse.row);
            let left_click = mouse.kind == MouseEventKind::Down(MouseButton::Left);
            if left_click && self.pick_menu(MenuInput::Click(at)) {
                return;
            }
            if matches!(self.mode, Mode::Browse) && self.list_area.contains(at) {
                if mouse.kind == MouseEventKind::ScrollUp {
                    self.tree.up();
                } else if mouse.kind == MouseEventKind::ScrollDown {
                    self.tree.down();
                } else if left_click {
                    self.click_row(at);
                }
            }
        }
    }

    fn click_row(&mut self, at: Position) {
        // ratatui-widgets 0.3.2 list/state.rs exposes the first visible item,
        // but no hit testing. Each explorer label occupies exactly one line.
        let row = self
            .list_state
            .offset()
            .saturating_add(usize::from(at.y.saturating_sub(self.list_area.y)));
        if !self.tree.select_row(row) {
            return;
        }
        self.footer = None;
        let column = usize::from(at.x.saturating_sub(self.list_area.x));
        let selected = self.tree.selected();
        if matches!(selected, Some(Item::Folder(..) | Item::ProjectScopes))
            || self.tree.project_disclosure_column() == Some(column)
        {
            self.tree.toggle_branch();
        } else {
            self.activate(Some(at));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{super::*, *};
    use crate::test_support;
    use ratatui::crossterm::event::MouseEvent;
    use ratatui::{backend::TestBackend, layout::Rect};

    fn left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: event::KeyModifiers::NONE,
        }
    }

    #[test]
    fn project_clicks_offer_actions_for_each_state_and_dismiss_outside() {
        let live = &["view", "close", "deregister"][..];
        for (offset, height, state, options, hit_row) in [
            (0, 10, State::Missing, &["deregister"][..], Some(2)),
            (
                1,
                3,
                State::NotRunning,
                &["open", "deregister"][..],
                Some(0),
            ),
            (0, 4, State::Running, live, Some(0)),
            (0, 10, State::Starting, live, None),
            (0, 10, State::Waiting(None), live, None),
        ] {
            let mut explorer = super::super::tests::explorer();
            let agents = (options == live).then(tree::tests::lead_only);
            explorer
                .tree
                .replace(vec![tree::tests::project("sample", state, agents)]);
            test_support::render(40, height, |frame| explorer.draw(frame));
            *explorer.list_state.offset_mut() = offset;
            let row = u16::from(offset == 0);
            explorer.event(Event::Mouse(left_click(39, row)));
            test_support::render(40, height, |frame| explorer.draw(frame));
            assert!(
                matches!(&explorer.mode, Mode::Menu(menu) if menu.labels().eq(options.iter().copied()))
            );
            if let (Mode::Menu(menu), Some(y)) = (&explorer.mode, hit_row) {
                let hit = |x, y| menu.action_at(Position::new(x, y)).map(menu::Action::label);
                assert_eq!(hit(39, row), None);
                assert_eq!(hit(30, y), options.first().copied());
                assert_eq!(hit(30, explorer.list_area.bottom()), None);
            }
            explorer.event(Event::Mouse(left_click(0, 0)));
            assert!(matches!(explorer.mode, Mode::Browse));
            assert_eq!(explorer.tree.cursor(), 1);
        }
    }

    #[test]
    fn worker_clicks_and_enter_offer_close_for_live_and_lost_workers() {
        let mut explorer = super::super::tests::explorer();
        explorer.tree = tree::tests::tree();
        explorer.event(Event::Mouse(left_click(10, 3)));
        assert!(matches!(&explorer.mode, Mode::Menu(menu) if menu.labels().eq(["view", "close"])));
        explorer.key(KeyCode::Esc);
        explorer.tree.down();
        explorer.key(KeyCode::Enter);
        assert!(matches!(&explorer.mode, Mode::Menu(menu) if menu.labels().eq(["close"])));
    }

    #[test]
    fn clicks_toggle_folders_and_project_disclosures() {
        let mut explorer = super::super::tests::explorer();
        explorer.tree = tree::tests::tree();
        explorer.list_area = Rect::new(2, 2, 36, 30);
        for (row, text) in tree::tests::labels(&explorer.tree).iter().enumerate() {
            let Some(column) = text.chars().position(|ch| "▸▾".contains(ch)) else {
                continue;
            };
            let column = u16::try_from(column).unwrap();
            let click = Event::Mouse(left_click(column + 2, u16::try_from(row + 2).unwrap()));
            let count = explorer.tree.items().len();
            explorer.event(click.clone());
            assert!(matches!(explorer.mode, Mode::Browse));
            assert_ne!(explorer.tree.items().len(), count);
            explorer.event(click);
            assert_eq!(explorer.tree.items().len(), count);
        }
    }

    #[test]
    fn clicks_ignore_blank_space_other_buttons_and_prompts() {
        let mut explorer = super::super::tests::explorer();
        explorer.list_area.height = 20;
        let click = left_click(0, 1);
        for (kind, column, row) in [
            (click.kind, 0, explorer.list_area.bottom()),
            (
                click.kind,
                0,
                u16::try_from(explorer.tree.items().len()).unwrap(),
            ),
            (click.kind, explorer.list_area.right(), 1),
            (MouseEventKind::Down(MouseButton::Right), 0, 1),
            (MouseEventKind::Up(MouseButton::Left), 0, 1),
        ] {
            let mut mouse = left_click(column, row);
            mouse.kind = kind;
            explorer.event(Event::Mouse(mouse));
            assert_eq!(explorer.tree.cursor(), 0);
        }
        explorer.key(KeyCode::Char('r'));
        explorer.event(Event::Mouse(click));
        assert_eq!(explorer.tree.cursor(), 0);
    }

    #[test]
    fn wheel_is_bounded_and_preserves_viewport_when_reversing() {
        let mut explorer = super::super::tests::explorer();
        let mut terminal = ratatui::Terminal::new(TestBackend::new(40, 6)).unwrap();
        let mut wheel = left_click(0, 0);
        wheel.kind = MouseEventKind::ScrollDown;
        for _ in 0..20 {
            explorer.event(Event::Mouse(wheel));
            terminal.draw(|frame| explorer.draw(frame)).unwrap();
        }
        assert_eq!(explorer.tree.cursor(), explorer.tree.items().len() - 1);
        let offset = explorer.list_state.offset();
        assert!(offset > 0);
        wheel.kind = MouseEventKind::ScrollUp;
        explorer.event(Event::Mouse(wheel));
        terminal.draw(|frame| explorer.draw(frame)).unwrap();
        assert_eq!(explorer.list_state.offset(), offset);
        for _ in 0..20 {
            explorer.event(Event::Mouse(wheel));
        }
        assert_eq!(explorer.tree.cursor(), 0);
    }
}

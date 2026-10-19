use super::{Explorer, Mode};
use ratatui::{
    crossterm::event::{Event, KeyEventKind, MouseEventKind},
    layout::Position,
};

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

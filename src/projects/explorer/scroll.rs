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
        let mut explorer = Explorer {
            pane: TmuxTarget::pane("%1").unwrap(),
            cwd: "/tmp".into(),
            tree: Tree::default(),
            list_state: ListState::default(),
            list_area: ratatui::layout::Rect::new(0, 0, 40, 4),
            collected: Instant::now(),
            theme: Theme::parse(None).unwrap(),
            mode: Mode::Browse,
            footer: None,
            binary: "/tmp/niles".into(),
            installed: SystemTime::UNIX_EPOCH,
        };
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

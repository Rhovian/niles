use super::*;

pub(super) fn explorer() -> Explorer {
    Explorer {
        pane: TmuxTarget::pane("%1").unwrap(),
        cwd: "/tmp".into(),
        tree: Tree::default(),
        list_state: ListState::default(),
        list_area: ratatui::layout::Rect::new(0, 0, 40, 4),
        collected: Instant::now(),
        theme: Theme::parse(None).unwrap(),
        mode: Mode::Browse,
        footer: None,
        collection_error: None,
        binary: "/tmp/niles".into(),
        installed: SystemTime::UNIX_EPOCH,
    }
}

#[test]
fn footer_keys_are_muted() {
    let theme = Theme::parse(None).unwrap();
    let line = key_hints(&theme);
    assert_eq!(
        line.to_string(),
        "↵ open · r register · c close · d deregister"
    );
    assert_eq!(line.style, theme.style(StyleKey::Muted));
}

#[test]
fn failed_refresh_retains_tree_and_error_until_newer_success() {
    use crate::projects::{registry::Entry, rows::Row};
    use ratatui::backend::TestBackend;

    let mut explorer = explorer();
    let initial = explorer.collected + REFRESH;
    explorer.accept_collection(initial, Err(anyhow::anyhow!("tmux unavailable")));
    assert_eq!(
        explorer.collection_error.as_deref(),
        Some("refresh failed; retrying: tmux unavailable")
    );
    explorer.key(KeyCode::Down).unwrap();
    explorer.key(KeyCode::Up).unwrap();
    let start = initial + REFRESH;
    explorer.accept_collection(
        start,
        Ok(vec![tree::Project {
            row: Row {
                entry: Entry {
                    name: ProjectName::parse("retained").unwrap(),
                    path: "/tmp".into(),
                },
                state: State::NotRunning,
            },
            agents: None,
        }]),
    );
    assert!(explorer.collection_error.is_none());
    explorer.key(KeyCode::Down).unwrap();
    let item_count = explorer.tree.items().len();
    let failed = start + REFRESH;
    explorer.accept_collection(
        failed,
        Err(anyhow::anyhow!("transcript unavailable").context("collect project")),
    );
    assert!(
        matches!(explorer.tree.selected(), Some(Item::Project(project))
        if project.row.entry.name.as_str() == "retained")
    );
    explorer.key(KeyCode::Up).unwrap();
    let error = "refresh failed; retrying: collect project: transcript unavailable";
    assert_eq!(explorer.collection_error.as_deref(), Some(error));
    let mut terminal = ratatui::Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| explorer.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(rendered.contains(error));
    explorer.accept_collection(start, Ok(Vec::new()));
    assert_eq!(explorer.collection_error.as_deref(), Some(error));
    assert_eq!(explorer.tree.items().len(), item_count);
    explorer.accept_collection(failed + REFRESH, Ok(Vec::new()));
    assert!(explorer.collection_error.is_none());
    assert_eq!(explorer.tree.items().len(), item_count - 1);
    explorer.accept_collection(failed, Err(anyhow::anyhow!("stale failure")));
    assert!(explorer.collection_error.is_none());
}

#[test]
fn deregister_requires_project_selection_and_confirmation() {
    use crate::projects::{registry::Entry, rows::Row};
    let mut explorer = explorer();
    explorer.key(KeyCode::Char('d')).unwrap();
    assert_eq!(
        explorer.footer.as_deref(),
        Some("select a project to deregister")
    );
    explorer.tree.replace(vec![tree::Project {
        row: Row {
            entry: Entry {
                name: ProjectName::parse("sample").unwrap(),
                path: "/tmp/sample".into(),
            },
            state: State::NotRunning,
        },
        agents: None,
    }]);
    explorer.key(KeyCode::Down).unwrap();
    explorer.key(KeyCode::Char('d')).unwrap();
    assert!(matches!(&explorer.mode, Mode::Deregister(entry) if entry.name.as_str() == "sample"));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 20)).unwrap();
    terminal.draw(|frame| explorer.draw(frame)).unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("deletes all state and managed worktrees"));
    explorer.key(KeyCode::Char('n')).unwrap();
    assert!(matches!(explorer.mode, Mode::Browse));
}

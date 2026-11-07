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
        Ok(vec![tree::tests::project(
            "retained",
            State::NotRunning,
            None,
        )]),
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
    let rendered = crate::test_support::render(100, 20, |frame| explorer.draw(frame));
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
    let mut explorer = explorer();
    explorer.key(KeyCode::Char('d')).unwrap();
    assert_eq!(
        explorer.footer.as_deref(),
        Some("select a project to deregister")
    );
    explorer.tree.replace(vec![tree::tests::project(
        "sample",
        State::NotRunning,
        None,
    )]);
    explorer.key(KeyCode::Down).unwrap();
    explorer.key(KeyCode::Char('d')).unwrap();
    assert!(matches!(&explorer.mode, Mode::Deregister(entry) if entry.name.as_str() == "sample"));
    let rendered = crate::test_support::render(120, 20, |frame| explorer.draw(frame));
    assert!(rendered.contains("deletes all state and managed worktrees"));
    explorer.key(KeyCode::Char('n')).unwrap();
    assert!(matches!(explorer.mode, Mode::Browse));
}

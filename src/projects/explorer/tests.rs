use super::*;
use crate::test_support;
use ratatui::layout::Rect;

pub(super) fn explorer() -> Explorer {
    Explorer {
        pane: TmuxTarget::pane("%1").unwrap(),
        cwd: "/tmp".into(),
        tree: Tree::default(),
        list_state: ListState::default(),
        list_area: Rect::new(0, 0, 40, 4),
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
fn failed_refresh_retains_tree_and_error_until_newer_success() {
    let mut explorer = explorer();
    let initial = explorer.collected + REFRESH;
    explorer.accept_collection(initial, Err(anyhow::anyhow!("tmux unavailable")));
    assert_eq!(
        explorer.collection_error.as_deref(),
        Some("refresh failed; retrying: tmux unavailable")
    );
    explorer.key(KeyCode::Down);
    explorer.key(KeyCode::Up);
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
    explorer.key(KeyCode::Down);
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
    explorer.key(KeyCode::Up);
    let error = "refresh failed; retrying: collect project: transcript unavailable";
    assert_eq!(explorer.collection_error.as_deref(), Some(error));
    let rendered = test_support::render(100, 20, |frame| explorer.draw(frame));
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
fn keyboard_moves_the_menu_highlight_and_escape_dismisses_it() {
    use menu::Action;

    let mut explorer = explorer();
    explorer.tree = tree::tests::tree();
    for (key, action) in [
        (KeyCode::Enter, None),
        (KeyCode::Enter, None),
        (KeyCode::Up, Some("deregister")),
        (KeyCode::Up, Some("close")),
        (KeyCode::Up, Some("view")),
        (KeyCode::Up, Some("view")),
        (KeyCode::Down, Some("close")),
        (KeyCode::Down, Some("deregister")),
        (KeyCode::Down, Some("deregister")),
    ] {
        explorer.key(key);
        assert!(
            matches!(&explorer.mode, Mode::Menu(menu) if menu.highlighted().map(Action::label) == action)
        );
    }
    let rendered = test_support::render(40, 12, |frame| explorer.draw(frame));
    assert!(rendered.contains("↑↓ choose · ↵ run · esc dismiss"));
    explorer.key(KeyCode::Esc);
    assert!(matches!(explorer.mode, Mode::Browse));
    assert_eq!(explorer.tree.cursor(), 1);
}

#[test]
fn refresh_dismisses_moved_rows_and_changed_menu_actions() {
    let mut explorer = explorer();
    let project = |state| tree::tests::project("sample", state, None);
    explorer.tree.replace(vec![project(State::NotRunning)]);
    explorer.tree.down();
    explorer.key(KeyCode::Enter);
    let started = explorer.collected + REFRESH;
    explorer.accept_collection(started, Ok(vec![project(State::NotRunning)]));
    assert!(matches!(explorer.mode, Mode::Menu(_)));
    let shifted = started + REFRESH;
    let earlier = || tree::tests::project("earlier", State::Missing, None);
    let rows = |state| vec![earlier(), project(state)];
    explorer.accept_collection(shifted, Ok(rows(State::NotRunning)));
    assert!(matches!(explorer.mode, Mode::Browse));
    assert_eq!(explorer.tree.cursor(), 2);
    explorer.key(KeyCode::Enter);
    explorer.accept_collection(shifted + REFRESH, Ok(rows(State::Missing)));
    assert!(matches!(explorer.mode, Mode::Browse));
}

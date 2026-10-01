//! Reads a draft from the cursor's position in the visible pane.

use crate::{agents::ComposerState, tmux::CursorPosition};

pub(crate) fn recognize(screen: &str, cursor: CursorPosition, marker: &str) -> ComposerState {
    if !cursor.visible {
        return ComposerState::Unknown;
    }
    let lines: Vec<_> = screen.lines().collect();
    let Some(line) = lines.get(cursor.y) else {
        return ComposerState::Unknown;
    };
    if line.starts_with(marker) {
        return match cursor.x.cmp(&2) {
            std::cmp::Ordering::Equal => ComposerState::Empty,
            std::cmp::Ordering::Greater => ComposerState::Typed,
            std::cmp::Ordering::Less => ComposerState::Unknown,
        };
    }
    if !line.starts_with("  ") {
        return ComposerState::Unknown;
    }
    for line in lines[..cursor.y].iter().rev() {
        if line.starts_with(marker) {
            return ComposerState::Typed;
        }
        if !line.starts_with("  ") {
            break;
        }
    }
    ComposerState::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_identifies_drafts_for_both_families() {
        for marker in ["❯\u{a0}", "› "] {
            let cursor = |x, y, visible| CursorPosition { x, y, visible };
            let empty = format!("header\n{marker}placeholder\nfooter\n");
            let typed = format!("header\n{marker}hello\nfooter\n");
            let wrapped = format!("header\n{marker}hello\n  world\nfooter\n");
            let dialog = format!("{marker}hello\n╭ dialog\n  option\n");
            assert_eq!(
                recognize(&empty, cursor(2, 1, true), marker),
                ComposerState::Empty
            );
            assert_eq!(
                recognize(&typed, cursor(7, 1, true), marker),
                ComposerState::Typed
            );
            assert_eq!(
                recognize(&wrapped, cursor(7, 2, true), marker),
                ComposerState::Typed
            );
            assert_eq!(
                recognize(&typed, cursor(4, 0, true), marker),
                ComposerState::Unknown
            );
            assert_eq!(
                recognize(&dialog, cursor(4, 2, true), marker),
                ComposerState::Unknown
            );
            assert_eq!(
                recognize(&typed, cursor(7, 1, false), marker),
                ComposerState::Unknown
            );
            assert_eq!(
                recognize(&typed, cursor(7, 3, true), marker),
                ComposerState::Unknown
            );
        }
    }
}

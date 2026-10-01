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
        // Both markers are single-cell characters, so their width in cells is their char count.
        return match cursor.x.cmp(&marker.chars().count()) {
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
        use ComposerState::{Empty, Typed, Unknown};
        for marker in ["❯\u{a0}", "› "] {
            let empty = format!("header\n{marker}placeholder\nfooter\n");
            let typed = format!("header\n{marker}hello\nfooter\n");
            let wrapped = format!("header\n{marker}hello\n  world\nfooter\n");
            let dialog = format!("{marker}hello\n╭ dialog\n  option\n");
            let cases = [
                (&empty, 2, 1, true, Empty),
                (&typed, 7, 1, true, Typed),
                (&wrapped, 7, 2, true, Typed),
                (&typed, 4, 0, true, Unknown),
                (&dialog, 4, 2, true, Unknown),
                (&typed, 7, 1, false, Unknown),
                (&typed, 7, 3, true, Unknown),
            ];
            for (screen, x, y, visible, expected) in cases {
                let cursor = CursorPosition { x, y, visible };
                assert_eq!(
                    recognize(screen, cursor, marker),
                    expected,
                    "{screen:?} at {x},{y}"
                );
            }
        }
    }
}

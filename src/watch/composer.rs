//! Reads a draft from the cursor's position in the visible pane.

use crate::tmux::CursorPosition;

pub(crate) fn recognize(screen: &str, cursor: CursorPosition, marker: &str) -> bool {
    if !cursor.visible {
        return false;
    }
    let lines: Vec<_> = screen.lines().collect();
    let Some(line) = lines.get(cursor.y) else {
        return false;
    };
    if line.starts_with(marker) {
        // Both markers are single-cell characters, so their width in cells is their char count.
        return cursor.x > marker.chars().count();
    }
    if !line.starts_with("  ") {
        return false;
    }
    for line in lines[..cursor.y].iter().rev() {
        if line.starts_with(marker) {
            return true;
        }
        if !line.starts_with("  ") {
            break;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_identifies_drafts_for_both_families() {
        for marker in ["❯\u{a0}", "› "] {
            let screen = format!("header\n{marker}hello\n  world\nfooter\n");
            let dialog = format!("{marker}hello\n╭ dialog\n  option\n");
            for (text, x, y, visible, expected) in [
                (&screen, 2, 1, true, false),
                (&screen, 7, 1, true, true),
                (&screen, 7, 2, true, true),
                (&screen, 4, 0, true, false),
                (&dialog, 4, 2, true, false),
                (&screen, 7, 1, false, false),
                (&screen, 7, 4, true, false),
            ] {
                assert_eq!(
                    recognize(text, CursorPosition { x, y, visible }, marker),
                    expected
                );
            }
        }
    }
}

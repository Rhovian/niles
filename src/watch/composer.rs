//! Reads a draft from the cursor's position in the visible pane.

use crate::tmux::CursorPosition;

pub(crate) fn recognize(screen: &str, cursor: CursorPosition, markers: &[&str]) -> bool {
    if !cursor.visible {
        return false;
    }
    let lines: Vec<_> = screen.lines().collect();
    let Some(line) = lines.get(cursor.y) else {
        return false;
    };
    if let Some(marker) = markers.iter().find(|marker| line.starts_with(**marker)) {
        // All marker characters occupy one cell each.
        return cursor.x > marker.chars().count();
    }
    if !line.starts_with("  ") {
        return false;
    }
    for line in lines[..cursor.y].iter().rev() {
        if markers.iter().any(|marker| line.starts_with(marker)) {
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
    fn cursor_identifies_drafts_for_every_declared_marker() {
        for (family, markers, marker) in crate::agents::profiles().flat_map(|profile| {
            profile.composer.into_iter().flat_map(move |markers| {
                markers
                    .iter()
                    .map(move |marker| (profile.id, markers, marker))
            })
        }) {
            let screen = format!("header\n{marker}hello\n  world\nfooter\n");
            let empty = format!("{marker}\n");
            let dialog = format!("{marker}hello\n╭ dialog\n  option\n");
            for (text, x, y, visible, expected) in [
                (&empty, 2, 0, true, false),
                (&screen, 2, 1, true, false),
                (&screen, 7, 1, true, true),
                (&screen, 7, 2, true, true),
                (&screen, 4, 0, true, false),
                (&dialog, 4, 2, true, false),
                (&screen, 7, 1, false, false),
                (&screen, 7, 4, true, false),
            ] {
                assert_eq!(
                    recognize(text, CursorPosition { x, y, visible }, markers),
                    expected,
                    "{family} {marker:?}"
                );
            }
        }
    }
}

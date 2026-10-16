use super::{CONFIRM_INTERVAL, CONFIRM_TIMEOUT, confirm};
use crate::{agents::SwitchStep, tmux};
use anyhow::{Context, Result, bail, ensure};
use std::{collections::HashSet, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Picker {
    Quick,
    Models,
    Effort,
    Advanced,
}

// Codex rust-v0.162.1, tui/src/chatwidget/{model_popups,session_model_selection}.rs:
// /model has no inline arguments or search. Final rows expose `s` to apply only to the
// active session, including Plan mode, without its scope prompt or persisting defaults.
pub(super) fn switch(pane: &tmux::TmuxTarget, model: &str, effort: &str) -> Result<()> {
    let screen = tmux::capture_visible_pane(pane)?;
    let (picker, _) = selection(&screen, model)?;
    if picker == Picker::Quick {
        highlight(pane, Picker::Quick, model, "All models")?;
        tmux::send_input(pane, "", &["C-m"])?;
    }
    highlight(pane, Picker::Models, model, model)?;
    tmux::send_input(pane, "", &["C-m"])?;
    let (picker, label) = match effort {
        "max" | "ultra" => {
            highlight(pane, Picker::Effort, model, "More reasoning…")?;
            tmux::send_input(pane, "", &["C-m"])?;
            (Picker::Advanced, effort)
        }
        "xhigh" => (Picker::Effort, "Extra high"),
        _ => (Picker::Effort, effort),
    };
    let screen = highlight(pane, picker, model, label)?;
    ensure!(screen.contains("s session"), "Codex session action missing");
    let before = tmux::capture_pane(pane, 0)?;
    confirm(
        pane,
        &SwitchStep {
            text: String::new(),
            keys: vec!["s"],
            acknowledgement: (
                format!("Model changed to {model} {effort} for this session only"),
                String::new(),
            ),
        },
    )?;
    check_update(&before, &tmux::capture_pane(pane, 0)?)
}

// Codex rust-v0.162.1 app/{model_defaults,thread_settings}.rs emits the success line
// even when the server rejects the update. That error must veto the acknowledgement.
fn check_update(before: &str, after: &str) -> Result<()> {
    const ERROR: &str = "■ ";
    ensure!(
        after.matches(ERROR).count() <= before.matches(ERROR).count(),
        "Codex failed to update thread settings"
    );
    Ok(())
}

fn highlight(pane: &tmux::TmuxTarget, picker: Picker, model: &str, label: &str) -> Result<String> {
    let deadline = Instant::now() + CONFIRM_TIMEOUT;
    let mut visited = HashSet::new();
    loop {
        let screen = tmux::capture_visible_pane(pane)?;
        let (current, selected) = selection(&screen, model)?;
        if current != picker {
            ensure!(Instant::now() < deadline, "Codex did not open {picker:?}");
            std::thread::sleep(CONFIRM_INTERVAL);
            continue;
        }
        if selected.eq_ignore_ascii_case(label) {
            return Ok(screen);
        }
        ensure!(
            visited.insert(selected.to_owned()) && Instant::now() < deadline,
            "Codex {picker:?} has no choice {label:?}"
        );
        tmux::send_input(pane, "", &["Down"])?;
    }
}

// The pinned TUI's bottom_pane/list_selection_view.rs renders numbered rows with a
// single › selection and supports arrow navigation even when rows scroll off screen.
fn selection<'a>(screen: &'a str, model: &str) -> Result<(Picker, &'a str)> {
    let lines: Vec<_> = screen.lines().map(str::trim).collect();
    let (index, picker) = lines
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, line)| {
            let picker = match *line {
                "Select Model" => Picker::Quick,
                "Select Model and Effort" => Picker::Models,
                "Advanced Reasoning" => Picker::Advanced,
                line if line
                    .strip_prefix("Select Reasoning Level for ")
                    .is_some_and(|name| name.eq_ignore_ascii_case(model)) =>
                {
                    Picker::Effort
                }
                _ => return None,
            };
            Some((index, picker))
        })
        .context("unrecognized Codex picker")?;
    let mut selected = lines[index + 1..]
        .iter()
        .filter_map(|line| line.strip_prefix("› "));
    let row = selected.next().context("Codex picker selection missing")?;
    ensure!(
        selected.next().is_none(),
        "ambiguous Codex picker selection"
    );
    let (number, text) = row.split_once(". ").context("invalid Codex picker row")?;
    ensure!(
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()),
        "invalid Codex picker number"
    );
    let name = text.split_once("  ").map_or(text, |(name, _)| name);
    let name = name.split_once(" (").map_or(name, |(name, _)| name);
    if name.is_empty() {
        bail!("empty Codex choice");
    }
    Ok((picker, name))
}

#[cfg(test)]
mod tests;

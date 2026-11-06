use super::ui;
use crate::skills::{self, reference::GithubRef};
use anyhow::{Context, Result, ensure};
use camino::Utf8Path;
use ratatui::{
    crossterm::event::KeyCode,
    widgets::{Paragraph, Wrap},
};
use std::fs;

pub(super) enum Choice {
    Next(GithubRef),
    Back,
    Quit,
}

pub(super) fn reference(text: &str, cwd: &Utf8Path, global: &Utf8Path) -> Result<GithubRef> {
    let reference: GithubRef = text.parse()?;
    ensure_available(&reference.name, cwd, global)?;
    Ok(reference)
}

fn ensure_available(name: &skills::SkillName, cwd: &Utf8Path, global: &Utf8Path) -> Result<()> {
    ensure!(
        skills::find(name, cwd, global)?.is_none(),
        "skill {name} is already installed; choose a different name"
    );
    Ok(())
}

pub(super) fn install<T>(
    reference: GithubRef,
    cwd: &Utf8Path,
    global: &Utf8Path,
    finish: impl FnOnce() -> Result<T>,
) -> Result<T> {
    ensure_available(&reference.name, cwd, global)?;
    let pointer = skills::import(
        reference,
        skills::Origin::Global,
        skills::ImportMode::NewPrimary,
    )?;
    match finish() {
        Ok(value) => Ok(value),
        Err(error) => {
            fs::remove_file(&pointer)
                .with_context(|| format!("setup failed ({error:#}); could not remove {pointer}"))?;
            Err(error)
        }
    }
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "text input ignores non-text keys"
)]
pub(super) fn input(
    text: &mut String,
    header: &str,
    cwd: &Utf8Path,
    global: &Utf8Path,
) -> Result<Choice> {
    let mut error = String::new();
    ui::screen(|terminal| {
        loop {
            let prompt = format!(
                "{header}\n\nCustom lead skill URL (skill import reference)\ngithub.com/owner/repo/path[@rev]\n{text}\n{error}\n\nEnter continue · Esc back · F10 quit"
            );
            terminal.draw(|frame| {
                frame.render_widget(
                    Paragraph::new(prompt).wrap(Wrap { trim: false }),
                    frame.area(),
                );
            })?;
            match ui::key()? {
                KeyCode::Esc => return Ok(Choice::Back),
                KeyCode::F(10) => return Ok(Choice::Quit),
                KeyCode::Enter => match reference(text, cwd, global) {
                    Ok(reference) => return Ok(Choice::Next(reference)),
                    Err(problem) => error = format!("{problem:#}"),
                },
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(ch) => text.push(ch),
                _ => {}
            }
        }
    })
}

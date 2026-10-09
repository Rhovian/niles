use anyhow::Result;
use camino::Utf8Path;
use ratatui::{
    DefaultTerminal,
    crossterm::event::{self, Event, KeyEventKind},
};

use crate::{config::spec::ProjectConfig, theme::Theme, workspace_manifest::WorkspaceManifest};

mod columns;
mod draw;
mod form;
mod presets;
mod role;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use presets::load as load_presets;

use columns::{Columns, Pick};
use form::Form;
pub(crate) use role::Role;

pub(crate) enum Choice {
    Save(WorkspaceManifest),
    /// Launch with the manifest as it is.
    Keep,
    /// Stop the launch.
    Quit,
}

pub(crate) fn roles(
    root: &Utf8Path,
    manifest: WorkspaceManifest,
    config: &ProjectConfig,
) -> Result<Choice> {
    let theme = Theme::load()?;
    let mut form = Form::new(manifest, config)?;
    let mut terminal = ratatui::init();
    let result = run_form(&mut terminal, root, &mut form, config, &theme);
    ratatui::restore();
    result
}

fn run_form(
    terminal: &mut DefaultTerminal,
    root: &Utf8Path,
    form: &mut Form,
    config: &ProjectConfig,
    theme: &Theme,
) -> Result<Choice> {
    loop {
        terminal.draw(|frame| draw::form(frame, form, theme, root.as_str()))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(choice) = form.key(key.code, config)?
        {
            return Ok(choice);
        }
    }
}

/// Uses the caller's terminal, including CONFIG's existing raw mode and alternate screen.
pub(crate) fn role(
    terminal: &mut DefaultTerminal,
    role: Role,
    manifest: &WorkspaceManifest,
    config: &ProjectConfig,
    theme: &Theme,
) -> Result<Option<String>> {
    let families = columns::families(config)?;
    let mut columns = Columns::new(
        &families,
        &role.value(manifest),
        role == Role::Reviewer,
        config,
    )?;
    loop {
        terminal.draw(|frame| draw::single(frame, &columns, role, theme))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match columns.key(key.code) {
                Pick::Pending => {}
                Pick::Cancelled => return Ok(None),
                Pick::Selected(value) => return Ok(Some(value)),
            }
        }
    }
}

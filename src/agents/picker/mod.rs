use crate::agents::roster::Tier;
use crate::config::spec::ProjectConfig;
use crate::theme::Theme;
use crate::workspace_manifest::WorkspaceManifest;
use anyhow::Result;
use camino::Utf8Path;
use form::{Decision, Form, Screen};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

mod columns;
mod draw;
mod form;
#[cfg(test)]
mod tests;

pub(crate) enum Choice {
    Save(Box<WorkspaceManifest>),
    /// Launch with the manifest as it is.
    Keep,
    /// Stop the launch.
    Quit,
}

pub(crate) fn tiers(
    root: &Utf8Path,
    manifest: WorkspaceManifest,
    config: &ProjectConfig,
) -> Result<Choice> {
    let mut form = Form::new(manifest, config)?;
    let decision = screen(root, &mut form, |form, key| form.key(key, config))?;
    Ok(choice(&form, decision))
}

fn choice(form: &Form, decision: Decision) -> Choice {
    match decision {
        Decision::Save => Choice::Save(Box::new(form.draft.clone())),
        Decision::Keep => Choice::Keep,
        Decision::Quit => Choice::Quit,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SetupAction {
    Next,
    Back,
    Quit,
}

pub(crate) struct SetupTiers(Form);
impl SetupTiers {
    pub(crate) fn new(manifest: WorkspaceManifest, config: &ProjectConfig) -> Result<Self> {
        let mut form = Form::new(manifest, config)?;
        form.setup = true;
        Ok(Self(form))
    }
    pub(crate) fn draft(&self) -> &WorkspaceManifest {
        &self.0.draft
    }

    fn key(form: &mut Form, key: KeyCode, config: &ProjectConfig) -> Result<Option<SetupAction>> {
        if matches!(key, KeyCode::Esc | KeyCode::Left) && matches!(form.screen, Screen::Tiers) {
            return Ok(Some(SetupAction::Back));
        }
        Ok(form.key(key, config)?.map(|decision| match decision {
            Decision::Save | Decision::Keep => SetupAction::Next,
            Decision::Quit => SetupAction::Quit,
        }))
    }

    pub(crate) fn pick(&mut self, root: &Utf8Path, config: &ProjectConfig) -> Result<SetupAction> {
        screen(root, &mut self.0, |form, key| Self::key(form, key, config))
    }
}

fn screen<T>(
    root: &Utf8Path,
    form: &mut Form,
    on_key: impl FnMut(&mut Form, KeyCode) -> Result<Option<T>>,
) -> Result<T> {
    let theme = Theme::load()?;
    let mut terminal = ratatui::init();
    let result = run_form(&mut terminal, root, form, &theme, on_key);
    ratatui::restore();
    result
}

fn run_form<T>(
    terminal: &mut DefaultTerminal,
    root: &Utf8Path,
    form: &mut Form,
    theme: &Theme,
    mut on_key: impl FnMut(&mut Form, KeyCode) -> Result<Option<T>>,
) -> Result<T> {
    loop {
        terminal.draw(|frame| draw::form(frame, form, theme, root.as_str()))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(choice) = on_key(form, key.code)?
        {
            return Ok(choice);
        }
    }
}

/// Reuses CONFIG's terminal and the same editable form as setup and startup.
pub(crate) fn configure(
    terminal: &mut DefaultTerminal,
    root: &Utf8Path,
    manifest: WorkspaceManifest,
    config: &ProjectConfig,
    theme: &Theme,
    tier: Tier,
) -> Result<Choice> {
    let mut form = Form::new(manifest, config)?;
    form.select_agent(tier, 0)?;
    let decision = run_form(terminal, root, &mut form, theme, |form, key| {
        form.key(key, config)
    })?;
    Ok(choice(&form, decision))
}

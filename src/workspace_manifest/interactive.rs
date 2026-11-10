use super::setup::SetupSelection;
use super::tiers_table::print_manifest_tiers;
use super::{WorkspaceManifest, check, load, manifest_path, save};
use crate::agents::picker::{self, Choice};
use crate::config::spec::{ProjectConfig, load_project_config_from};
use crate::projects::onboard;
use crate::tmux::LeadStart;
use anyhow::{Context, Result};
use camino::Utf8Path;
use std::io::{self, IsTerminal, Write};

/// The manifest to launch with, or `None` when the operator declines setup or quits the picker.
pub(crate) fn ensure_interactive(
    root: &Utf8Path,
    start: LeadStart,
    selection: Option<&SetupSelection>,
) -> Result<Option<WorkspaceManifest>> {
    ensure(
        root,
        selection,
        start == LeadStart::Attended && io::stdin().is_terminal(),
        &mut io::stdout(),
        onboard::setup,
        picker::tiers,
    )
}

fn ensure(
    root: &Utf8Path,
    selection: Option<&SetupSelection>,
    interactive: bool,
    output: &mut impl Write,
    set_up: impl FnOnce(&Utf8Path) -> Result<Option<WorkspaceManifest>>,
    pick: impl FnOnce(&Utf8Path, WorkspaceManifest, &ProjectConfig) -> Result<Choice>,
) -> Result<Option<WorkspaceManifest>> {
    let config = load_project_config_from(root)?;
    let path = manifest_path(root, selection)?;
    let existing = match load(root, selection)? {
        Some(manifest) => manifest,
        None if interactive => return set_up(root),
        None => anyhow::bail!(
            "workspace manifest {path} does not exist; select the project in `niles` in a terminal to set it up"
        ),
    };
    if !interactive {
        print_manifest_tiers(output, &existing, &config)?;
        return Ok(Some(existing));
    }
    match pick(root, existing.clone(), &config)? {
        Choice::Save(draft) => save_changes(root, selection, &existing, &draft).map(Some),
        Choice::Keep => Ok(Some(existing)),
        Choice::Quit => Ok(None),
    }
}

/// Re-read before setting edited tiers and entry agent. Race accepted: concurrent edits of the same tier
/// inside the save window are lost, last writer wins.
pub(crate) fn save_changes(
    root: &Utf8Path,
    selection: Option<&SetupSelection>,
    before: &WorkspaceManifest,
    draft: &WorkspaceManifest,
) -> Result<WorkspaceManifest> {
    let path = manifest_path(root, selection)?;
    let mut latest = load(root, selection)?.with_context(|| format!("{path} is gone"))?;
    let mut changed = false;
    for (tier, binding) in draft.tiers.iter() {
        if before.tiers.get(tier) != Some(binding) {
            latest.tiers.set(tier, binding.clone());
            changed = true;
        }
    }
    if draft.entry_agent != before.entry_agent {
        latest.entry_agent = draft.entry_agent.clone();
        changed = true;
    }
    if changed {
        check(&latest, &load_project_config_from(root)?.models)?;
        save(root, &latest, selection)?;
    }
    Ok(latest)
}

#[cfg(test)]
mod tests;

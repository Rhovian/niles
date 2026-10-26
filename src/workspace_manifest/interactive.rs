use std::io::{self, IsTerminal, Write};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use super::{
    WorkspaceManifest, load, manifest_path, save, setup::SetupSelection,
    skills_table::print_manifest_skills,
};
use crate::{
    agents::picker::{self, Choice},
    config::spec::load_project_config_from,
};

/// The manifest to launch with, or `None` when the operator quit the picker.
pub(crate) fn ensure_interactive(
    root: &Utf8Path,
    start: crate::tmux::LeadStart,
    selection: Option<&SetupSelection>,
) -> Result<Option<WorkspaceManifest>> {
    ensure(
        root,
        selection,
        start == crate::tmux::LeadStart::Attended && io::stdin().is_terminal(),
        &mut io::stdout(),
        picker::skills,
    )
}

fn ensure(
    root: &Utf8Path,
    selection: Option<&SetupSelection>,
    interactive: bool,
    output: &mut impl Write,
    pick: impl FnOnce(
        &Utf8Path,
        WorkspaceManifest,
        &crate::config::spec::ProjectConfig,
        Option<&str>,
    ) -> Result<Choice>,
) -> Result<Option<WorkspaceManifest>> {
    let config = load_project_config_from(root)?;
    // serde-saphyr 1.3.0's from_str_with_options returns its typed deserialization Error
    // (src/de/api.rs); only that error opens replacement, never a filesystem failure.
    let (existing, replacement_reason) = match load(root, selection) {
        Ok(existing) => (existing, None),
        Err(error) if interactive && error.is::<serde_saphyr::Error>() => (
            None,
            Some(format!(
                "Cannot parse manifest: {error:#}. Pick skills and save to replace it with defaults; q leaves it unchanged."
            )),
        ),
        Err(error) => return Err(error),
    };
    if !interactive {
        let Some(manifest) = existing else {
            bail!(
                "workspace manifest {} does not exist; run `niles` from an interactive terminal",
                manifest_path(root, selection)?
            );
        };
        print_manifest_skills(output, &manifest, &config)?;
        return Ok(Some(manifest));
    }
    let defaults = match &existing {
        Some(manifest) => manifest.clone(),
        None => WorkspaceManifest::default(),
    };
    let save_reason = replacement_reason.as_deref().or_else(|| {
        existing.is_none().then_some(
            "No workspace manifest exists yet. Save the shown skills to continue; q quits.",
        )
    });
    match pick(root, defaults, &config, save_reason)? {
        Choice::Save(draft) if replacement_reason.is_some() => {
            save(root, &draft, selection)?;
            Ok(Some(*draft))
        }
        Choice::Save(draft) => save_changes(root, selection, existing.as_ref(), &draft).map(Some),
        Choice::Keep => existing.context("no workspace manifest written").map(Some),
        Choice::Quit => Ok(None),
    }
}

/// Re-read before setting skills. Race accepted: a concurrent edit of the same skill inside the
/// save window is lost, last writer wins.
fn save_changes(
    root: &Utf8Path,
    selection: Option<&SetupSelection>,
    before: Option<&WorkspaceManifest>,
    draft: &WorkspaceManifest,
) -> Result<WorkspaceManifest> {
    let mut latest = match (load(root, selection)?, before) {
        (Some(manifest), _) => manifest,
        (None, None) => {
            save(root, draft, selection)?;
            return Ok(draft.clone());
        }
        (None, Some(_)) => bail!("{} is gone", manifest_path(root, selection)?),
    };
    let defaults = WorkspaceManifest::default();
    let before = match before {
        Some(before) => before,
        None => &defaults,
    };
    let mut changed = false;
    for (skill, binding) in &draft.bindings {
        if before.bindings.get(skill) != Some(binding) {
            latest.set_primary(skill, binding.standard.primary.clone())?;
            changed = true;
        }
    }
    if changed {
        save(root, &latest, selection)?;
    }
    Ok(latest)
}

#[cfg(test)]
mod tests;

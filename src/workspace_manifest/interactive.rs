use std::io::{self, IsTerminal, Write};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use super::{WorkspaceManifest, load, manifest_path, roles_table::print_manifest_roles, save};
use crate::{
    agents::picker::{self, Role},
    config::spec::load_project_config_from,
};

pub fn ensure_interactive(root: &Utf8Path) -> Result<WorkspaceManifest> {
    ensure(
        root,
        io::stdin().is_terminal(),
        &mut io::stdout(),
        picker::roles,
    )
}

fn ensure(
    root: &Utf8Path,
    interactive: bool,
    output: &mut impl Write,
    pick: impl FnOnce(
        &Utf8Path,
        WorkspaceManifest,
        &crate::config::spec::ProjectConfig,
    ) -> Result<Option<WorkspaceManifest>>,
) -> Result<WorkspaceManifest> {
    let config = load_project_config_from(root)?;
    let existing = load(root)?;
    if !interactive {
        let Some(manifest) = existing else {
            bail!(
                "workspace manifest {} does not exist; run `niles` from an interactive terminal",
                manifest_path(root)
            );
        };
        print_manifest_roles(output, &manifest, &config)?;
        return Ok(manifest);
    }
    let defaults = match &existing {
        Some(manifest) => manifest.clone(),
        None => WorkspaceManifest::default(),
    };
    match pick(root, defaults, &config)? {
        Some(draft) => save_changes(root, existing.as_ref(), &draft),
        None => existing.context("no workspace manifest written"),
    }
}

/// Re-read before setting roles. Race accepted: a concurrent edit of the same role inside the
/// save window is lost, last writer wins.
fn save_changes(
    root: &Utf8Path,
    before: Option<&WorkspaceManifest>,
    draft: &WorkspaceManifest,
) -> Result<WorkspaceManifest> {
    let mut latest = match (load(root)?, before) {
        (Some(manifest), _) => manifest,
        (None, None) => {
            save(root, draft)?;
            return Ok(draft.clone());
        }
        (None, Some(_)) => bail!("{} is gone", manifest_path(root)),
    };
    let defaults = WorkspaceManifest::default();
    let before = match before {
        Some(before) => before,
        None => &defaults,
    };
    let mut changed = false;
    for role in Role::ALL {
        if role.changed(before, draft) {
            role.set(&mut latest, role.value(draft));
            changed = true;
        }
    }
    if changed {
        save(root, &latest)?;
    }
    Ok(latest)
}

#[cfg(test)]
mod tests;

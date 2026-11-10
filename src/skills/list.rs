use super::{SkillName, display_description, global_dir, resolve};
use crate::workspace_manifest::setup::SetupSelection;
use crate::{util, workspace_manifest};
use anyhow::{Context, Result};
use camino::Utf8Path;
use std::collections::BTreeSet;
use std::io::{self, ErrorKind};
use std::{fs, iter};

pub(crate) fn list(selection: Option<&SetupSelection>) -> Result<()> {
    let workspace = util::current_dir_utf8()?;
    let global = global_dir()?;
    let manifest = workspace_manifest::load_for_skill_listing(&workspace, selection)?
        .context("skills requires a workspace manifest")?;
    let Discovery {
        mut names,
        warnings,
    } = available(&workspace, &global);
    for error in warnings {
        warn(&error);
    }
    names.extend(manifest.bindings.keys().cloned());
    for name in
        iter::once(&manifest.entry).chain(names.iter().filter(|name| **name != manifest.entry))
    {
        // Operator policy: discovery warns and continues; launches still reject invalid skills.
        let skill = match resolve(name, &workspace, &global) {
            Ok(skill) => skill,
            Err(error) => {
                warn(&error);
                continue;
            }
        };
        let description = display_description(&skill.description);
        println!("skill: {skill}\t{description}");
    }
    Ok(())
}

struct Discovery {
    names: BTreeSet<SkillName>,
    warnings: Vec<anyhow::Error>,
}

fn available(workspace: &Utf8Path, global: &Utf8Path) -> Discovery {
    let mut names = BTreeSet::new();
    let mut warnings = Vec::new();
    // Rust 1.89 std/fs.rs: read_dir enumerates entries; resolve owns precedence and symlink rules.
    for root in [workspace.join(".niles/skills"), global.join("skills")] {
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                warnings.push(anyhow::Error::new(error).context(format!("failed to list {root}")));
                continue;
            }
        };
        for entry in entries {
            match candidate(entry) {
                Ok(Some(name)) => {
                    names.insert(name);
                }
                Ok(None) => {}
                Err(error) => warnings.push(error),
            }
        }
    }
    Discovery { names, warnings }
}

fn candidate(entry: io::Result<fs::DirEntry>) -> Result<Option<SkillName>> {
    let entry = entry?;
    let kind = entry.file_type()?;
    if !kind.is_dir() && !kind.is_symlink() {
        return Ok(None);
    }
    let path = entry.path();
    if !path.join("SKILL.md").try_exists()? && !path.join("SKILL.ref").try_exists()? {
        return Ok(None);
    }
    let name = entry
        .file_name()
        .into_string()
        .map_err(|name| anyhow::anyhow!("non-UTF-8 skill name: {name:?}"))?
        .parse()?;
    Ok(Some(name))
}

fn warn(error: &anyhow::Error) {
    eprintln!("warning: {error:#}");
}

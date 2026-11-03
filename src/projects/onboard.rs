mod defaults;
#[cfg(test)]
mod tests;
mod ui;
use super::{register, registry};
use crate::{skills, workspace_manifest as manifest};
use anyhow::{Context, Result, ensure};
use camino::Utf8Path;
use manifest::WorkspaceManifest;
use std::{
    fs::{self, OpenOptions},
    io::{self, IsTerminal, Write},
};

pub(crate) fn setup(cwd: &Utf8Path) -> Result<Option<WorkspaceManifest>> {
    let installed: Vec<_> = crate::agents::profiles()
        .filter(|p| crate::util::find_on_path(p.binary).is_some())
        .map(|p| p.id)
        .collect();
    ensure!(
        !installed.is_empty(),
        "no CLI found; install claude, codex or pi"
    );
    let header = format!(
        "Setting up Niles in {cwd}\nDetected CLIs: {}",
        installed.join(", ")
    );
    let global = skills::global_dir()?;
    let mut modes = ui::Options::new(
        crate::store::parse_yaml(include_str!("onboard/modes.yaml"))?,
        false,
    );
    if !modes.pick(&header, "How do you work?")? {
        return Ok(None);
    }
    let mut generated = defaults::manifest(&installed)?;
    let mode = modes.chosen();
    generated.entry = mode.0.parse()?;
    if mode.0 == "lead" {
        let (rows, warnings) = entry_skills(mode, cwd, &global);
        if rows.len() > 1 {
            let question = format!("Choose the lead's entry skill\n{}", warnings.join("\n"));
            let mut entries = ui::Options::new(rows, false);
            if !entries.pick(&header, &question)? {
                return Ok(None);
            }
            generated.entry = entries.chosen().0.parse()?;
        } else {
            for warning in warnings {
                eprintln!("warning: {warning}");
            }
        }
    }
    let config = crate::config::spec::load_project_config_from(cwd)?;
    let mut manifest = match crate::agents::picker::setup_tiers(cwd, generated.clone(), &config)? {
        crate::agents::picker::Choice::Save(draft) => *draft,
        crate::agents::picker::Choice::Keep => generated,
        crate::agents::picker::Choice::Quit => return Ok(None),
    };
    // The entry agent follows the accepted standard tier, including edits made in the picker.
    manifest.entry_agent = manifest.tiers.standard.primary.clone();
    let rows: Vec<(String, String)> = crate::store::parse_yaml(defaults::SKILLS)?;
    let mut starters = ui::Options::new(
        rows.into_iter()
            .filter(|(name, _)| name != manifest.entry.as_str())
            .collect(),
        true,
    );
    let question = format!(
        "Optional starter skills — instructions that give agents a role. None selected by default.\n\
         Missing selections and the entry skill are fetched from {} at latest (Git HEAD). Existing skills are kept.",
        defaults::REPOSITORY,
    );
    if !starters.pick(&header, &question)? {
        return Ok(None);
    }
    if !ui::confirm(
        &header,
        "Warning: Workers bypass approval prompts and run without a sandbox. See https://github.com/Rhovian/niles/blob/main/docs/security.md for details.",
    )? {
        return Ok(None);
    }
    let starter_skills = starters
        .selected()
        .map(str::parse)
        .collect::<Result<Vec<skills::SkillName>>>()?;
    let tiers = manifest
        .tiers
        .iter()
        .map(|(tier, binding)| {
            format!(
                "{}: {} (secondaries: {})",
                tier.as_str(),
                binding.primary,
                binding.secondaries.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let summary = format!(
        "Project: {cwd}\nEntry skill: {}\n{tiers}\nOptional skills: {}\n\nEnter writes the manifest and starts.",
        manifest.entry,
        starters.selected().collect::<Vec<_>>().join(", ")
    );
    if !ui::confirm(&header, &summary)? {
        return Ok(None);
    }
    manifest::check(&manifest, &config.models)?;
    let body = serde_saphyr::to_string(&manifest)?;
    let names: std::collections::BTreeSet<_> = manifest
        .bindings
        .iter()
        .flat_map(|(name, binding)| std::iter::once(name).chain(binding.attached_skills()))
        .chain(&starter_skills)
        .chain(std::iter::once(&manifest.entry))
        .collect();
    for name in names {
        if skills::find(name, cwd, &global)?.is_none() {
            println!("fetching {name} from latest");
            skills::import(defaults::reference(name)?, skills::Origin::Global)?;
        }
    }
    let path = manifest::manifest_path(cwd, None)?;
    fs::create_dir_all(path.parent().context("manifest has no parent")?)?;
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => file.write_all(body.as_bytes())?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    manifest::load(cwd, None)
}
pub(super) fn first_run(cwd: &Utf8Path) -> Result<Option<registry::Entry>> {
    if !io::stdin().is_terminal() || !registry::entries()?.is_empty() {
        return Ok(None);
    }
    ui::register(cwd)
}

fn entry_skills(
    mode: &(String, String),
    cwd: &Utf8Path,
    global: &Utf8Path,
) -> (Vec<(String, String)>, Vec<String>) {
    let discovery = skills::available(cwd, global);
    let mut rows = vec![mode.clone()];
    let mut warnings = discovery
        .warnings
        .into_iter()
        .map(|error| format!("Skipping skill: {error:#}"))
        .collect::<Vec<_>>();
    for name in discovery.names {
        if name.as_str() == mode.0 {
            continue;
        }
        let candidate = skills::inspect(&name, cwd, global).and_then(|skill| {
            skill.primary_body()?;
            Ok(skill)
        });
        // Discovery follows the skills-list policy: warn about unusable candidates;
        // only the operator's chosen entry is required to be launchable.
        match candidate {
            Ok(skill) => rows.push((
                name.to_string(),
                skills::display_description(&skill.description),
            )),
            Err(error) => warnings.push(format!("Skipping {name}: {error:#}")),
        }
    }
    (rows, warnings)
}

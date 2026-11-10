use super::{register, registry};
use crate::agents::picker::{SetupAction, SetupTiers};
use crate::agents::{self, ModelRoster};
use crate::config::spec;
use crate::store::{self, Publication};
use crate::{skills, util, workspace_manifest as manifest};
use anyhow::{Context, Result, ensure};
use camino::Utf8Path;
use manifest::WorkspaceManifest;
use std::collections::BTreeSet;
use std::io::{self, IsTerminal, Write};
use std::{fs, iter};

mod custom;
mod defaults;
#[cfg(test)]
mod tests;
mod ui;
pub(crate) mod unattended;

pub(crate) fn setup(cwd: &Utf8Path) -> Result<Option<WorkspaceManifest>> {
    let installed: Vec<_> = agents::profiles()
        .filter(|p| util::find_on_path(p.binary).is_some())
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
        store::parse_yaml(include_str!("onboard/modes.yaml"))?,
        false,
    )?;
    let config = spec::load_project_config_from(cwd)?;
    let generated = defaults::manifest(&installed)?;
    let entry = generated.entry.clone();
    let mut tiers = SetupTiers::new(generated, &config)?;
    let mut entries = ui::Options::new(
        store::parse_yaml(include_str!("onboard/leads.yaml"))?,
        false,
    )?;
    let rows: Vec<(String, String)> = store::parse_yaml(defaults::SKILLS)?;
    let mut starters = ui::Options::new(rows.clone(), true)?;
    let mut confirmation = ui::Options::new(
        vec![
            ("Continue".into(), "Accept, write manifest and start".into()),
            (
                "Cancel".into(),
                "Leave setup without writing a manifest".into(),
            ),
        ],
        false,
    )?;
    let mut custom_text = String::new();
    enum EntrySkill {
        Named(skills::SkillName),
        Custom(skills::reference::GithubRef),
    }
    impl EntrySkill {
        fn name(&self) -> &skills::SkillName {
            match self {
                Self::Named(name) => name,
                Self::Custom(reference) => &reference.name,
            }
        }
    }
    let mut entry_skill = EntrySkill::Named(entry);
    enum Step {
        Mode,
        Entry,
        Custom,
        Tiers,
        Starters,
        Final,
    }
    let mut step = Step::Mode;
    loop {
        use ui::Action::{Back, Next, Quit};
        step = match step {
            Step::Mode => match modes.pick(&header, "How do you work?")? {
                Back | Quit => return Ok(None),
                Next if modes.chosen().0 == "lead" => Step::Entry,
                Next => {
                    entry_skill = EntrySkill::Named(modes.chosen().0.parse()?);
                    Step::Tiers
                }
            },
            Step::Entry => match entries.pick(&header, "Choose the lead's entry skill")? {
                Quit => return Ok(None),
                Back => Step::Mode,
                Next if entries.chosen().0 == "Custom" => Step::Custom,
                Next => {
                    entry_skill = EntrySkill::Named(entries.chosen().0.parse()?);
                    Step::Tiers
                }
            },
            Step::Custom => match custom::input(&mut custom_text, &header, cwd, &global)? {
                custom::Choice::Quit => return Ok(None),
                custom::Choice::Back => Step::Entry,
                custom::Choice::Next(reference) => {
                    entry_skill = EntrySkill::Custom(reference);
                    Step::Tiers
                }
            },
            Step::Tiers => match tiers.pick(cwd, &config)? {
                SetupAction::Quit => return Ok(None),
                SetupAction::Back => {
                    if modes.chosen().0 != "lead" {
                        Step::Mode
                    } else if entries.chosen().0 == "Custom" {
                        Step::Custom
                    } else {
                        Step::Entry
                    }
                }
                SetupAction::Next => Step::Starters,
            },
            Step::Starters => {
                starters.replace_rows(
                    rows.iter()
                        .filter(|(name, _)| name != entry_skill.name().as_str())
                        .cloned()
                        .collect(),
                )?;
                let question = format!(
                    "Optional starter skills — instructions that give agents a role. None selected by default.\n\
                     Missing selections and the entry skill are fetched from {} at latest (Git HEAD). Existing skills are kept.",
                    defaults::REPOSITORY,
                );
                match starters.pick(&header, &question)? {
                    Quit => return Ok(None),
                    Back => Step::Tiers,
                    Next => Step::Final,
                }
            }
            Step::Final => {
                let manifest = tiers.draft();
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
                let source = match &entry_skill {
                    EntrySkill::Custom(reference) => format!("\nCustom URL: {reference}"),
                    EntrySkill::Named(_) => String::new(),
                };
                let summary = format!(
                    "Warning: Workers bypass approval prompts and run without a sandbox. See https://github.com/Rhovian/niles/blob/main/docs/security.md for details.\n\nProject: {cwd}\nEntry skill: {}{source}\nEntry agent: {}\n{tiers}\nOptional skills: {}\n\nEnter writes the manifest and starts.",
                    entry_skill.name(),
                    manifest.entry_agent,
                    starters.selected().collect::<Vec<_>>().join(", ")
                );
                match ui::confirm(&mut confirmation, &header, &summary)? {
                    Quit => return Ok(None),
                    Back => Step::Starters,
                    Next => break,
                }
            }
        };
    }
    let mut manifest = tiers.draft().clone();
    manifest.entry = entry_skill.name().clone();
    let starter_skills = starters
        .selected()
        .map(str::parse)
        .collect::<Result<Vec<skills::SkillName>>>()?;
    let finish = || {
        finish(cwd, &global, &manifest, &starter_skills, &config.models)?;
        manifest::load(cwd, None)
    };
    match entry_skill {
        EntrySkill::Custom(reference) => custom::install(reference, cwd, &global, finish),
        EntrySkill::Named(_) => finish(),
    }
}
enum ManifestWrite {
    Written,
    Existing,
}

fn finish(
    cwd: &Utf8Path,
    global: &Utf8Path,
    manifest: &WorkspaceManifest,
    starter_skills: &[skills::SkillName],
    models: &ModelRoster,
) -> Result<ManifestWrite> {
    manifest::check(manifest, models)?;
    let body = serde_saphyr::to_string(manifest)?;
    let names: BTreeSet<_> = manifest
        .bindings
        .iter()
        .flat_map(|(name, binding)| iter::once(name).chain(binding.attached_skills()))
        .chain(starter_skills)
        .chain(iter::once(&manifest.entry))
        .collect();
    for name in names {
        if skills::find(name, cwd, global)?.is_none() {
            println!("fetching {name} from latest");
            skills::import(
                defaults::reference(name)?,
                skills::Origin::Global,
                skills::ImportMode::Replace,
            )?;
        }
    }
    let path = manifest::manifest_path(cwd, None)?;
    fs::create_dir_all(path.parent().context("manifest has no parent")?)?;
    match store::publish(&path, Publication::Create, |file| {
        file.write_all(body.as_bytes())
    }) {
        Ok(_) => Ok(ManifestWrite::Written),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(ManifestWrite::Existing),
        Err(error) => Err(error.into()),
    }
}
pub(super) fn first_run(cwd: &Utf8Path) -> Result<Option<registry::Entry>> {
    if !io::stdin().is_terminal() || !registry::entries()?.is_empty() {
        return Ok(None);
    }
    ui::register(cwd)
}

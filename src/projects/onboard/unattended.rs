use crate::{
    agents::AgentSpec,
    skills::SkillName,
    tmux,
    workspace_manifest::{self, Tiers, WorkspaceManifest},
};
use anyhow::{Context, Result, ensure};
use clap::Args;

#[derive(Debug, Args)]
pub(crate) struct Options {
    /// Installed skill name, or a missing skill to fetch from Rhovian/niles.
    #[arg(long, value_name = "SKILL")]
    entry_skill: SkillName,
    /// Exact family:model:effort listed in --tiers.
    #[arg(long, value_name = "AGENT")]
    entry_agent: String,
    /// Manifest YAML: standard is required; each tier has primary and secondaries.
    #[arg(long, value_name = "YAML", value_parser = crate::store::parse_yaml::<Tiers>)]
    tiers: Tiers,
    /// Installed skills, or missing starter names from the bundled catalog.
    #[arg(long, value_name = "SKILL", value_delimiter = ',')]
    starter_skills: Vec<SkillName>,
    /// Register this project and launch its entry agent in a detached tmux session.
    #[arg(long, value_name = "PROJECT")]
    start: Option<String>,
}

pub(crate) fn run(options: Options) -> Result<()> {
    let cwd = crate::util::current_dir_utf8()?;
    let path = workspace_manifest::manifest_path(&cwd, None)?;
    ensure!(!path.try_exists()?, "manifest already exists: {path}");
    let config = crate::config::spec::load_project_config_from(&cwd)?;
    let mut manifest = WorkspaceManifest {
        entry: options.entry_skill,
        entry_agent: options.entry_agent,
        tiers: options.tiers,
        bindings: Default::default(),
        source: None,
        checkin: None,
        recheck: None,
        worktree: None,
    };
    manifest.tiers.dedup(&config.models)?;
    for agent in manifest.tiers.agents() {
        let spec = AgentSpec::parse(agent, &config.models)?;
        let binary = spec.profile().binary;
        ensure!(
            crate::util::find_on_path(binary).is_some(),
            "CLI not found on PATH: {binary}"
        );
    }
    let project = options
        .start
        .map(|name| {
            let name = super::registry::ProjectName::parse(&name)?;
            if let Some(existing) = super::registry::find(name.as_str())? {
                ensure!(
                    existing.path == cwd,
                    "project name is already registered elsewhere"
                );
            }
            if let super::register::Directory::Registered(existing) =
                super::register::directory("", &cwd)?
            {
                ensure!(
                    existing.name == name,
                    "project directory is already registered as {}",
                    existing.name.as_str()
                );
            }
            Ok::<_, anyhow::Error>(name)
        })
        .transpose()?;
    if project.is_some() {
        ensure!(
            crate::util::find_on_path("tmux").is_some(),
            "CLI not found on PATH: tmux"
        );
    }
    let written = super::finish(
        &cwd,
        &crate::skills::global_dir()?,
        &manifest,
        &options.starter_skills,
        &config.models,
    )?;
    ensure!(
        matches!(written, super::ManifestWrite::Written),
        "manifest already exists: {path}"
    );
    workspace_manifest::load(&cwd, None)?.context("setup did not produce a manifest")?;
    println!("manifest: {path}");
    if let Some(name) = project {
        let entry = match super::registry::find(name.as_str())? {
            Some(entry) => entry,
            None => super::register::register(name.as_str(), &cwd)?,
        };
        super::super::lifecycle::start_entry(
            &entry,
            tmux::LeadStart::Unattended,
            &crate::theme::Theme::load()?,
        )?;
        println!("entry session: {} (open with niles)", name.as_str());
    }
    Ok(())
}

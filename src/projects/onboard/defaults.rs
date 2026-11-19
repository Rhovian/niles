use crate::skills::SkillName;
use crate::skills::reference::GithubRef;
use crate::store;
use crate::workspace_manifest::{TierBinding, Tiers, WorkspaceManifest};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const REPOSITORY: &str = "github.com/Rhovian/niles";
pub(super) const SKILLS: &str = include_str!("skills.yaml");
pub(super) fn fetchable_names() -> Result<BTreeSet<SkillName>> {
    let mut names = BTreeSet::new();
    for source in [SKILLS, include_str!("modes.yaml")] {
        let rows: Vec<(SkillName, String)> = store::parse_yaml(source)?;
        names.extend(rows.into_iter().map(|(name, _)| name));
    }
    Ok(names)
}
pub(super) fn reference(name: &SkillName) -> Result<GithubRef> {
    format!("{REPOSITORY}/skills/{name}").parse()
}
pub(super) fn manifest(installed: &[&str]) -> Result<WorkspaceManifest> {
    let mut agents = installed.iter().map(|family| (*family).to_owned());
    let standard = TierBinding {
        primary: agents.next().context("no agent installed")?,
        secondaries: agents.collect(),
    };
    Ok(WorkspaceManifest {
        bindings: BTreeMap::new(),
        // Onboarding writes the lead's agent out as an explicit, editable choice.
        entry_agent: standard.primary.clone(),
        tiers: Tiers {
            standard,
            frontier: None,
            economy: None,
            mechanical: None,
        },
        entry: "lead".parse()?,
        source: None,
        checkin: None,
        recheck: None,
        worktree: None,
    })
}

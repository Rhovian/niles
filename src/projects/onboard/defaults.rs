use crate::agents::roster::Tier::{self, Economy, Frontier, Mechanical, Standard};
use crate::skills::{SkillName, reference::GithubRef};
use crate::workspace_manifest::{TierBinding, Tiers, WorkspaceManifest};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
pub(super) const REPOSITORY: &str = "github.com/Rhovian/niles";
pub(super) const RELEASE: &str = concat!("v", env!("CARGO_PKG_VERSION"));
pub(super) const SKILLS: &str = include_str!("skills.yaml");
pub(super) fn reference(name: &SkillName) -> Result<GithubRef> {
    format!("{REPOSITORY}/skills/{name}@{RELEASE}").parse()
}
pub(super) fn manifest(installed: &[&str]) -> Result<WorkspaceManifest> {
    let families: Vec<(String, BTreeMap<Tier, String>)> =
        crate::store::parse_yaml(include_str!("tiers.yaml"))?;
    let tier = |tier| {
        let mut agents = families
            .iter()
            .filter(|(family, _)| installed.contains(&family.as_str()))
            .filter_map(|(_, agents)| agents.get(&tier))
            .cloned();
        Some(TierBinding {
            primary: agents.next()?,
            secondaries: agents.collect(),
        })
    };
    let standard = tier(Standard).context("no standard agent installed")?;
    Ok(WorkspaceManifest {
        bindings: BTreeMap::new(),
        // Onboarding writes the lead's agent out as an explicit, editable choice.
        entry_agent: standard.primary.clone(),
        tiers: Tiers {
            standard,
            frontier: tier(Frontier),
            economy: tier(Economy),
            mechanical: tier(Mechanical),
        },
        entry: "lead".parse()?,
        source: None,
        checkin: None,
        recheck: None,
        worktree: None,
    })
}

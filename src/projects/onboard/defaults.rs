use crate::agents::roster::Tier::{self, Economy, Frontier, Mechanical, Standard};
use crate::skills::{SkillName, reference::GithubRef};
use crate::workspace_manifest::{Binding, TierBinding, WorkspaceManifest};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
pub(super) const REPOSITORY: &str = "github.com/Rhovian/niles";
pub(super) const RELEASE: &str = concat!("v", env!("CARGO_PKG_VERSION"));
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    tiers: Vec<Tier>,
    families: Vec<(String, BTreeMap<Tier, String>)>,
}
pub(super) fn reference(name: &SkillName) -> Result<GithubRef> {
    format!("{REPOSITORY}/skills/{name}@{RELEASE}").parse()
}
pub(super) fn manifest(installed: &[&str]) -> Result<WorkspaceManifest> {
    let policies: BTreeMap<SkillName, Policy> =
        crate::store::parse_yaml(include_str!("skills.yaml"))?;
    let mut bindings = BTreeMap::new();
    for (skill, policy) in policies {
        let tier = |tier| {
            if !policy.tiers.contains(&tier) {
                return None;
            }
            let mut agents = policy
                .families
                .iter()
                .filter(|(family, _)| installed.contains(&family.as_str()))
                .filter_map(|(_, agents)| agents.get(&tier))
                .cloned();
            Some(TierBinding {
                primary: agents.next()?,
                secondaries: agents.collect(),
            })
        };
        let mut binding = Binding::from(tier(Standard).context("no standard agent installed")?);
        binding.frontier = tier(Frontier);
        binding.economy = tier(Economy);
        binding.mechanical = tier(Mechanical);
        bindings.insert(skill, binding);
    }
    Ok(WorkspaceManifest {
        bindings,
        entry: "lead".parse()?,
        source: None,
        checkin: None,
        recheck: None,
        worktree: None,
    })
}

use crate::agents::{
    AgentSpec, ModelRoster,
    roster::Tier::{self, Economy, Frontier, Mechanical, Standard},
};
use crate::skills::{SkillName, reference::GithubRef};
use crate::workspace_manifest::{TierBinding, Tiers, WorkspaceManifest};
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet};
pub(super) const REPOSITORY: &str = "github.com/Rhovian/niles";
pub(super) const SKILLS: &str = include_str!("skills.yaml");
pub(super) fn fetchable_names() -> Result<BTreeSet<SkillName>> {
    let mut names = BTreeSet::new();
    for source in [SKILLS, include_str!("modes.yaml")] {
        let rows: Vec<(SkillName, String)> = crate::store::parse_yaml(source)?;
        names.extend(rows.into_iter().map(|(name, _)| name));
    }
    Ok(names)
}
pub(super) fn reference(name: &SkillName) -> Result<GithubRef> {
    format!("{REPOSITORY}/skills/{name}").parse()
}
pub(super) fn manifest(installed: &[&str]) -> Result<WorkspaceManifest> {
    let ordered = candidates(include_str!("tiers.yaml"), &ModelRoster::builtin()?)?;
    let tier = |tier| {
        let mut agents = ordered
            .get(&tier)?
            .iter()
            .filter(|(_, spec)| installed.contains(&spec.family()))
            .map(|(agent, _)| agent.clone());
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

fn candidates(
    data: &str,
    roster: &ModelRoster,
) -> Result<BTreeMap<Tier, Vec<(String, AgentSpec)>>> {
    let ordered: BTreeMap<Tier, Vec<String>> = crate::store::parse_yaml(data)?;
    let mut parsed = BTreeMap::new();
    for (tier, agents) in ordered {
        let mut candidates = Vec::new();
        for agent in agents {
            let spec = AgentSpec::parse(&agent, roster)?;
            let entry = roster.entry(
                spec.family(),
                spec.model().context("setup agent needs a model")?,
                spec.effort(),
            )?;
            ensure!(
                entry.tier == tier,
                "setup agent {agent} has the wrong roster tier"
            );
            candidates.push((agent, spec));
        }
        parsed.insert(tier, candidates);
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_candidates_require_roster_existence_and_matching_tier() {
        let roster = ModelRoster::builtin().unwrap();
        let error = candidates("standard: [claude:opus:high]", &roster).unwrap_err();
        assert!(error.to_string().contains("wrong roster tier"));
        let error = candidates("standard: [claude:unknown:high]", &roster).unwrap_err();
        assert!(error.to_string().contains("no registry metadata"));
    }
}

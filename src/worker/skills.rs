use crate::skills::{self, Skill, SkillName};
use crate::util::current_dir_utf8;
use crate::workspace_manifest::Binding;
use anyhow::{Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use std::collections::HashSet;
use std::iter;

const MAX_STAGE_BYTES: usize = 32_768;

pub(super) struct LaunchSkills {
    pub prompt: String,
    pub provenance: String,
}

pub(super) fn prepare(
    primary: &Skill,
    binding: &Binding,
    with: &[SkillName],
    workspace: &Utf8Path,
) -> Result<LaunchSkills> {
    let global = skills::global_dir()?;
    let mut provenance = String::new();
    let mut resolve = |kind, names: &[SkillName]| -> Result<Vec<Skill>> {
        let skills = names
            .iter()
            .map(|name| skills::resolve(name, workspace, &global))
            .collect::<Result<Vec<_>>>()?;
        for skill in &skills {
            provenance.push_str(&format!("{kind}: {skill}\n"));
        }
        Ok(skills)
    };
    let always = resolve("always_skill", &binding.always)?;
    let on_stage = resolve("on_stage_skill", &binding.on_stage)?;
    let with = resolve("with_skill", with)?;
    check_stage_size(&on_stage)?;
    for gate in binding.gates.iter().flat_map(|gates| &gates.0.post) {
        provenance.push_str(&format!(
            "gate: {} run {}\n",
            gate.name,
            serde_json::to_string(&gate.run)?
        ));
    }
    let mut prompt = String::new();
    let mut seen = HashSet::new();
    for skill in iter::once(primary)
        .chain(&always)
        .chain(&with)
        .filter(|skill| seen.insert(skill.name.clone()))
    {
        prompt.push_str(&format!(
            "## Skill: {}\n{}\n",
            skill.name,
            skill.primary_body()?
        ));
    }
    Ok(LaunchSkills { prompt, provenance })
}

fn check_stage_size(skills: &[Skill]) -> Result<()> {
    ensure!(
        skills
            .iter()
            .try_fold(MAX_STAGE_BYTES, |remaining, skill| remaining
                .checked_sub(skill.body.len()))
            .is_some(),
        "on_stage bodies exceed {MAX_STAGE_BYTES} bytes"
    );
    Ok(())
}

pub(super) fn stage_skills(names: &[SkillName], workspace: &Utf8Path) -> Result<Vec<Skill>> {
    let global = skills::global_dir()?;
    let skills = names
        .iter()
        .map(|name| skills::resolve(name, workspace, &global))
        .collect::<Result<Vec<_>>>()?;
    check_stage_size(&skills)?;
    Ok(skills)
}

pub(crate) fn skill_workspace() -> Result<Utf8PathBuf> {
    match super::meta::worker_dir_from_env_if_set()? {
        Some(dir) => Ok(super::meta::read_meta(&dir)?.project),
        None => current_dir_utf8(),
    }
}

pub(super) fn load(name: &SkillName) -> Result<()> {
    let skill = skills::resolve(name, &skill_workspace()?, &skills::global_dir()?)?;
    println!("skill: {skill}");
    print!("{}", skill.body);
    Ok(())
}

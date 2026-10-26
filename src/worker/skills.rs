use std::collections::HashSet;

use anyhow::{Result, ensure};
use camino::Utf8Path;

use crate::{
    skills::{self, Skill, SkillName, Source},
    workspace_manifest::Binding,
};

const MAX_MENU: usize = 16;
const MAX_STAGE_BYTES: usize = 32_768;

pub(super) struct LaunchSkills {
    pub names: Vec<SkillName>,
    pub prompt: String,
    pub menu: String,
    pub provenance: String,
    pub warnings: String,
}

pub(super) fn prepare(
    primary: &Skill,
    binding: &Binding,
    menu: &[SkillName],
    with: &[SkillName],
    from: bool,
    workspace: &Utf8Path,
) -> Result<LaunchSkills> {
    let mut names = binding.skills.clone();
    names.extend_from_slice(menu);
    let mut seen = HashSet::new();
    names.retain(|name| seen.insert(name.clone()));
    ensure!(
        names.len() <= MAX_MENU,
        "menu has {} skills; maximum is {MAX_MENU}",
        names.len()
    );
    let global = skills::global_dir()?;
    let mut provenance = String::new();
    let mut resolve = |kind, names: &[SkillName]| {
        names
            .iter()
            .map(|name| {
                let skill = skills::resolve(name, workspace, &global)?;
                provenance.push_str(&format!("{kind}: {skill}\n"));
                Ok(skill)
            })
            .collect::<Result<Vec<_>>>()
    };
    let menu_skills = resolve("menu_skill", &names)?;
    for skill in &menu_skills {
        ensure!(
            !matches!(skill.source, Source::Builtin(_)),
            "menu skill {skill} needs a disk SKILL.md"
        );
    }
    let always = resolve("always_skill", &binding.always)?;
    let on_from = resolve("on_from_skill", &binding.on_from)?;
    let on_stage = resolve("on_stage_skill", &binding.on_stage)?;
    let with = resolve("with_skill", with)?;
    ensure!(
        on_stage.iter().map(|skill| skill.body.len()).sum::<usize>() <= MAX_STAGE_BYTES,
        "on_stage bodies exceed {MAX_STAGE_BYTES} bytes"
    );
    let mut menu = String::new();
    let mut warnings = String::new();
    if !menu_skills.is_empty() {
        menu.push_str("\n\n## Skills\nRead a skill when its description applies to your task (`niles agent skill <name>`, or the path):\n");
    }
    for skill in &menu_skills {
        if !skill
            .description
            .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
            .any(|word| word.eq_ignore_ascii_case("when"))
        {
            warnings.push_str(&format!(
                "warning: menu skill {} description lacks the whole word 'when' (when to use it)\n",
                skill.name
            ));
        }
        menu.push_str(&format!("- {}\n", menu_row(skill)));
    }
    let mut prompt = String::new();
    seen.clear();
    for skill in std::iter::once(primary)
        .chain(&always)
        .chain(on_from.iter().filter(|_| from))
        .chain(&with)
        .filter(|skill| seen.insert(skill.name.clone()))
    {
        prompt.push_str(&format!(
            "## Skill: {}\n{}\n",
            skill.name,
            skill.primary_body()?
        ));
    }
    prompt.push_str(include_str!("../../prompts/skill_common.md"));
    Ok(LaunchSkills {
        names,
        prompt,
        menu,
        provenance,
        warnings,
    })
}

fn menu_row(skill: &Skill) -> String {
    let description = skills::display_description(&skill.description);
    format!("{}: {description} ({})", skill.name, skill.source)
}

pub(super) fn discover(name: Option<&SkillName>) -> Result<()> {
    let dir = super::meta::worker_dir_from_env()?;
    let meta = super::meta::read_meta(&dir)?;
    let global = skills::global_dir()?;
    match name {
        None => {
            for name in &meta.menu {
                let skill = skills::resolve(name, &meta.project, &global)?;
                println!("{}", menu_row(&skill));
            }
        }
        Some(name) => {
            let valid = meta
                .menu
                .iter()
                .chain(&meta.always)
                .chain(&meta.on_from)
                .chain(&meta.on_stage)
                .chain(&meta.with)
                .collect::<Vec<_>>();
            ensure!(
                valid.contains(&name),
                "skill {name} is not available; valid skills: {}",
                valid
                    .iter()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let skill = skills::resolve(name, &meta.project, &global)?;
            println!("skill: {skill}");
            print!("{}", skill.body);
        }
    }
    Ok(())
}

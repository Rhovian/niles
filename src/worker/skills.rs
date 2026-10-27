use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, ensure};
use camino::Utf8Path;

use crate::{
    skills::{
        self, Skill, SkillName,
        reference::{MenuItem, PinnedRef},
    },
    workspace_manifest::Binding,
};

const MAX_MENU: usize = 16;
const MAX_STAGE_BYTES: usize = 32_768;

pub(super) struct LaunchSkills {
    pub names: Vec<SkillName>,
    pub imports: BTreeMap<SkillName, PinnedRef>,
    pub prompt: String,
    pub menu: String,
    pub provenance: String,
    pub warnings: String,
}

pub(super) fn prepare(
    primary: &Skill,
    binding: &Binding,
    menu: &[MenuItem],
    with: &[SkillName],
    from: bool,
    workspace: &Utf8Path,
) -> Result<LaunchSkills> {
    let mut imports = BTreeMap::new();
    let mut names = binding.skills.clone();
    for item in menu {
        let name = item.name();
        if let MenuItem::Ref(reference) = item {
            let occupied = std::iter::once(&primary.name)
                .chain(&binding.skills)
                .chain(&binding.always)
                .chain(&binding.on_from)
                .chain(&binding.on_stage)
                .chain(with)
                .any(|other| other == name);
            ensure!(
                !occupied && menu.iter().filter(|other| other.name() == name).count() == 1,
                "ephemeral skill name {name} collides with another spawn skill"
            );
            imports.insert(name.clone(), reference.clone());
        }
        names.push(name.clone());
    }
    let mut seen = HashSet::new();
    names.retain(|name| seen.insert(name.clone()));
    ensure!(
        names.len() <= MAX_MENU,
        "menu has {} skills; maximum is {MAX_MENU}",
        names.len()
    );
    let global = skills::global_dir()?;
    let imports: BTreeMap<_, _> = imports
        .into_iter()
        .map(|(name, reference)| Ok((name, skills::pin(reference)?)))
        .collect::<Result<_>>()?;
    let mut provenance = String::new();
    let mut resolve = |kind, names: &[SkillName]| -> Result<Vec<Skill>> {
        let skills = names
            .iter()
            .map(|name| resolve_available(name, &imports, workspace, &global))
            .collect::<Result<Vec<_>>>()?;
        for skill in &skills {
            provenance.push_str(&format!("{kind}: {skill}\n"));
        }
        Ok(skills)
    };
    let menu_skills = resolve("menu_skill", &names)?;
    let always = resolve("always_skill", &binding.always)?;
    let on_from = resolve("on_from_skill", &binding.on_from)?;
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
    prompt.push_str(include_str!("../../skills/skill_common.md"));
    Ok(LaunchSkills {
        names,
        imports,
        prompt,
        menu,
        provenance,
        warnings,
    })
}

fn resolve_available(
    name: &SkillName,
    imports: &BTreeMap<SkillName, PinnedRef>,
    workspace: &Utf8Path,
    global: &Utf8Path,
) -> Result<Skill> {
    match imports.get(name) {
        Some(reference) => skills::imported(reference, global),
        None => skills::resolve(name, workspace, global),
    }
}

fn check_stage_size(skills: &[Skill]) -> Result<()> {
    ensure!(
        skills.iter().map(|skill| skill.body.len()).sum::<usize>() <= MAX_STAGE_BYTES,
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
                let skill = resolve_available(name, &meta.imports, &meta.project, &global)?;
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
            let skill = resolve_available(name, &meta.imports, &meta.project, &global)?;
            println!("skill: {skill}");
            print!("{}", skill.body);
        }
    }
    Ok(())
}

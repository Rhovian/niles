use super::{Origin, SkillName, builtin::SKILLS};
use anyhow::{Context, Result, bail, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use std::fmt;

#[derive(Debug)]
pub(crate) enum Source {
    Builtin(&'static str),
    Disk(Origin, Utf8PathBuf),
}

#[derive(Debug)]
pub(crate) struct Skill {
    pub name: SkillName,
    pub description: String,
    pub body: String,
    pub source: Source,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin(_) => f.write_str("built-in"),
            Self::Disk(origin, dir) => write!(f, "{origin} {}", dir.join("SKILL.md")),
        }
    }
}
impl fmt::Display for Skill {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.source)
    }
}
impl Skill {
    pub fn primary_body(&self) -> Result<&str> {
        ensure!(
            !self.body.trim().is_empty(),
            "primary skill {self} has an empty body ({} bytes)",
            self.body.len()
        );
        Ok(&self.body)
    }
}

pub(crate) fn global_dir() -> Result<Utf8PathBuf> {
    Ok(crate::util::home_dir()?.join(".niles"))
}

pub(crate) fn locate(name: &SkillName, workspace: &Utf8Path, global: &Utf8Path) -> Result<Source> {
    let builtin = SKILLS
        .iter()
        .find(|(key, _)| *key == name.as_str())
        .map(|(_, text)| Source::Builtin(text));
    let roots = [
        builtin,
        Some(Source::Disk(
            Origin::Global,
            global.join("skills").join(name.as_str()),
        )),
        Some(Source::Disk(
            Origin::Workspace,
            workspace.join(".niles/skills").join(name.as_str()),
        )),
    ];
    for source in roots.into_iter().flatten().rev() {
        if let Source::Disk(origin, dir) = &source
            && !super::files::exists(&dir.join("SKILL.md"), *origin)
                .with_context(|| format!("invalid skill {name} ({source})"))?
        {
            continue;
        }
        return Ok(source);
    }
    bail!(
        "skill `{name}` has no SKILL.md: built-ins are {}; put one at {global}/skills/{name}/ or {workspace}/.niles/skills/{name}/",
        SKILLS
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(crate) fn resolve(name: &SkillName, workspace: &Utf8Path, global: &Utf8Path) -> Result<Skill> {
    let source = locate(name, workspace, global)?;
    let text = match &source {
        Source::Builtin(text) => (*text).to_owned(),
        Source::Disk(origin, dir) => super::files::read(&dir.join("SKILL.md"), *origin)?,
    };
    let (description, body) =
        parse(name, &text).with_context(|| format!("invalid skill {name} ({source})"))?;
    Ok(Skill {
        name: name.clone(),
        description,
        body,
        source,
    })
}

// serde-saphyr 1.3.0 parses YAML documents, not Markdown frontmatter
// (src/lib.rs from_str_with_options). Delimit only the YAML; retain body bytes.
fn parse(name: &SkillName, text: &str) -> Result<(String, String)> {
    let text = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))
        .context("frontmatter must start with --- on its own line")?;
    let mut end = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            #[derive(Deserialize)]
            struct Header {
                name: String,
                description: String,
            }
            // Never include parser errors: serde type errors can quote repository text.
            let header: Header = crate::store::parse_yaml(&text[..end]).map_err(|_| {
                anyhow::anyhow!("frontmatter requires valid YAML with name and description strings")
            })?;
            ensure!(
                header.name == name.as_str(),
                "frontmatter name must equal its directory name"
            );
            let count = header.description.chars().count();
            ensure!(
                !header.description.trim().is_empty() && count <= 1024,
                "frontmatter description must be nonempty and at most 1024 characters (got {count})"
            );
            return Ok((header.description, text[end + line.len()..].to_owned()));
        }
        end += line.len();
    }
    bail!("frontmatter is missing its closing --- line")
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;

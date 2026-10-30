use super::{Origin, SkillName, reference::PinnedRef};
use anyhow::{Context, Result, bail, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use std::fmt;

#[derive(Debug)]
pub(crate) enum Source {
    Local {
        origin: Origin,
        path: Utf8PathBuf,
    },
    Import {
        reference: PinnedRef,
        path: Utf8PathBuf,
    },
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
            Self::Local { origin, path } => write!(f, "{origin} {path}"),
            Self::Import { reference, path } => {
                write!(f, "import {} {path}", reference.short())
            }
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

pub(crate) fn find(
    name: &SkillName,
    workspace: &Utf8Path,
    global: &Utf8Path,
) -> Result<Option<Source>> {
    for (origin, dir) in [
        (
            Origin::Workspace,
            workspace.join(".niles/skills").join(name.as_str()),
        ),
        (Origin::Global, global.join("skills").join(name.as_str())),
    ] {
        let markdown = dir.join("SKILL.md");
        let pointer = dir.join("SKILL.ref");
        let md = super::files::exists(&markdown, origin)?;
        let rf = super::files::exists(&pointer, origin)?;
        ensure!(!(md && rf), "skill has both {markdown} and {pointer}");
        if md {
            return Ok(Some(Source::Local {
                origin,
                path: markdown,
            }));
        }
        if rf {
            let text = super::files::read(&pointer, origin)?;
            let reference: PinnedRef =
                PinnedRef::document(&text).with_context(|| format!("invalid {pointer}"))?;
            ensure!(
                &reference.reference().name == name,
                "skill ref name must equal its directory name: {pointer}"
            );
            let path = super::cache::directory(&reference, global).join("SKILL.md");
            return Ok(Some(Source::Import { reference, path }));
        }
    }
    Ok(None)
}

pub(crate) fn locate(name: &SkillName, workspace: &Utf8Path, global: &Utf8Path) -> Result<Source> {
    find(name, workspace, global)?.with_context(|| format!(
        "skill `{name}` has no SKILL.md or SKILL.ref: put one at {global}/skills/{name}/ or {workspace}/.niles/skills/{name}/, or pin one with `niles skills import <owner>/<repo>/<path>`"
    ))
}

pub(crate) fn resolve(name: &SkillName, workspace: &Utf8Path, global: &Utf8Path) -> Result<Skill> {
    let source = locate(name, workspace, global)?;
    read(name, source, global)
}

pub(crate) fn imported(reference: &PinnedRef, global: &Utf8Path) -> Result<Skill> {
    let path = super::cache::directory(reference, global).join("SKILL.md");
    read(
        &reference.reference().name,
        Source::Import {
            reference: reference.clone(),
            path,
        },
        global,
    )
}

fn read(name: &SkillName, source: Source, global: &Utf8Path) -> Result<Skill> {
    let text = match &source {
        Source::Local { origin, path } => super::files::read(path, *origin)?,
        Source::Import { reference, path } => {
            super::cache::fetch(reference, global)?;
            super::files::read(path, Origin::Workspace)?
        }
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

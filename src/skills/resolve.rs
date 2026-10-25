use super::{Origin, SkillName, builtin::SKILLS};
use anyhow::{Context, Result, bail, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use std::{
    fmt, fs,
    io::{ErrorKind, Read},
    os::unix::fs::OpenOptionsExt,
};

// Includes frontmatter headroom beyond the 64 KiB assembled-primary budget.
const MAX_SKILL_BYTES: usize = 131_072;

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
            Self::Disk(origin, dir) => {
                let origin = match origin {
                    Origin::Global => "global",
                    Origin::Workspace => "workspace",
                };
                write!(f, "{origin} {}", dir.join("SKILL.md"))
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
        if let Source::Disk(origin, dir) = &source {
            let path = dir.join("SKILL.md");
            if *origin == Origin::Workspace {
                match fs::symlink_metadata(dir) {
                    Ok(meta) => ensure!(
                        meta.is_dir(),
                        "skill directory must be a directory without symlinks: {dir}"
                    ),
                    Err(err) if err.kind() == ErrorKind::NotFound => continue,
                    Err(err) => {
                        return Err(err).with_context(|| format!("failed to inspect {dir}"));
                    }
                }
            }
            match fs::symlink_metadata(&path) {
                Ok(meta) => ensure!(
                    *origin != Origin::Workspace || meta.is_file(),
                    "skill must be a regular file without symlinks: {path}"
                ),
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(err) => return Err(err).with_context(|| format!("failed to inspect {path}")),
            }
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
        Source::Disk(origin, dir) => {
            let path = dir.join("SKILL.md");
            let flags = if *origin == Origin::Workspace {
                libc::O_NOFOLLOW
            } else {
                0
            };
            let file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | flags)
                .open(&path)
                .with_context(|| format!("failed to open {path}"))?;
            let metadata = file
                .metadata()
                .with_context(|| format!("failed to inspect {path}"))?;
            ensure!(metadata.is_file(), "skill must be a regular file: {path}");
            let mut bytes = Vec::new();
            file.take((MAX_SKILL_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .with_context(|| format!("failed to read {path}"))?;
            ensure!(
                bytes.len() <= MAX_SKILL_BYTES,
                "skill file {path} read {} bytes (file size {} bytes); maximum is {MAX_SKILL_BYTES} bytes",
                bytes.len(),
                metadata.len()
            );
            String::from_utf8(bytes).with_context(|| format!("skill file is not UTF-8: {path}"))?
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

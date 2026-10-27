use super::SkillName;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GithubRef {
    pub owner: String,
    pub repo: String,
    pub path: String,
    pub rev: String,
    pub name: SkillName,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct PinnedRef(GithubRef);

#[derive(Clone, Debug)]
pub(crate) enum MenuItem {
    Name(SkillName),
    Ref(GithubRef),
}

impl MenuItem {
    pub fn name(&self) -> &SkillName {
        match self {
            Self::Name(name) => name,
            Self::Ref(reference) => &reference.name,
        }
    }
}
impl FromStr for MenuItem {
    type Err = anyhow::Error;
    fn from_str(text: &str) -> Result<Self> {
        if text.contains('/') {
            Ok(Self::Ref(text.parse()?))
        } else {
            Ok(Self::Name(text.parse()?))
        }
    }
}

impl FromStr for GithubRef {
    type Err = anyhow::Error;
    // Git 2.50.1 Documentation/git-check-ref-format.adoc defines Git ref syntax,
    // not this host/repository/skill grammar; parse it once before using Git argv.
    fn from_str(text: &str) -> Result<Self> {
        ensure!(
            !text.chars().any(|ch| ch.is_control() || ch == '\\'),
            "skill ref contains control characters or backslashes"
        );
        let (location, rev) = match text.split_once('@') {
            Some((location, rev)) => (location, rev),
            None => (text, "HEAD"),
        };
        ensure!(!rev.is_empty(), "skill revision must be nonempty");
        let mut segments: Vec<_> = location.split('/').collect();
        if segments[0].contains('.') {
            ensure!(
                segments[0] == "github.com",
                "only github.com skill refs are supported"
            );
            segments.remove(0);
        }
        ensure!(segments.len() >= 2, "skill ref requires owner/repo");
        ensure!(
            segments
                .iter()
                .all(|segment| !segment.is_empty() && !matches!(*segment, "." | "..")),
            "skill ref segments must be nonempty and cannot be . or .."
        );
        for segment in &segments[..2] {
            ensure!(
                segment
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || b"-_.".contains(&ch)),
                "invalid GitHub owner or repository"
            );
        }
        let name = segments.last().context("skill ref has no name")?.parse()?;
        Ok(Self {
            owner: segments[0].to_owned(),
            repo: segments[1].to_owned(),
            path: segments[2..].join("/"),
            rev: rev.to_owned(),
            name,
        })
    }
}
impl fmt::Display for GithubRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "github.com/{}/{}", self.owner, self.repo)?;
        if !self.path.is_empty() {
            write!(f, "/{}", self.path)?;
        }
        write!(f, "@{}", self.rev)
    }
}
impl GithubRef {
    pub fn is_pinned(&self) -> bool {
        self.rev.len() == 40 && self.rev.bytes().all(|ch| ch.is_ascii_hexdigit())
    }
    pub fn with_sha(mut self, sha: String) -> Result<PinnedRef> {
        self.rev = sha;
        self.try_into()
    }
}
impl TryFrom<GithubRef> for PinnedRef {
    type Error = anyhow::Error;
    fn try_from(mut reference: GithubRef) -> Result<Self> {
        ensure!(
            reference.is_pinned(),
            "skill ref requires a full 40-hex SHA"
        );
        reference.rev.make_ascii_lowercase();
        Ok(Self(reference))
    }
}
impl PinnedRef {
    pub fn short(&self) -> String {
        let mut text = self.to_string();
        text.truncate(text.len() - (40 - 7));
        text
    }
    pub fn document(text: &str) -> Result<Self> {
        let line = match text
            .strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
        {
            Some(line) => line,
            None => text,
        };
        line.to_owned().try_into()
    }
    pub fn reference(&self) -> &GithubRef {
        &self.0
    }
}
impl fmt::Display for PinnedRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl TryFrom<String> for PinnedRef {
    type Error = anyhow::Error;
    fn try_from(text: String) -> Result<Self> {
        let reference: GithubRef = text.parse()?;
        // CLI input normalizes hexadecimal case; persisted pointers must already
        // be canonical, so reject uppercase explicitly before typed conversion.
        ensure!(
            !reference.rev.bytes().any(|ch| ch.is_ascii_uppercase()),
            "SKILL.ref requires a lowercase SHA"
        );
        let pinned = Self::try_from(reference)?;
        ensure!(
            text == pinned.to_string(),
            "SKILL.ref must use canonical github.com/owner/repo/path@sha form"
        );
        Ok(pinned)
    }
}
impl From<PinnedRef> for String {
    fn from(reference: PinnedRef) -> Self {
        reference.to_string()
    }
}

#[cfg(test)]
#[path = "reference_tests.rs"]
mod tests;

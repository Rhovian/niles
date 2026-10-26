use crate::workspace_manifest::setup::SetupSelection;
use std::{fmt, str::FromStr};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_general_category::{GeneralCategory, get_general_category};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) enum Origin {
    Global,
    Workspace,
}

pub(crate) const DEFAULT_ENTRY_SKILL: &str = "lead";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct SkillName(String);

impl SkillName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SkillName {
    fn default() -> Self {
        Self(DEFAULT_ENTRY_SKILL.to_owned())
    }
}

// serde_derive 1.0.229 src/de.rs:435 delegates try_from to this constructor;
// clap_builder 4.6.7 builder/value_parser.rs delegates FromStr. Neither enforces skill grammar.
impl TryFrom<String> for SkillName {
    type Error = anyhow::Error;
    fn try_from(value: String) -> Result<Self> {
        if value.is_empty()
            || !value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
        {
            bail!("invalid skill name {value:?}: expected ASCII letters, digits, '-' or '_'");
        }
        Ok(Self(value))
    }
}
impl FromStr for SkillName {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        value.to_owned().try_into()
    }
}
impl fmt::Display for SkillName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Workspace => "workspace",
        }
    }
}
impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl FromStr for Origin {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        [Self::Global, Self::Workspace]
            .into_iter()
            .find(|origin| origin.as_str() == value)
            .with_context(|| format!("invalid origin {value:?}: expected global or workspace"))
    }
}
impl TryFrom<String> for Origin {
    type Error = anyhow::Error;
    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}
impl From<Origin> for String {
    fn from(value: Origin) -> Self {
        value.as_str().to_owned()
    }
}

pub(crate) mod builtin;
pub(crate) mod files;
mod resolve;
pub(crate) use resolve::{Skill, Source, global_dir, locate, resolve};

pub(crate) fn display_description(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|ch| {
            !matches!(
                get_general_category(*ch),
                GeneralCategory::Control | GeneralCategory::Format
            )
        })
        .take(200)
        .collect::<String>()
}

pub(crate) fn list(selection: Option<&SetupSelection>) -> Result<()> {
    let workspace = crate::util::current_dir_utf8()?;
    let global = global_dir()?;
    let manifest = crate::workspace_manifest::load(&workspace, selection)?
        .ok_or_else(|| anyhow::anyhow!("skills requires a workspace manifest"))?;
    for (name, binding) in manifest.ordered_bindings() {
        let skill = resolve(name, &workspace, &global)?;
        let description = display_description(&skill.description);
        print!("skill: {skill}\t{description}");
        for (tier, agents) in binding.tiers() {
            print!("\t{}: {}", tier.as_str(), agents.primary);
        }
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_spelling_round_trips_through_serde_and_selection_parsing() {
        for origin in [Origin::Global, Origin::Workspace] {
            let text = origin.as_str();
            assert_eq!(text.parse::<Origin>().unwrap(), origin);
            let json = serde_json::to_string(&origin).unwrap();
            assert_eq!(json, format!("\"{text}\""));
            assert_eq!(serde_json::from_str::<Origin>(&json).unwrap(), origin);
        }
        for invalid in ["builtin", "Global", ""] {
            assert!(invalid.parse::<Origin>().is_err());
            assert!(serde_json::from_value::<Origin>(serde_json::json!(invalid)).is_err());
        }
    }

    #[test]
    fn names_validate_at_cli_and_serde_boundaries() {
        for name in ["a", "Review-2", "local_style"] {
            assert!(name.parse::<SkillName>().is_ok());
        }
        for name in ["", "../worker", "two words", "a:b", "é"] {
            assert!(name.parse::<SkillName>().is_err(), "{name}");
            assert!(serde_json::from_value::<SkillName>(serde_json::json!(name)).is_err());
        }
    }

    #[test]
    fn orchestration_does_not_name_builtin_skills() {
        fn scan(dir: &std::path::Path) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy();
                if name == "tests"
                    || name == "tests.rs"
                    || name.ends_with("_tests.rs")
                    || name == "test_support.rs"
                    || path.ends_with("skills/builtin.rs")
                {
                    continue;
                }
                if path.is_dir() {
                    scan(&path);
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                for line in source.split("\nmod tests {").next().unwrap().lines() {
                    // A worker directory and audit field are identities, not skill selection.
                    if (path.ends_with("store/paths.rs") && line.contains("const WORKERS_DIR"))
                        || (path.ends_with("worker/retier.rs") && line.contains("\"worker\": id"))
                    {
                        continue;
                    }
                    for name in "worker reviewer security design research solo".split_whitespace() {
                        let literal = format!("\"{name}\"");
                        assert!(!line.contains(&literal), "{path:?}: {line}");
                    }
                }
            }
        }
        scan(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
    }
}

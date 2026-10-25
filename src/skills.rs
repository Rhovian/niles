use std::{fmt, str::FromStr};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

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

pub(crate) mod builtin;

#[cfg(test)]
mod tests {
    use super::*;

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

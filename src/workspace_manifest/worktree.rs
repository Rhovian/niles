use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, de::Error};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeConfig {
    #[serde(default, deserialize_with = "environment")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
}

fn environment<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    let env = BTreeMap::<String, String>::deserialize(deserializer)?;
    // Serde 1.0.229 implements map deserialization, not process-environment constraints:
    // https://docs.rs/serde/1.0.229/src/serde/core/de/impls.rs.html
    for (key, value) in &env {
        let mut chars = key.chars();
        if !chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            || value.contains('\0')
        {
            return Err(D::Error::custom(
                "worktree env requires shell identifiers and NUL-free values",
            ));
        }
    }
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_schema_rejects_invalid_process_environment_and_unknown_fields() {
        for body in [
            "env: {'BAD-KEY': value}",
            "env: {'': value}",
            "env: {'9KEY': value}",
            "env: {'X; touch injected': value}",
            r#"env: {KEY: "nul\0value"}"#,
            "unknown: value",
        ] {
            assert!(
                crate::store::parse_yaml::<WorktreeConfig>(body).is_err(),
                "{body}"
            );
        }
        let config: WorktreeConfig = crate::store::parse_yaml("setup: 'echo ready'").unwrap();
        assert!(config.env.is_empty());
        assert_eq!(config.setup.as_deref(), Some("echo ready"));
        let config: WorktreeConfig = crate::store::parse_yaml("env: {_KEY_1: value}").unwrap();
        assert!(config.setup.is_none());
        assert_eq!(config.env["_KEY_1"], "value");
    }
}

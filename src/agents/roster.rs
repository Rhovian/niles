use std::{
    collections::{BTreeMap, BTreeSet},
    num::{NonZeroU32, NonZeroU64},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::families;

const BUILTIN_ROSTER: &str = include_str!("roster.yaml");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRoster {
    families: BTreeMap<String, BTreeMap<String, Model>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    efforts: Vec<String>,
    context_window: Option<u64>,
    #[serde(skip)]
    entries: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Frontier,
    Standard,
    Economy,
    Mechanical,
}

impl Tier {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Frontier => "frontier",
            Self::Standard => "standard",
            Self::Economy => "economy",
            Self::Mechanical => "mechanical",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    pub effort: Option<String>,
    pub tier: Tier,
    pub description: String,
    pub cost: Option<NonZeroU32>,
    pub speed: Option<NonZeroU32>,
    pub context_window: NonZeroU64,
}

type RawRegistry = BTreeMap<String, BTreeMap<String, Vec<RegistryEntry>>>;

#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub(crate) struct RawRoster(BTreeMap<String, BTreeMap<String, Model>>);

impl ModelRoster {
    pub(crate) fn builtin() -> Result<Self> {
        parse(BUILTIN_ROSTER)
    }

    pub(crate) fn load() -> Result<Self> {
        let path = crate::config::user::path()?.with_file_name("roster.yaml");
        Self::builtin()?.with_user_overrides(&path)
    }

    fn with_user_overrides(self, path: &camino::Utf8Path) -> Result<Self> {
        match crate::store::read_optional_yaml::<RawRegistry>(path)? {
            Some(raw) => Ok(self.merge(
                Self::from_registry(raw)
                    .with_context(|| format!("invalid model registry {path}"))?,
            )),
            None => Ok(self),
        }
    }

    pub(crate) fn with_overrides(self, overrides: RawRoster) -> Result<Self> {
        let mut overrides = Self::from_raw(overrides)?;
        for (family, models) in &mut overrides.families {
            for (name, model) in models {
                if let Some(existing) = self
                    .families
                    .get(family)
                    .and_then(|models| models.get(name))
                {
                    model.entries = existing.entries.clone();
                }
            }
        }
        Ok(self.merge(overrides))
    }

    fn merge(mut self, overrides: Self) -> Self {
        for (family, models) in overrides.families {
            self.families.entry(family).or_default().extend(models);
        }
        self
    }

    // serde-saphyr 1.3.0's typed deserialization handles YAML and duplicate keys,
    // not Niles's model/effort normalization: https://docs.rs/serde-saphyr/1.3.0/serde_saphyr/.
    fn from_registry(raw: RawRegistry) -> Result<Self> {
        let mut roster = RawRoster::default();
        for (family, models) in raw {
            let mut parsed = BTreeMap::new();
            for (name, mut entries) in models {
                if entries.is_empty()
                    || (entries.len() > 1 && entries.iter().any(|entry| entry.effort.is_none()))
                {
                    bail!("{family}:{name} needs efforts or a single entry without an effort");
                }
                for entry in &mut entries {
                    if entry.description.trim().is_empty()
                        || entry.description.chars().any(char::is_control)
                    {
                        bail!(
                            "{family}:{name} description must be one non-empty line without controls"
                        );
                    }
                    entry.effort = entry.effort.as_deref().map(normalize_effort);
                }
                let efforts = entries
                    .iter()
                    .filter_map(|entry| entry.effort.clone())
                    .collect();
                let context_window = entries.iter().map(|entry| entry.context_window.get()).min();
                parsed.insert(
                    name,
                    Model {
                        efforts,
                        context_window,
                        entries,
                    },
                );
            }
            roster.0.insert(family, parsed);
        }
        Self::from_raw(roster)
    }

    pub fn entry(&self, family: &str, model: &str, effort: Option<&str>) -> Result<&RegistryEntry> {
        self.families.get(family).and_then(|models| models.get(model))
            .and_then(|model| model.entries.iter().find(|entry| entry.effort.as_deref() == effort))
            .with_context(|| format!("no registry metadata for {family}:{model} effort {effort:?}; register it in ~/.niles/roster.yaml"))
    }

    fn from_raw(raw: RawRoster) -> Result<Self> {
        let known_families = families::known_agent_ids().collect::<Vec<_>>();
        let expected_families = known_families.join(", ");
        let mut parsed = BTreeMap::new();

        for (family, models) in raw.0 {
            if !known_families.contains(&family.as_str()) {
                bail!("unknown model family `{family}`; expected {expected_families}");
            }
            let mut normalized_models = BTreeSet::new();
            let mut entries = BTreeMap::new();
            for (name, mut model) in models {
                let name = normalize_model(&family, &name)?;
                if !normalized_models.insert(name.clone()) {
                    bail!("duplicate {family} model `{name}` after normalization");
                }
                model.efforts = normalize_efforts(&family, &name, model.efforts)?;
                entries.insert(name, model);
            }
            parsed.insert(family, entries);
        }

        Ok(Self { families: parsed })
    }

    pub(crate) fn model_names(&self, family: &str) -> impl Iterator<Item = &str> {
        self.families
            .get(family)
            .into_iter()
            .flatten()
            .map(|(model, _)| model.as_str())
    }

    pub(crate) fn supported_efforts(&self, family: &str, model: &str) -> Option<&[String]> {
        Some(&self.families.get(family)?.get(model)?.efforts)
    }

    pub(crate) fn context_window(&self, family: &str, model: &str) -> Option<u64> {
        self.families.get(family)?.get(model)?.context_window
    }

    pub(crate) fn registry_rows(
        &self,
        mut allows: impl FnMut(&str, &str, Option<&str>) -> Result<bool>,
    ) -> Result<Vec<[String; 8]>> {
        let mut rows = Vec::new();
        for family in families::known_agent_ids() {
            for name in self.model_names(family) {
                let efforts = self
                    .supported_efforts(family, name)
                    .context("model has no effort ladder")?;
                let pairs = if efforts.is_empty() {
                    vec![None]
                } else {
                    efforts.iter().map(|effort| Some(effort.as_str())).collect()
                };
                for effort in pairs {
                    if allows(family, name, effort)? {
                        let entry = self.entry(family, name, effort)?;
                        rows.push([
                            family.to_owned(),
                            name.to_owned(),
                            effort.iter().copied().collect(),
                            entry.tier.as_str().to_owned(),
                            serde_json::to_string(&entry.description)?,
                            relative_rank(entry.cost),
                            relative_rank(entry.speed),
                            entry.context_window.to_string(),
                        ]);
                    }
                }
            }
        }
        Ok(rows)
    }

    pub(crate) fn rows(&self) -> Vec<[String; 3]> {
        let mut rows = Vec::new();
        for family in families::known_agent_ids() {
            if let Some(models) = self.families.get(family) {
                rows.extend(models.iter().map(|(name, model)| {
                    [family.to_owned(), name.clone(), model.efforts.join(" ")]
                }));
            }
        }
        rows
    }
}

fn relative_rank(rank: Option<NonZeroU32>) -> String {
    match rank {
        Some(rank) => rank.to_string(),
        None => "unknown".to_owned(),
    }
}

pub(crate) fn parse(body: &str) -> Result<ModelRoster> {
    let raw = crate::store::parse_yaml(body)?;
    ModelRoster::from_registry(raw)
}

pub(crate) fn normalize_model(family: &str, model: &str) -> Result<String> {
    let normalized = model.to_ascii_lowercase();
    if !normalized.is_empty()
        && normalized
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | ':'))
    {
        return Ok(normalized);
    }

    bail!("invalid {family} model `{model}`; expected letters, digits, '.', '_', '-', '/', or ':'")
}

pub(crate) fn normalize_effort(effort: &str) -> String {
    match effort.to_ascii_lowercase().as_str() {
        "med" => "medium".to_owned(),
        value => value.to_owned(),
    }
}

fn normalize_efforts(family: &str, model: &str, efforts: Vec<String>) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::with_capacity(efforts.len());
    for effort in efforts {
        let effort = normalize_effort(&effort);
        if effort.is_empty()
            || !effort
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        {
            bail!(
                "invalid effort `{effort}` for {family} model `{model}`; expected letters, digits, '_', or '-'"
            );
        }
        if !seen.insert(effort.clone()) {
            bail!("duplicate effort `{effort}` for {family} model `{model}`");
        }
        normalized.push(effort);
    }
    Ok(normalized)
}

#[cfg(test)]
pub(crate) const FIXTURE_REGISTRY: &str = r#"
codex:
  fixture:
    - &entry {effort: med, tier: standard, description: 'bounded, "work"', cost: 2, speed: 3, context_window: 1000}
    - {<<: *entry, effort: high, tier: frontier, context_window: 2000}
  spare:
    - {<<: *entry, effort: low, tier: economy, cost: null, speed: null}
claude:
  haiku:
    - {<<: *entry, effort: null, tier: mechanical}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_fixture_parses_exact_pair_metadata() {
        let roster = parse(FIXTURE_REGISTRY).unwrap();
        assert_eq!(roster.context_window("codex", "fixture"), Some(1000));
        let high = roster.entry("codex", "fixture", Some("high")).unwrap();
        assert_eq!(high.context_window.get(), 2000);
        assert!(roster.entry("codex", "fixture", Some("low")).is_err());
        let rows = roster.registry_rows(|_, _, _| Ok(true)).unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[0].join(","),
            r#"codex,fixture,medium,standard,"bounded, \"work\"",2,3,1000"#
        );
        assert_eq!(rows[2][5..7], ["unknown", "unknown"]);
        assert_eq!(rows[3][2], "");
    }

    #[test]
    fn registry_fixture_rejects_invalid_entries_and_normalized_duplicates() {
        for (from, to, cause) in [
            ("tier: standard", "tier: unknown", "unknown variant"),
            ("cost: 2", "cost: 0", "nonzero"),
            ("speed: 3", "speed: 0", "nonzero"),
            ("context_window: 1000", "context_window: 0", "nonzero"),
            ("cost: 2", "extra: true, cost: 2", "unknown field"),
            ("effort: med", "effort: 'bad effort'", "invalid effort"),
            ("effort: med", "effort: high", "duplicate effort"),
            ("effort: med", "effort: null", "without an effort"),
            ("'bounded, \"work\"'", "''", "non-empty line"),
            ("'bounded, \"work\"'", r#""bad\nline""#, "without controls"),
            ("codex:", "unknown:", "unknown model family"),
            ("  spare:", "  FIXTURE:", "after normalization"),
            ("  spare:", "  'bad model':", "invalid codex model"),
        ] {
            let body = FIXTURE_REGISTRY.replace(from, to);
            let error = parse(&body).unwrap_err().to_string();
            assert!(error.contains(cause), "{from} => {to}: {error}");
        }
        assert!(parse("codex: {fixture: []}").is_err());
    }

    #[test]
    fn registry_fixture_user_file_replaces_models_and_errors_with_path() {
        let path = crate::test_support::temp_test_path("registry-override");
        let roster = parse(FIXTURE_REGISTRY).unwrap();
        assert_eq!(roster.clone().with_user_overrides(&path).unwrap(), roster);
        std::fs::write(&path, "pi: {added: [{tier: economy, description: added, cost: 1, speed: 4, context_window: 3000}]}\ncodex: {fixture: [{effort: low, tier: mechanical, description: changed, cost: 4, speed: 1, context_window: 3000}]}").unwrap();
        let changed = roster.clone().with_user_overrides(&path).unwrap();
        assert!(changed.entry("codex", "fixture", Some("medium")).is_err());
        let rows = changed.registry_rows(|_, _, _| Ok(true)).unwrap();
        assert_eq!(
            rows[0].join(","),
            "codex,fixture,low,mechanical,\"changed\",4,1,3000"
        );
        assert!(changed.entry("codex", "spare", Some("low")).is_ok());
        assert!(changed.entry("pi", "added", None).is_ok());
        for body in ["codex: {fixture: [{tier: invalid}]}", "unknown: {}"] {
            std::fs::write(&path, body).unwrap();
            let result = roster.clone().with_user_overrides(&path);
            let error = format!("{:#}", result.unwrap_err());
            assert!(error.contains(path.as_str()), "{error}");
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn embedded_roster_parses_and_matches_profiles() {
        let roster = ModelRoster::builtin().unwrap();
        assert_eq!(roster.registry_rows(|_, _, _| Ok(true)).unwrap().len(), 66);
        assert!(
            roster
                .supported_efforts("claude", "haiku")
                .unwrap()
                .is_empty()
        );
        let roster_families = roster
            .families
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let profile_families = families::known_agent_ids().collect::<BTreeSet<_>>();

        assert_eq!(roster_families, profile_families);
    }
}

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde::Deserialize;

use super::families;

const BUILTIN_ROSTER: &str = include_str!("roster.yaml");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRoster {
    families: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub(crate) struct RawRoster(BTreeMap<String, BTreeMap<String, RawModel>>);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModel {
    efforts: Vec<String>,
}

impl ModelRoster {
    pub(crate) fn builtin() -> Result<Self> {
        parse(BUILTIN_ROSTER)
    }

    pub(crate) fn with_overrides(mut self, overrides: RawRoster) -> Result<Self> {
        let mut overrides = Self::from_raw(overrides)?;

        for (family, models) in &mut self.families {
            let Some(additions) = overrides.families.remove(family) else {
                continue;
            };
            models.extend(additions);
        }
        debug_assert!(overrides.families.is_empty());
        Ok(self)
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
            for (name, model) in models {
                let name = normalize_model(&family, &name)?;
                if !normalized_models.insert(name.clone()) {
                    bail!("duplicate {family} model `{name}` after normalization");
                }
                let efforts = normalize_efforts(&family, &name, model.efforts)?;
                entries.insert(name, efforts);
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
        self.families.get(family)?.get(model).map(Vec::as_slice)
    }

    pub(crate) fn rows(&self) -> Vec<[String; 3]> {
        let mut rows = Vec::new();
        for family in families::known_agent_ids() {
            if let Some(models) = self.families.get(family) {
                rows.extend(
                    models.iter().map(|(model, efforts)| {
                        [family.to_owned(), model.clone(), efforts.join(" ")]
                    }),
                );
            }
        }
        rows
    }
}

pub(crate) fn parse(body: &str) -> Result<ModelRoster> {
    let raw = crate::schema::parse_yaml(body)?;
    ModelRoster::from_raw(raw)
}

pub(crate) fn normalize_model(family: &str, model: &str) -> Result<String> {
    let normalized = model.to_ascii_lowercase();
    if !normalized.is_empty()
        && normalized
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/'))
    {
        return Ok(normalized);
    }

    bail!("invalid {family} model `{model}`; expected letters, digits, '.', '_', '-', or '/'")
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
mod tests {
    use super::*;

    #[test]
    fn embedded_roster_parses_and_matches_profiles() {
        let roster = ModelRoster::builtin().unwrap();
        let roster_families = roster
            .families
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let profile_families = families::known_agent_ids().collect::<BTreeSet<_>>();

        assert_eq!(roster_families, profile_families);
        for family in families::known_agent_ids() {
            let profile = families::profile_for(family).unwrap();
            assert!(
                roster
                    .supported_efforts(family, profile.default_model)
                    .is_some(),
                "{} default model `{}` is missing from the built-in roster",
                profile.id,
                profile.default_model
            );
        }
    }
}

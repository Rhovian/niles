use super::profiles;
use crate::util;
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::{Arc, OnceLock};

mod query;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Catalogue {
    pub args: &'static [&'static str],
    pub request: Option<StdinRequest>,
    pub decode: fn(Value) -> Result<Models>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StdinRequest {
    pub body: &'static str,
    pub response_id_pointer: Option<&'static str>,
}

pub(crate) type Models = BTreeMap<String, Model>;
type Families = BTreeMap<&'static str, OnceLock<Result<Models, String>>>;

#[derive(Debug, Clone)]
pub struct ModelCatalogue {
    families: Arc<Families>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct Model {
    pub(crate) efforts: Vec<String>,
    pub(crate) context_window: Option<NonZeroU64>,
}

impl Default for ModelCatalogue {
    fn default() -> Self {
        static FAMILIES: OnceLock<Arc<Families>> = OnceLock::new();
        Self {
            families: Arc::clone(
                FAMILIES.get_or_init(|| {
                    Arc::new(profiles().map(|p| (p.id, OnceLock::new())).collect())
                }),
            ),
        }
    }
}

impl ModelCatalogue {
    fn family(&self, family: &str) -> Result<&Models> {
        // OnceLock in std 1.89 stores one parsed response per family for this process.
        // https://doc.rust-lang.org/1.89.0/std/sync/struct.OnceLock.html#method.get_or_init
        let profile = super::profile_for(family).context("unknown catalogue family")?;
        self.families
            .get(family)
            .context("unknown catalogue family")?
            .get_or_init(|| {
                let Some(binary) = util::find_on_path(profile.binary) else {
                    return Ok(Models::new());
                };
                query::models(profile.catalogue, binary.as_std_path())
                    .and_then(|body| {
                        (profile.catalogue.decode)(body).context("invalid catalogue response")
                    })
                    .map_err(|error| format!("{} model catalogue: {error:#}", profile.binary))
            })
            .as_ref()
            .map_err(|error| anyhow!(error.clone()))
    }

    pub(crate) fn models(&self, family: &str) -> Result<impl Iterator<Item = (&str, &[String])>> {
        Ok(self
            .family(family)?
            .iter()
            .map(|(name, model)| (name.as_str(), model.efforts.as_slice())))
    }

    pub(crate) fn context_window(&self, family: &str, model: &str) -> Result<Option<u64>> {
        Ok(self
            .family(family)?
            .get(model)
            .and_then(|model| model.context_window.map(NonZeroU64::get)))
    }

    pub(crate) fn model_rows(&self, family: &str) -> Result<Vec<[String; 4]>> {
        let mut rows = Vec::new();
        for (name, model) in self.family(family)? {
            let efforts = model
                .efforts
                .iter()
                .map(|effort| Some(effort.as_str()))
                .chain(model.efforts.is_empty().then_some(None));
            for effort in efforts {
                rows.push([
                    family.to_owned(),
                    name.clone(),
                    effort.map_or_else(String::new, str::to_owned),
                    model
                        .context_window
                        .map_or_else(|| "unknown".to_owned(), |window| window.to_string()),
                ]);
            }
        }
        Ok(rows)
    }

    #[cfg(test)]
    pub(crate) fn fixture() -> Self {
        use crate::store;
        let raw: BTreeMap<String, Models> =
            store::parse_yaml(include_str!("../test_support/catalogue.yaml")).unwrap();
        Self {
            families: profiles()
                .map(|p| {
                    (
                        p.id,
                        OnceLock::from(Ok(raw.get(p.id).expect("fixture family").clone())),
                    )
                })
                .collect::<BTreeMap<_, _>>()
                .into(),
        }
    }
    #[cfg(test)]
    pub(crate) fn with_error(mut self, family: &str) -> Self {
        *Arc::make_mut(&mut self.families)
            .get_mut(family)
            .expect("fixture family") =
            OnceLock::from(Err(format!("{family} model catalogue: fixture failure")));
        self
    }
}

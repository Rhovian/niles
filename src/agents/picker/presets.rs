use anyhow::Result;
use serde::Deserialize;

use crate::{
    agents::{self, AgentSpec},
    config::spec::ProjectConfig,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPreset {
    name: String,
    lead: String,
    worker: String,
    reviewer: String,
    security: String,
}

pub(crate) struct Preset {
    pub name: String,
    pub values: Result<[String; 4], String>,
}

pub(crate) fn load(config: &ProjectConfig) -> Result<Vec<Preset>> {
    let raw: Vec<RawPreset> = crate::store::parse_yaml(include_str!("../presets.yaml"))?;
    Ok(raw
        .into_iter()
        .map(|raw| {
            let values = [raw.lead, raw.worker, raw.reviewer, raw.security];
            let valid = values.iter().enumerate().try_for_each(|(index, value)| {
                if index == 2 && value == "lead" {
                    return Ok(());
                }
                let spec = AgentSpec::parse(value, &config.models)?;
                agents::canonical_manifest_agent(&spec, config).map(|_| ())
            });
            Preset {
                name: raw.name,
                values: valid
                    .map(|()| values)
                    .map_err(|err: anyhow::Error| err.to_string()),
            }
        })
        .collect())
}

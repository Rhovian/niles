use anyhow::Result;
use serde::Deserialize;

use super::ScalarRole;

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
    design: Vec<String>,
}

pub(crate) struct Preset {
    pub name: String,
    pub design: crate::workspace_manifest::RoleBinding,
    pub values: Result<[(ScalarRole, String); 4], String>,
}

pub(crate) fn load(config: &ProjectConfig) -> Result<Vec<Preset>> {
    let raw: Vec<RawPreset> = crate::store::parse_yaml(include_str!("../presets.yaml"))?;
    Ok(raw
        .into_iter()
        .map(|raw| {
            let values = [
                (ScalarRole::Lead, raw.lead),
                (ScalarRole::Worker, raw.worker),
                (ScalarRole::Reviewer, raw.reviewer),
                (ScalarRole::Security, raw.security),
            ];
            let valid = values
                .iter()
                .map(|(_, value)| value)
                .chain(&raw.design)
                .try_for_each(|value| {
                    let spec = AgentSpec::parse(value, &config.models)?;
                    agents::canonical_manifest_agent(&spec, config).map(|_| ())
                });
            Preset {
                name: raw.name,
                design: crate::workspace_manifest::RoleBinding(vec![
                    crate::workspace_manifest::AgentGroup {
                        when: None,
                        models: raw.design,
                        efforts: None,
                    },
                ]),
                values: valid
                    .map(|()| values)
                    .map_err(|err: anyhow::Error| err.to_string()),
            }
        })
        .collect())
}

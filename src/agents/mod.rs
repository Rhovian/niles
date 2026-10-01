use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::config::spec::{AgentConfig, ProjectConfig};

mod families;
pub(crate) mod picker;
pub(crate) mod roster;
#[cfg(test)]
mod tests;

pub use families::{BriefDelivery, known_agent_ids, profile_for};
pub use roster::ModelRoster;

const CUSTOM_AGENT_BRIEF: BriefDelivery = BriefDelivery::Arg;

#[derive(Debug, Clone, Copy)]
pub enum InvocationDefaults {
    Foreground,
    Worker,
}

#[derive(Debug, Clone)]
pub struct AgentInvocation {
    pub binary: String,
    pub args: Vec<String>,
    pub brief: BriefDelivery,
    pub spec: AgentSpec,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    family: String,
    model: Option<String>,
    effort: Option<String>,
}

pub fn config_for<'a>(
    configs: &'a BTreeMap<String, AgentConfig>,
    agent: &str,
    models: &ModelRoster,
) -> Result<Option<&'a AgentConfig>> {
    let spec = AgentSpec::parse(agent, models)?;
    Ok(configs.get(agent).or_else(|| configs.get(spec.family())))
}

pub fn invocation(
    agent: &str,
    config: Option<&AgentConfig>,
    defaults: InvocationDefaults,
    models: &ModelRoster,
) -> Result<AgentInvocation> {
    let spec = AgentSpec::parse(agent, models)?;
    // Both launch paths come through here, so effective-roster validation cannot be skipped by
    // one of them: no probe, manifest, or subprocess gets a separate answer.
    validate_model(&spec, models)?;
    let default_invocation = default_invocation(&spec, defaults);

    let mut invocation = match config {
        Some(config) => AgentInvocation {
            binary: configured_or_default_binary(
                config.binary.as_deref(),
                default_invocation.binary,
            ),
            args: if config.args.is_empty() {
                default_invocation.args
            } else {
                config.args.clone()
            },
            brief: config.prompt.into(),
            env: default_invocation.env,
            spec,
        },
        None => default_invocation,
    };
    if let Some(profile) = profile_for(invocation.spec.family()) {
        invocation.args.extend(families::tier_args(
            profile,
            invocation.spec.model(),
            invocation.spec.effort(),
        ));
    }
    Ok(invocation)
}

fn default_invocation(spec: &AgentSpec, defaults: InvocationDefaults) -> AgentInvocation {
    match profile_for(spec.family()) {
        Some(profile) => {
            let (args, brief) = match defaults {
                InvocationDefaults::Foreground => (profile.foreground_args, profile.lead_brief),
                InvocationDefaults::Worker => (profile.worker_args, profile.worker_brief),
            };
            AgentInvocation {
                binary: profile.binary.to_owned(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                brief,
                env: profile
                    .launch_env
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                    .collect(),
                spec: spec.clone(),
            }
        }
        None => AgentInvocation {
            binary: spec.family().to_owned(),
            args: Vec::new(),
            brief: CUSTOM_AGENT_BRIEF,
            env: Vec::new(),
            spec: spec.clone(),
        },
    }
}

fn configured_or_default_binary(configured: Option<&str>, default: String) -> String {
    match configured {
        Some(binary) => binary.to_owned(),
        None => default,
    }
}

impl AgentSpec {
    pub fn parse(agent: &str, models: &ModelRoster) -> Result<Self> {
        let parts = agent.split(':').collect::<Vec<_>>();
        if parts.is_empty() || parts.len() > 3 {
            bail!("invalid agent spec `{agent}`; expected family[:model[:effort]]");
        }
        if parts.iter().any(|part| part.trim().is_empty()) {
            bail!("invalid agent spec `{agent}`; family, model, and effort cannot be empty");
        }

        Self::from_parts(
            parts[0],
            parts.get(1).copied(),
            parts.get(2).copied(),
            models,
        )
    }

    pub fn from_parts(
        family: &str,
        model: Option<&str>,
        effort: Option<&str>,
        models: &ModelRoster,
    ) -> Result<Self> {
        let family = canonical_family(family).unwrap_or_else(|| family.to_owned());
        if effort.is_some() && model.is_none() {
            bail!("invalid agent spec; effort requires a model");
        }
        if model.is_some() && profile_for(&family).is_none() {
            bail!(
                "unknown agent family `{family}`; model/effort qualifiers are only supported for builtin agents"
            );
        }
        let model = model
            .map(|value| roster::normalize_model(&family, value))
            .transpose()?;
        let effort = effort
            .zip(model.as_deref())
            .map(|(value, model)| normalize_effort(&family, model, value, models))
            .transpose()?;

        Ok(Self {
            family,
            model,
            effort,
        })
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn effort(&self) -> Option<&str> {
        self.effort.as_deref()
    }

    pub fn tiered_family(&self) -> Option<String> {
        self.model.is_some().then(|| self.family.clone())
    }

    pub fn canonical(&self) -> String {
        let mut spec = self.family.clone();
        if let Some(model) = &self.model {
            spec.push(':');
            spec.push_str(model);
        }
        if let Some(effort) = &self.effort {
            spec.push(':');
            spec.push_str(effort);
        }
        spec
    }
}

fn canonical_family(family: &str) -> Option<String> {
    profile_for(family).map(|profile| profile.id.to_owned())
}

fn normalize_effort(
    family: &str,
    model: &str,
    effort: &str,
    models: &ModelRoster,
) -> Result<String> {
    let normalized = roster::normalize_effort(effort);
    match models.supported_efforts(family, model) {
        None => Ok(normalized),
        Some(supported) if supported.iter().any(|candidate| candidate == &normalized) => {
            Ok(normalized)
        }
        Some([]) => {
            bail!("{family} model `{model}` takes no effort; drop the effort from the agent spec")
        }
        Some(_) => {
            bail!("unsupported {family} effort `{effort}` for model `{model}` in agent spec")
        }
    }
}

pub(crate) fn validate_model(spec: &AgentSpec, models: &ModelRoster) -> Result<()> {
    let Some(model) = spec.model() else {
        return Ok(());
    };

    if models.supported_efforts(spec.family(), model).is_some() {
        return Ok(());
    }

    bail!(
        "unsupported {} model `{model}` in agent spec",
        spec.family()
    )
}

pub(crate) fn canonical_manifest_agent(spec: &AgentSpec, config: &ProjectConfig) -> Result<String> {
    validate_model(spec, &config.models)?;
    if profile_for(spec.family()).is_some()
        || config.agents.contains_key(&spec.canonical())
        || config.agents.contains_key(spec.family())
    {
        return Ok(spec.canonical());
    }

    bail!(
        "unknown agent `{}`; configure it in niles.yaml or choose a builtin agent",
        spec.family()
    )
}

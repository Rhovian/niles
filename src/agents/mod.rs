use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::config::spec::AgentConfig;

mod families;
pub(crate) mod picker;
pub(crate) mod roster;
#[cfg(test)]
mod tests;

pub use families::{AgentProfile, BriefDelivery};
pub use roster::ModelRoster;

const CUSTOM_AGENT_DEFAULT_ARGS: &[&str] = &[];
const CUSTOM_AGENT_LAUNCH_ENV: &[(&str, &str)] = &[];
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTier {
    pub family: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

pub fn known_agent_ids() -> impl Iterator<Item = &'static str> {
    families::known_agent_ids()
}

pub fn profile_for(agent: &str) -> Option<AgentProfile> {
    families::profile_for(agent)
}

pub fn parse_spec(agent: &str, models: &ModelRoster) -> Result<AgentSpec> {
    AgentSpec::parse(agent, models)
}

pub fn config_for<'a>(
    configs: &'a BTreeMap<String, AgentConfig>,
    agent: &str,
    models: &ModelRoster,
) -> Result<Option<&'a AgentConfig>> {
    let spec = AgentSpec::parse(agent, models)?;
    Ok(configs.get(agent).or_else(|| configs.get(spec.family())))
}

pub fn default_binary(agent: &str) -> String {
    let family = canonical_family(agent).unwrap_or_else(|| agent.to_owned());
    default_binary_for_family(family.clone(), profile_for(&family))
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
    invocation
        .args
        .extend(tier_args_for_spec(&invocation.spec)?);
    Ok(invocation)
}

pub fn foreground_invocation(
    agent: &str,
    config: Option<&AgentConfig>,
    models: &ModelRoster,
) -> Result<AgentInvocation> {
    invocation(agent, config, InvocationDefaults::Foreground, models)
}

fn default_invocation(spec: &AgentSpec, defaults: InvocationDefaults) -> AgentInvocation {
    let profile = profile_for(spec.family());
    match defaults {
        InvocationDefaults::Foreground => AgentInvocation {
            binary: default_binary(spec.family()),
            args: args_for_foreground(profile),
            brief: brief_for_profile(profile, defaults),
            env: launch_env(profile),
            spec: spec.clone(),
        },
        InvocationDefaults::Worker => AgentInvocation {
            binary: default_binary(spec.family()),
            args: args_for_worker(profile),
            brief: brief_for_profile(profile, defaults),
            env: launch_env(profile),
            spec: spec.clone(),
        },
    }
}

fn default_binary_for_family(family: String, profile: Option<AgentProfile>) -> String {
    match profile {
        Some(profile) => profile.binary.to_owned(),
        None => family,
    }
}

fn configured_or_default_binary(configured: Option<&str>, default: String) -> String {
    match configured {
        Some(binary) => binary.to_owned(),
        None => default,
    }
}

fn args_for_foreground(profile: Option<AgentProfile>) -> Vec<String> {
    match profile {
        Some(profile) => args(profile.foreground_args),
        None => Vec::new(),
    }
}

fn args_for_worker(profile: Option<AgentProfile>) -> Vec<String> {
    match profile {
        Some(profile) => args(profile.worker_args),
        None => args(CUSTOM_AGENT_DEFAULT_ARGS),
    }
}

fn brief_for_profile(profile: Option<AgentProfile>, defaults: InvocationDefaults) -> BriefDelivery {
    let Some(profile) = profile else {
        return CUSTOM_AGENT_BRIEF;
    };
    match defaults {
        InvocationDefaults::Foreground => profile.lead_brief,
        InvocationDefaults::Worker => profile.worker_brief,
    }
}

fn args(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn launch_env(profile: Option<AgentProfile>) -> Vec<(String, String)> {
    let launch_env = match profile {
        Some(profile) => profile.launch_env,
        None => CUSTOM_AGENT_LAUNCH_ENV,
    };
    launch_env
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn tier_args_for_spec(spec: &AgentSpec) -> Result<Vec<String>> {
    if let Some(profile) = profile_for(spec.family()) {
        return Ok(families::tier_args(profile, spec.model(), spec.effort()));
    }

    if spec.model().is_some() || spec.effort().is_some() {
        bail!(
            "agent `{}` does not support model/effort qualifiers",
            spec.family()
        );
    }

    Ok(Vec::new())
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
        let model = model
            .map(|value| normalize_model(&family, value))
            .transpose()?;
        let effort = effort
            .map(|value| normalize_effort(&family, model.as_deref(), value, models))
            .transpose()?;
        if effort.is_some() && model.is_none() {
            bail!("invalid agent spec; effort requires a model");
        }
        if model.is_some() && profile_for(&family).is_none() {
            bail!(
                "unknown agent family `{family}`; model/effort qualifiers are only supported for builtin agents"
            );
        }

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

    pub fn tier(&self) -> Option<AgentTier> {
        if self.model.is_none() && self.effort.is_none() {
            return None;
        }

        Some(AgentTier {
            family: self.family.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
        })
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

fn normalize_model(family: &str, model: &str) -> Result<String> {
    profile_for(family)
        .map(|_| roster::normalize_model(family, model))
        .unwrap_or_else(|| Ok(model.to_owned()))
}

fn normalize_effort(
    family: &str,
    model: Option<&str>,
    effort: &str,
    models: &ModelRoster,
) -> Result<String> {
    if profile_for(family).is_none() {
        return Ok(effort.to_owned());
    }

    let normalized = roster::normalize_effort(effort);
    let Some(model) = model else {
        return Ok(normalized);
    };
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

    if profile_for(spec.family()).is_none() {
        return Ok(());
    }

    if models.supported_efforts(spec.family(), model).is_some() {
        return Ok(());
    }

    bail!(
        "unsupported {} model `{model}` in agent spec",
        spec.family()
    )
}

pub(crate) fn canonical_manifest_agent(
    spec: &AgentSpec,
    agent_configs: &BTreeMap<String, AgentConfig>,
    models: &ModelRoster,
) -> Result<String> {
    validate_model(spec, models)?;
    if profile_for(spec.family()).is_some()
        || agent_configs.contains_key(&spec.canonical())
        || agent_configs.contains_key(spec.family())
    {
        return Ok(spec.canonical());
    }

    bail!(
        "unknown agent `{}`; configure it in niles.yaml or choose a builtin agent",
        spec.family()
    )
}

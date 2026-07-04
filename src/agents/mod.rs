use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::config::spec::{AgentConfig, PromptMode};

pub(crate) mod capabilities;
pub(crate) mod catalog;
mod families;
pub(crate) mod picker;
#[cfg(test)]
mod tests;
pub(crate) mod version;

pub use families::AgentProfile;

#[derive(Debug, Clone, Copy)]
pub enum InvocationDefaults {
    Default,
    Foreground,
    Worker,
}

#[derive(Debug, Clone)]
pub struct AgentInvocation {
    pub binary: String,
    pub args: Vec<String>,
    pub prompt: PromptMode,
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

pub fn parse_spec(agent: &str) -> Result<AgentSpec> {
    AgentSpec::parse(agent)
}

pub fn config_for<'a>(
    configs: &'a BTreeMap<String, AgentConfig>,
    agent: &str,
) -> Result<Option<&'a AgentConfig>> {
    let spec = AgentSpec::parse(agent)?;
    Ok(configs.get(agent).or_else(|| configs.get(spec.family())))
}

pub fn default_binary(agent: &str) -> String {
    let family = family_or_self(agent);
    profile_for(&family)
        .map(|profile| profile.binary.to_owned())
        .unwrap_or(family)
}

pub fn default_args(agent: &str) -> Vec<String> {
    let family = family_or_self(agent);
    profile_for(&family).map(profile_args).unwrap_or_default()
}

pub fn default_prompt(agent: &str) -> PromptMode {
    let family = family_or_self(agent);
    profile_for(&family)
        .map(|profile| profile.prompt)
        .unwrap_or(PromptMode::Arg)
}

pub fn invocation(
    agent: &str,
    config: Option<&AgentConfig>,
    defaults: InvocationDefaults,
) -> Result<AgentInvocation> {
    let spec = AgentSpec::parse(agent)?;
    let default_invocation = default_invocation(&spec, defaults);

    let mut invocation = match config {
        Some(config) => AgentInvocation {
            binary: config.binary.clone().unwrap_or(default_invocation.binary),
            args: if config.args.is_empty() {
                default_invocation.args
            } else {
                config.args.clone()
            },
            prompt: config.prompt,
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

pub fn foreground_invocation(agent: &str, config: Option<&AgentConfig>) -> Result<AgentInvocation> {
    invocation(agent, config, InvocationDefaults::Foreground)
}

fn default_invocation(spec: &AgentSpec, defaults: InvocationDefaults) -> AgentInvocation {
    let profile = profile_for(spec.family());
    match defaults {
        InvocationDefaults::Default => AgentInvocation {
            binary: default_binary(spec.family()),
            args: default_args(spec.family()),
            prompt: default_prompt(spec.family()),
            env: launch_env(profile),
            spec: spec.clone(),
        },
        InvocationDefaults::Foreground => AgentInvocation {
            binary: default_binary(spec.family()),
            args: Vec::new(),
            prompt: prompt_for_profile(profile),
            env: launch_env(profile),
            spec: spec.clone(),
        },
        InvocationDefaults::Worker => AgentInvocation {
            binary: default_binary(spec.family()),
            args: args_for_worker(spec.family(), profile),
            prompt: prompt_for_profile(profile),
            env: launch_env(profile),
            spec: spec.clone(),
        },
    }
}

fn args_for_worker(agent: &str, profile: Option<AgentProfile>) -> Vec<String> {
    profile
        .map(|profile| args(profile.worker_args))
        .unwrap_or_else(|| default_args(agent))
}

fn prompt_for_profile(profile: Option<AgentProfile>) -> PromptMode {
    profile
        .map(|profile| profile.worker_prompt)
        .unwrap_or(PromptMode::Arg)
}

fn profile_args(profile: AgentProfile) -> Vec<String> {
    args(profile.args)
}

fn args(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn launch_env(profile: Option<AgentProfile>) -> Vec<(String, String)> {
    profile
        .map(|profile| {
            profile
                .launch_env
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

fn family_or_self(agent: &str) -> String {
    AgentSpec::parse(agent)
        .map(|spec| spec.family().to_owned())
        .unwrap_or_else(|_| agent.to_owned())
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
    pub fn parse(agent: &str) -> Result<Self> {
        let parts = agent.split(':').collect::<Vec<_>>();
        if parts.is_empty() || parts.len() > 3 {
            bail!("invalid agent spec `{agent}`; expected family[:model[:effort]]");
        }
        if parts.iter().any(|part| part.trim().is_empty()) {
            bail!("invalid agent spec `{agent}`; family, model, and effort cannot be empty");
        }

        Self::from_parts(parts[0], parts.get(1).copied(), parts.get(2).copied())
    }

    pub fn from_parts(family: &str, model: Option<&str>, effort: Option<&str>) -> Result<Self> {
        let family = canonical_family(family).unwrap_or_else(|| family.to_owned());
        let model = model
            .map(|value| normalize_model(&family, value))
            .transpose()?;
        let effort = effort
            .map(|value| normalize_effort(&family, value))
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
        .map(|profile| families::normalize_model(profile, model))
        .unwrap_or_else(|| Ok(model.to_owned()))
}

fn normalize_effort(family: &str, effort: &str) -> Result<String> {
    profile_for(family)
        .map(|profile| families::normalize_effort(profile, effort))
        .unwrap_or_else(|| Ok(effort.to_owned()))
}

pub(crate) fn validate_static_model(spec: &AgentSpec) -> Result<()> {
    let Some(model) = spec.model() else {
        return Ok(());
    };

    let Some(profile) = profile_for(spec.family()) else {
        return Ok(());
    };

    if families::supports_model(profile, model) {
        return Ok(());
    }

    bail!(
        "unsupported {} model `{model}` in agent spec",
        spec.family()
    )
}

pub(crate) fn default_model_aliases(family: &str) -> &'static [&'static str] {
    profile_for(family)
        .map(|profile| profile.model_aliases)
        .unwrap_or(&[])
}

pub(crate) fn supported_efforts(family: &str) -> &'static [&'static str] {
    profile_for(family)
        .map(|profile| profile.supported_efforts)
        .unwrap_or(&[])
}

pub(crate) fn model_group_label(family: &str, model: &str) -> String {
    profile_for(family)
        .map(|profile| families::model_group_label(profile, model))
        .unwrap_or_else(|| model.to_owned())
}

pub(crate) fn manager_prompt_args(
    agent: &str,
    brief: String,
    startup_prompt: String,
) -> Vec<String> {
    let profile = AgentSpec::parse(agent)
        .ok()
        .and_then(|spec| profile_for(spec.family()));
    families::manager_prompt_args(profile, brief, startup_prompt)
}

pub(crate) fn canonical_manifest_agent(
    spec: &AgentSpec,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<String> {
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

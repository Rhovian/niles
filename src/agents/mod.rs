use anyhow::{Context, Result, bail};

mod families;
pub(crate) mod picker;
pub(crate) mod roster;
mod system_prompt;
#[cfg(test)]
mod tests;

pub use families::{BriefDelivery, SystemPromptArg, known_agent_ids, profile_for, profiles};
pub(crate) use families::{SwitchStep, session_link, switch_steps};
pub use roster::ModelRoster;

#[derive(Debug, Clone, Copy)]
pub enum InvocationDefaults {
    Foreground,
    Worker,
    Headless,
}

#[derive(Debug, Clone)]
pub struct AgentInvocation {
    pub binary: &'static str,
    pub args: Vec<String>,
    pub brief: BriefDelivery,
    pub system_prompt: SystemPromptArg,
    pub spec: AgentSpec,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    profile: &'static families::AgentProfile,
    model: Option<String>,
    effort: Option<String>,
}

pub fn invocation(
    agent: &str,
    defaults: InvocationDefaults,
    models: &ModelRoster,
) -> Result<AgentInvocation> {
    let spec = AgentSpec::parse(agent, models)?;
    validate_model(&spec, models)?;
    let profile = spec.profile();
    let args = match defaults {
        InvocationDefaults::Foreground => profile.foreground_args,
        InvocationDefaults::Worker | InvocationDefaults::Headless => profile.worker_args,
    };
    let mut args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    if matches!(defaults, InvocationDefaults::Headless) {
        args.extend(profile.headless.iter().map(|arg| (*arg).to_owned()));
    }
    args.extend(families::tier_args(profile, spec.model(), spec.effort()));
    Ok(AgentInvocation {
        binary: profile.binary,
        args,
        brief: profile.opening_turn,
        system_prompt: profile.system_prompt,
        env: profile
            .launch_env
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        spec,
    })
}

impl AgentSpec {
    pub fn parse(agent: &str, models: &ModelRoster) -> Result<Self> {
        let (family, rest) = match agent.split_once(':') {
            Some((family, rest)) => (family, Some(rest)),
            None => (agent, None),
        };
        // A model name may itself contain `:` (OpenRouter's `:free` variants). Such a name must be
        // on the roster, which is what tells its colon from the one before an effort.
        let listed = |model: &str| {
            profile_for(family).is_some_and(|profile| {
                models
                    .supported_efforts(profile.id, &model.to_ascii_lowercase())
                    .is_some()
            })
        };
        let (model, effort) = match rest {
            Some(rest) if listed(rest) => (Some(rest), None),
            Some(rest) => match rest.rsplit_once(':') {
                Some((model, effort)) if listed(model) || !model.contains(':') => {
                    (Some(model), Some(effort))
                }
                Some(_) => bail!("invalid agent spec `{agent}`; expected family[:model[:effort]]"),
                None => (Some(rest), None),
            },
            None => (None, None),
        };
        if [Some(family), model, effort]
            .into_iter()
            .flatten()
            .any(|part| part.trim().is_empty())
        {
            bail!("invalid agent spec `{agent}`; family, model, and effort cannot be empty");
        }

        Self::from_parts(family, model, effort, models)
    }

    pub fn from_parts(
        family: &str,
        model: Option<&str>,
        effort: Option<&str>,
        models: &ModelRoster,
    ) -> Result<Self> {
        let profile =
            profile_for(family).with_context(|| format!("unknown agent family `{family}`"))?;
        let family = profile.id;
        if effort.is_some() && model.is_none() {
            bail!("invalid agent spec; effort requires a model");
        }
        let model = model
            .map(|value| roster::normalize_model(family, value))
            .transpose()?;
        let effort = effort
            .zip(model.as_deref())
            .map(|(value, model)| normalize_effort(family, model, value, models))
            .transpose()?;

        Ok(Self {
            profile,
            model,
            effort,
        })
    }

    pub fn profile(&self) -> &'static families::AgentProfile {
        self.profile
    }

    pub fn family(&self) -> &str {
        self.profile.id
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn effort(&self) -> Option<&str> {
        self.effort.as_deref()
    }

    pub fn tiered_family(&self) -> Option<String> {
        self.model.is_some().then(|| self.profile.id.to_owned())
    }

    pub fn canonical(&self) -> String {
        let mut spec = self.profile.id.to_owned();
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

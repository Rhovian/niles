use anyhow::{Context, Result, bail};

mod acp;
pub(crate) use acp::{AuthSource, SessionConfig, SubscriptionStatus};
pub(crate) mod catalogue;
mod families;
mod mcp;
pub(crate) mod picker;
mod system_prompt;
#[cfg(test)]
mod tests;

pub use catalogue::ModelCatalogue;
pub(crate) use families::{AcpProfile, session_link};
pub use families::{SystemPromptArg, known_agent_ids, profile_for, profiles};

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
    pub system_prompt: SystemPromptArg,
    pub spec: AgentSpec,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct AgentSpec {
    profile: &'static families::AgentProfile,
    model: Option<String>,
    effort: Option<String>,
}

impl PartialEq for AgentSpec {
    fn eq(&self, other: &Self) -> bool {
        self.family() == other.family() && self.model == other.model && self.effort == other.effort
    }
}
impl Eq for AgentSpec {}

pub fn invocation(agent: &str, defaults: InvocationDefaults) -> Result<AgentInvocation> {
    let spec = AgentSpec::parse(agent)?;
    Ok(AgentInvocation::new(spec, defaults))
}

impl AgentInvocation {
    pub(crate) fn new(spec: AgentSpec, defaults: InvocationDefaults) -> Self {
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
        Self {
            binary: profile.binary,
            args,
            system_prompt: profile.system_prompt,
            env: profile
                .launch_env
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            spec,
        }
    }
}

impl AgentSpec {
    pub fn parse(agent: &str) -> Result<Self> {
        let (family, rest) = match agent.split_once(':') {
            Some((family, rest)) => (family, Some(rest)),
            None => (agent, None),
        };
        let (model, effort) = match rest {
            Some(rest) => match rest.rsplit_once(':') {
                Some((model, effort)) => (Some(model), (!effort.is_empty()).then_some(effort)),
                None => (Some(rest), None),
            },
            None => (None, None),
        };
        Self::from_parts(family, model, effort)
    }

    pub fn from_parts(family: &str, model: Option<&str>, effort: Option<&str>) -> Result<Self> {
        if [Some(family), model, effort]
            .into_iter()
            .flatten()
            .any(|part| part.trim().is_empty())
        {
            let spec = [Some(family), model, effort]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(":");
            bail!("invalid agent spec `{spec}`; family, model, and effort cannot be empty");
        }
        let profile =
            profile_for(family).with_context(|| format!("unknown agent family `{family}`"))?;
        if effort.is_some() && model.is_none() {
            bail!("invalid {family} agent spec; effort requires a model");
        }
        let model = model.map(str::to_owned);
        let effort = effort.map(str::to_owned);

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
        } else if self.model.as_ref().is_some_and(|model| model.contains(':')) {
            // The empty effort slot disambiguates colons belonging to the model.
            spec.push(':');
        }
        spec
    }
}

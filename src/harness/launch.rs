use super::acp;
use crate::agent_window::shell_quote;
use crate::{agents, telemetry::SessionLink, util};
use anyhow::{Context, Result, bail, ensure};
use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Copy, clap::ValueEnum, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum Billing {
    Subscription,
    ApiKey,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum Mode {
    NativeTui,
    Headless,
}

#[derive(Debug, Clone, Copy, clap::Args, Deserialize, schemars::JsonSchema)]
pub(crate) struct LaunchArgs {
    /// Launch mode. Defaults to native-tui, or the workspace's workers setting for spawn.
    #[arg(long, value_enum)]
    mode: Option<Mode>,
    /// Headless billing must be declared here or in the workspace's workers setting for spawn.
    #[arg(long, value_enum)]
    billing: Option<Billing>,
}

impl LaunchArgs {
    pub(crate) fn resolve(self, default: Launch) -> Result<Launch> {
        let mode = match (self.mode, default) {
            (Some(mode), _) => mode,
            (None, Launch::NativeTui {}) => Mode::NativeTui,
            (None, Launch::Headless { .. }) => Mode::Headless,
        };
        match (mode, self.billing) {
            (Mode::NativeTui, None) => Ok(Launch::NativeTui {}),
            (Mode::Headless, Some(Billing::Subscription)) => Ok(Launch::Headless {
                billing: HeadlessBilling::Subscription,
            }),
            (Mode::Headless, None) if default.is_headless() => Ok(default),
            (Mode::NativeTui, Some(_)) | (Mode::Headless, None | Some(Billing::ApiKey)) => bail!(
                "headless launch requires --billing subscription; API-key adapters are not qualified yet"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Launch {
    NativeTui {},
    Headless { billing: HeadlessBilling },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum HeadlessBilling {
    Subscription,
}

impl Launch {
    pub(crate) fn is_headless(self) -> bool {
        matches!(self, Self::Headless { .. })
    }
    pub(crate) fn qualify(self, spec: &agents::AgentSpec) -> Result<()> {
        if self.is_headless() {
            let profile = spec
                .profile()
                .acp
                .context("this agent has no qualified headless ACP adapter")?;
            ensure!(
                util::find_on_path(profile.binary).is_some(),
                "ACP adapter {} not found on PATH",
                profile.binary
            );
        }
        Ok(())
    }
    pub(crate) fn session_link(
        self,
        spec: &agents::AgentSpec,
        dir: &Utf8Path,
    ) -> Option<SessionLink> {
        if self.is_headless() {
            Some(SessionLink::Acp {
                dir: dir.to_owned(),
            })
        } else {
            agents::session_link(spec.family(), dir)
        }
    }
    pub(crate) fn configure(
        self,
        invocation: &mut agents::AgentInvocation,
        dir: &Utf8Path,
        primary: &str,
        worker: bool,
    ) -> Result<()> {
        if let Self::Headless {
            billing: HeadlessBilling::Subscription,
        } = self
        {
            let mut session = invocation
                .spec
                .profile()
                .acp
                .context("missing ACP profile")?
                .adapter
                .configure(&invocation.spec, primary, worker)?;
            session.env.extend(invocation.env.clone());
            acp::prepare(dir, invocation.spec.family(), session)?;
            let executable = env::current_exe()?;
            invocation.binary = "sh";
            invocation.args = vec![
                "-c".into(),
                format!(
                    "exec {} acp {} \"$@\"",
                    shell_quote(executable.to_str().context("non-UTF8 executable")?),
                    shell_quote(dir.as_str())
                ),
                "niles-acp".into(),
            ];
        } else if let Some(args) = invocation.spec.profile().mcp_args()? {
            // Keep variadic configuration flags ahead of the flags before the opening prompt.
            drop(invocation.args.splice(..0, args));
        }
        Ok(())
    }
}

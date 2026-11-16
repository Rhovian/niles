use super::{Mode, acp};
use crate::agent_window::shell_quote;
use crate::{agents, telemetry::SessionLink, util};
use anyhow::{Context, Result, ensure};
use camino::Utf8Path;
use std::env;

#[derive(Debug, Clone, Copy, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
pub(crate) enum Billing {
    Subscription,
    ApiKey,
}

#[derive(Debug, Clone, Copy, Default, clap::Args)]
pub(crate) struct Launch {
    #[arg(long, value_enum, default_value_t)]
    pub(crate) mode: Mode,
    /// Required for headless launches; never inferred from ambient credentials.
    #[arg(long, value_enum)]
    billing: Option<Billing>,
}
impl Launch {
    pub(crate) fn validate(self) -> Result<()> {
        ensure!(
            matches!(
                (self.mode, self.billing),
                (Mode::NativeTui, None) | (Mode::Headless, Some(Billing::Subscription))
            ),
            "headless launch requires --billing subscription; API-key adapters are not qualified yet"
        );
        Ok(())
    }
    pub(crate) fn qualify(self, spec: &agents::AgentSpec) -> Result<()> {
        if self.mode == Mode::Headless {
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
        if self.mode == Mode::Headless {
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
        if self.mode == Mode::Headless {
            let mut env = invocation.env.clone();
            env.extend(
                invocation
                    .spec
                    .profile()
                    .acp
                    .context("missing ACP profile")?
                    .adapter
                    .configure(&invocation.spec, primary, worker),
            );
            acp::prepare(
                dir,
                invocation.spec.family(),
                self.billing.context("headless billing is required")?,
                env,
            )?;
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
        }
        Ok(())
    }
}

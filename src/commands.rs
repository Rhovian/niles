use crate::harness::LaunchArgs;
use crate::skills::SkillName;
use crate::wait::{self, DEFAULT_INTERVAL, DEFAULT_TIMEOUT, Outcome};
use crate::worker::{self, Assignment, SpawnTree};
use crate::workspace_manifest::{Tier, setup::SetupSelection};
use anyhow::{Result, ensure};
use camino::Utf8PathBuf;
use clap::{ArgAction, Args};
use std::{io::Write, time::Duration};

pub(crate) struct Output<'a> {
    pub(crate) out: &'a mut dyn Write,
    pub(crate) err: &'a mut dyn Write,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Message(String);

impl TryFrom<String> for Message {
    type Error = anyhow::Error;

    fn try_from(text: String) -> Result<Self> {
        ensure!(!text.trim().is_empty(), "a message is required");
        Ok(Self(text))
    }
}

#[derive(Debug, Args)]
pub(crate) struct SpawnArgs {
    /// Worker task id used for window and metadata names.
    id: String,
    #[command(flatten)]
    launch: LaunchArgs,
    /// Installed primary skill to launch on a project tier.
    #[arg(long)]
    skill: SkillName,
    /// Inject an additional skill into the system prompt (repeatable).
    #[arg(long = "with")]
    with: Vec<SkillName>,
    /// Manifest tier whose primary to launch (standard by default).
    #[arg(long, value_enum, conflicts_with = "agent")]
    tier: Option<Tier>,
    /// Task label for grouping warm workers.
    #[arg(long = "task", value_name = "LABEL")]
    task_label: Option<String>,
    /// Agent id to launch; defaults to this skill's workspace manifest binding.
    #[arg(short, long)]
    agent: Option<String>,
    /// Run the agent in this directory while Niles tracks it from this workspace.
    #[arg(long, value_name = "PATH", conflicts_with = "worktree")]
    tree: Option<Utf8PathBuf>,
    /// Create or join a managed git worktree on this branch.
    #[arg(long, value_name = "BRANCH")]
    worktree: Option<String>,
    /// Create the worktree branch from this ref without tracking it.
    #[arg(long, value_name = "REF", requires = "worktree")]
    base: Option<String>,
    /// Check-in delay for this worker: a duration such as 1s, 90s, 5m or 1h. Defaults to
    /// this workspace's manifest `checkin`, then 5m. `0` or `off` arms none.
    #[arg(long, value_name = "DELAY")]
    checkin: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct Close {
    /// Worker id to close.
    #[arg(
        required_unless_present_any = ["task_label", "all"],
        conflicts_with_all = ["task_label", "all"]
    )]
    id: Option<String>,
    /// Close every live worker with this task label.
    #[arg(long = "task", value_name = "LABEL", conflicts_with = "all")]
    task_label: Option<String>,
    /// Close every live worker.
    #[arg(long, action = ArgAction::SetTrue)]
    all: bool,
}

#[derive(Debug, Args)]
pub(crate) struct Peek {
    /// Worker task id.
    id: String,
    /// Number of lines to capture. Use 0 for full tmux history.
    #[arg(short, long, default_value_t = crate::worker::DEFAULT_PEEK_LINES)]
    lines: usize,
    /// Disarm this worker's check-in after successfully capturing the pane.
    #[arg(long)]
    quiet: bool,
}

#[derive(Debug, Args)]
pub(crate) struct SendArgs {
    /// Check-in delay for this worker's next report: a duration such as 1s, 90s, 5m or 1h.
    /// Defaults to the manifest `checkin`, then 5m. `0` or `off` arms none.
    #[arg(long, value_name = "DELAY")]
    checkin: Option<String>,
    /// Worker task id.
    id: String,
}

#[derive(Debug, PartialEq, Eq, Args)]
pub(crate) struct Wait {
    /// Poll interval: a duration such as 500ms, 90s, 5m or 1h. Must be greater than zero.
    #[arg(
        long,
        default_value = wait::format_duration(DEFAULT_INTERVAL),
        value_parser = crate::wait::parse_interval
    )]
    interval: Duration,
    /// Maximum time to wait: a duration such as 500ms, 90s, 5m or 1h. `0` checks once without
    /// waiting.
    #[arg(
        long,
        default_value = wait::format_duration(DEFAULT_TIMEOUT),
        value_parser = crate::wait::parse_timeout
    )]
    timeout: Duration,
    /// Acknowledgement token from the last wake batch you received.
    #[arg(long)]
    ack: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct Target {
    /// Worker task id.
    pub(crate) id: String,
}

#[derive(Debug, Args)]
pub(crate) struct Approve {
    /// Worker task id.
    id: String,
    /// Pending permission request id.
    request: String,
    /// Offered option id. Omit to cancel the request.
    option: Option<String>,
}

impl Approve {
    pub(crate) fn run(self) -> Result<()> {
        worker::pane::approve(self.id, self.request, self.option)
    }
}

impl Close {
    pub(crate) fn run(self, output: &mut Output<'_>) -> Result<()> {
        worker::worker_close(self.id, self.task_label, self.all, output)
    }
}

impl Peek {
    pub(crate) fn run(self, output: &mut Output<'_>) -> Result<()> {
        worker::peek(self.id, self.lines, self.quiet, output)
    }
}

impl SpawnArgs {
    pub(crate) fn run(
        self,
        message: Message,
        selection: Option<&SetupSelection>,
        output: &mut Output<'_>,
    ) -> Result<worker::Spawned> {
        ensure!(
            self.tier.is_none() || self.agent.is_none(),
            "use tier or agent"
        );
        let tree = match (self.tree, self.worktree, self.base) {
            (Some(path), None, None) => Some(SpawnTree::Path(path)),
            (None, Some(branch), base) => Some(SpawnTree::Worktree { branch, base }),
            (None, None, None) => None,
            _ => anyhow::bail!("use tree or worktree; base requires worktree"),
        };
        worker::spawn(
            self.id,
            Assignment {
                launch: self.launch,
                skill: self.skill,
                with: self.with,
                tier: match self.tier {
                    Some(tier) => tier,
                    None => Tier::Standard,
                },
            },
            self.agent,
            worker::SpawnTask {
                label: self.task_label,
                text: message.0,
                checkin: self.checkin,
            },
            tree,
            selection,
            output,
        )
    }
}

impl SendArgs {
    pub(crate) fn run(
        self,
        message: Message,
        selection: Option<&SetupSelection>,
        output: &mut Output<'_>,
    ) -> Result<String> {
        worker::send(self.checkin, self.id, message.0, selection, output)
    }
}

impl Default for Wait {
    fn default() -> Self {
        Self {
            interval: DEFAULT_INTERVAL,
            timeout: DEFAULT_TIMEOUT,
            ack: None,
        }
    }
}

impl Wait {
    pub(crate) fn run(self) -> Result<Outcome> {
        wait::wait(self.interval, self.timeout, self.ack)
    }
}

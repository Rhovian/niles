use anyhow::{Context, Result, ensure};
use clap::{Subcommand, ValueEnum};

use crate::{
    util::append_line,
    wake::{self, WakeKind},
};

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// List this worker's menu skills and their sources.
    Skills,
    /// Print a skill from this worker's snapshotted skill lists.
    Skill { name: crate::skills::SkillName },
    /// Report to the lead through the worker's status log.
    #[command(
        after_help = "Only a status line wakes the lead. working wakes nobody. stage wakes nobody.\nFor a skill with staging configured, write report.md and run stage first (use your longest tool timeout), apply its post-work skills, then report done. Stage rounds are limited per worker task.\nWrite deliverables to report.md before done; it must exist and be non-empty.\ndone means hand back, not exit: stay open until the lead closes you.\nReport uncertainty with blocked or needs-decision.\nDetails must be non-empty and contain no control characters.\nNILES_AGENT_DIR must name an absolute worker directory with an existing status.log. done and stage also require valid meta.json."
    )]
    Status {
        kind: WorkerWake,
        #[arg(required = true, num_args = 1.., allow_hyphen_values = true)]
        detail: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum WorkerWake {
    Done,
    Failed,
    Blocked,
    NeedsDecision,
    Working,
    Stage,
}

impl From<WorkerWake> for WakeKind {
    fn from(kind: WorkerWake) -> Self {
        match kind {
            WorkerWake::Done => Self::Done,
            WorkerWake::Failed => Self::Failed,
            WorkerWake::Blocked => Self::Blocked,
            WorkerWake::NeedsDecision => Self::NeedsDecision,
            WorkerWake::Working => Self::Working,
            WorkerWake::Stage => Self::Stage,
        }
    }
}

pub(crate) fn run(command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::Skills => super::skills::discover(None),
        AgentCommand::Skill { name } => super::skills::discover(Some(&name)),
        AgentCommand::Status { kind, detail } => {
            let detail = detail.join(" ");
            ensure!(
                !detail.chars().any(char::is_control),
                "detail contains control characters"
            );
            let detail = detail.trim();
            ensure!(!detail.is_empty(), "detail must not be empty");
            let dir = super::meta::worker_dir_from_env()?;
            let log = wake::status_log_path(&dir);
            std::fs::metadata(&log).with_context(|| format!("missing status log: {log}"))?;
            match kind {
                WorkerWake::Stage => super::stage::stage(&dir, detail),
                WorkerWake::Done => super::stage::done(&dir, detail),
                WorkerWake::Working
                | WorkerWake::Blocked
                | WorkerWake::Failed
                | WorkerWake::NeedsDecision => append_line(&log, &wake::line(kind.into(), detail)),
            }
        }
    }
}

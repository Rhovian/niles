use crate::commands::Output;
use crate::skills::SkillName;
use crate::util::append_line;
use crate::wake::{self, WakeKind};
use anyhow::{Context, Result, ensure};
use clap::{Subcommand, ValueEnum};
use schemars::JsonSchema;
use serde::Deserialize;
use std::fs;

pub(crate) const ORCHESTRATION_CONTRACT: &str = "Niles orchestration contract (binding for every spawned worker, independent of skills):
Submit each dispatch's outcome via agent_status (CLI: niles agent status <kind> <detail>); never write status.log directly.
Only a status line wakes the lead. done, failed, blocked and needs-decision wake the lead; working wakes nobody. stage wakes nobody.
For a skill with staging configured, write report.md and run stage first, apply its post-work skills, then report done. Stage rounds are limited per worker task.
Write deliverables to report.md before done; it must exist and be non-empty.
done means hand back, not exit: stay open until the lead closes you. Follow-up arrives through send.
Report uncertainty with blocked or needs-decision; report inability to complete with failed.
Details must be non-empty and contain no control characters. Invalid submissions fail before appending a status line; correct the error and resubmit.
NILES_AGENT_DIR must name an absolute worker directory with an existing status.log. done and stage also require valid meta.json.
closed, gone and attention are reserved for Niles. A gone wake means a worker exited or stayed idle without reporting.
Finding tags, roles and routing are skill conventions, not requirements of this contract.";

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Print any installed skill, resolving workspace before global.
    Skill { name: SkillName },
    /// Report to the lead through the worker's status log.
    #[command(after_help = ORCHESTRATION_CONTRACT)]
    Status {
        kind: WorkerWake,
        #[arg(required = true, num_args = 1.., allow_hyphen_values = true)]
        detail: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
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

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Status {
    pub(crate) kind: WorkerWake,
    /// One line describing the dispatch outcome.
    pub(crate) detail: String,
}

impl Status {
    pub(crate) fn run(self, output: &mut Output<'_>) -> Result<()> {
        ensure!(
            !self.detail.chars().any(char::is_control),
            "detail contains control characters"
        );
        let detail = self.detail.trim();
        ensure!(!detail.is_empty(), "detail must not be empty");
        let dir = super::meta::worker_dir_from_env()?;
        let log = wake::status_log_path(&dir);
        fs::metadata(&log).with_context(|| format!("missing status log: {log}"))?;
        match self.kind {
            WorkerWake::Stage => super::stage::stage(&dir, detail, output),
            WorkerWake::Done => super::stage::done(&dir, detail),
            WorkerWake::Working
            | WorkerWake::Blocked
            | WorkerWake::Failed
            | WorkerWake::NeedsDecision => append_line(&log, &wake::line(self.kind.into(), detail)),
        }
    }
}

pub(crate) fn run(command: AgentCommand, output: &mut Output<'_>) -> Result<()> {
    match command {
        AgentCommand::Skill { name } => super::skills::load(&name),
        AgentCommand::Status { kind, detail } => Status {
            kind,
            detail: detail.join(" "),
        }
        .run(output),
    }
}

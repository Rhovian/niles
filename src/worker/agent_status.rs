use anyhow::{Context, Result, ensure};
use camino::Utf8PathBuf;
use clap::{Subcommand, ValueEnum};

use crate::{
    util::append_line,
    wake::{self, WakeKind},
};

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Report to the lead through the worker's status log.
    #[command(
        after_help = "Only a status line wakes the lead. working wakes nobody.\nWrite deliverables to report.md before done; it must exist and be non-empty.\ndone means hand back, not exit: stay open until the lead closes you.\nReport uncertainty with blocked or needs-decision.\nDetails must be non-empty and contain no control characters.\nNILES_AGENT_DIR must name an absolute worker directory with an existing status.log."
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
}

impl From<WorkerWake> for WakeKind {
    fn from(kind: WorkerWake) -> Self {
        match kind {
            WorkerWake::Done => Self::Done,
            WorkerWake::Failed => Self::Failed,
            WorkerWake::Blocked => Self::Blocked,
            WorkerWake::NeedsDecision => Self::NeedsDecision,
            WorkerWake::Working => Self::Working,
        }
    }
}

pub(crate) fn run(command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::Status { kind, detail } => {
            let detail = detail.join(" ");
            ensure!(
                !detail.chars().any(char::is_control),
                "detail contains control characters"
            );
            let detail = detail.trim();
            ensure!(!detail.is_empty(), "detail must not be empty");
            let dir = Utf8PathBuf::from(
                std::env::var("NILES_AGENT_DIR")
                    .context("NILES_AGENT_DIR is missing or invalid")?,
            );
            ensure!(
                dir.is_absolute() && dir.is_dir(),
                "NILES_AGENT_DIR must be an absolute worker directory: {dir}"
            );
            let log = wake::status_log_path(&dir);
            std::fs::metadata(&log).with_context(|| format!("missing status log: {log}"))?;
            let detail = if matches!(kind, WorkerWake::Done) {
                let report = super::meta::report_path(&dir);
                ensure!(
                    std::fs::metadata(&report)
                        .with_context(|| format!("missing report: {report}"))?
                        .len()
                        > 0,
                    "report is empty: {report}"
                );
                format!("{detail}; report: {report}")
            } else {
                detail.to_owned()
            };
            append_line(&log, &wake::line(kind.into(), &detail))
        }
    }
}

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

const BOOTSTRAP: &str = r#"# Niles {role} brief
You are niles worker `{id}`. Report with `niles agent status --help`.

id: {id}
task_label: {task_label}
project: {project}
report_file: {report_path}{design_record}

## Task

{task}
"#;

const SHARED_PREFACE: &str = "Work from this brief and the code. Do not read other agents' briefs, reports or notes in Niles state unless this brief or a message from the lead names one.

Deliverables go in the report file; the lead reads it, not your terminal.

Report uncertainty as uncertainty.";

const ROLE_WORKER_TEMPLATE: &str = include_str!("../../prompts/role_worker.md");
const ROLE_REVIEWER_TEMPLATE: &str = include_str!("../../prompts/role_reviewer.md");
const ROLE_SECURITY_TEMPLATE: &str = include_str!("../../prompts/role_security.md");
const ROLE_RESEARCH_TEMPLATE: &str = include_str!("../../prompts/role_research.md");
const ROLE_DESIGN_TEMPLATE: &str = include_str!("../../prompts/role_design.md");

/// Which role a spawned worker is playing.
///
/// A worker's brief is the bootstrap plus exactly one of these fragments. The
/// split exists so a role is never handed instructions addressed to a different one — most
/// importantly, so only the worker is told to run the project's checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum WorkerRole {
    /// Designs the record without editing the tree or running the gate.
    Design,
    /// Owns the change: implements it and runs the gate.
    Worker,
    /// Owns correctness, idiom and economy. Does not run the gate and does not audit.
    Reviewer,
    /// Owns the adversarial pass, commissioned only when the change is a security boundary.
    Security,
    /// Answers one question with cited evidence. Edits nothing and does not run the gate.
    Research,
}

impl WorkerRole {
    pub(super) fn binding(
        self,
        manifest: &crate::workspace_manifest::WorkspaceManifest,
    ) -> &crate::workspace_manifest::RoleBinding {
        match self {
            Self::Worker => &manifest.worker,
            Self::Reviewer => &manifest.reviewer,
            Self::Design => &manifest.design,
            Self::Security => &manifest.security,
            Self::Research => &manifest.research,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
            Self::Security => "security",
            Self::Research => "research",
        }
    }

    /// The full brief for this role: bootstrap, shared preface, then its own fragment.
    pub(crate) fn brief(self) -> String {
        format!("{BOOTSTRAP}\n{SHARED_PREFACE}\n\n{}", self.fragment())
    }

    pub(crate) fn fragment(self) -> &'static str {
        match self {
            Self::Design => ROLE_DESIGN_TEMPLATE,
            Self::Worker => ROLE_WORKER_TEMPLATE,
            Self::Reviewer => ROLE_REVIEWER_TEMPLATE,
            Self::Security => ROLE_SECURITY_TEMPLATE,
            Self::Research => ROLE_RESEARCH_TEMPLATE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [WorkerRole; 5] = [
        WorkerRole::Worker,
        WorkerRole::Reviewer,
        WorkerRole::Security,
        WorkerRole::Research,
        WorkerRole::Design,
    ];

    /// Every role points at the same self-describing status command.
    #[test]
    fn every_brief_carries_the_bootstrap_once() {
        for role in ALL {
            let brief = role.brief();
            assert_eq!(
                brief.matches("niles agent status --help").count(),
                1,
                "{} brief should carry the status command once",
                role.as_str()
            );
            // `{role}` is interpolated later, by `write_brief`.
            assert!(brief.contains("# Niles {role} brief"), "{brief}");
            assert!(brief.contains(&format!("You are the {}", role.as_str())));
        }
    }

    #[test]
    fn fragments_stay_short_enough_to_be_read() {
        for role in ALL {
            let lines = role.fragment().lines().count();
            assert!(
                lines <= 20,
                "{} fragment is {lines} lines; keep role briefs short",
                role.as_str()
            );
        }
    }
}

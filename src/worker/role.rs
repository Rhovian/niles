use clap::ValueEnum;

const ROLE_WORKER_TEMPLATE: &str = include_str!("../templates/role_worker.md");
const ROLE_REVIEWER_TEMPLATE: &str = include_str!("../templates/role_reviewer.md");

/// Which role a spawned worker is playing.
///
/// A worker's brief is the shared reporting contract plus exactly one of these fragments. The
/// split exists so a role is never handed instructions addressed to a different one — most
/// importantly, so only the worker is told to run the project's checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WorkerRole {
    /// Owns the change: implements it and runs the gate.
    Worker,
    /// Owns judgment about the change: reviews the diff and does not re-run the gate.
    Reviewer,
}

impl WorkerRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
        }
    }

    pub(crate) fn fragment(self) -> &'static str {
        match self {
            Self::Worker => ROLE_WORKER_TEMPLATE,
            Self::Reviewer => ROLE_REVIEWER_TEMPLATE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the split: the gate belongs to exactly one role.
    #[test]
    fn only_the_worker_is_told_to_run_the_gate() {
        assert!(WorkerRole::Worker.fragment().contains("You own the gate"));
        assert!(
            WorkerRole::Reviewer
                .fragment()
                .contains("Do not run the gate")
        );
        assert!(!WorkerRole::Worker.fragment().contains("Do not run the gate"));
        assert!(!WorkerRole::Reviewer.fragment().contains("You own the gate"));
    }

    /// A hardening finding needs a reachable attacker, or small changes accrete armour (#119).
    #[test]
    fn reviewer_must_name_the_attacker_before_a_hardening_finding() {
        let reviewer = WorkerRole::Reviewer.fragment();

        assert!(reviewer.contains("name the attacker"));
        assert!(reviewer.contains("it is not a finding"));
    }

    /// Review doctrine reaching an implementer is what made small features grow armour.
    #[test]
    fn worker_fragment_carries_no_review_doctrine() {
        let worker = WorkerRole::Worker.fragment();

        for reviewer_only in ["attacker", "amplification", "Review the delta", "hardening"] {
            assert!(
                !worker.contains(reviewer_only),
                "worker fragment should not mention {reviewer_only:?}"
            );
        }
    }

    #[test]
    fn fragments_stay_short_enough_to_be_read() {
        for role in [WorkerRole::Worker, WorkerRole::Reviewer] {
            let lines = role.fragment().lines().count();
            assert!(
                lines <= 20,
                "{} fragment is {lines} lines; keep role briefs short",
                role.as_str()
            );
        }
    }
}

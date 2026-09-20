use clap::ValueEnum;

/// The part of a worker brief that does not depend on the role: what it is, where its report
/// goes, and how it wakes the lead. Kept here rather than in a fourth template so the three
/// role files hold only what differs between roles, and the contract has one home.
const SHARED_CONTRACT: &str = r#"# Niles {role} brief

id: {id}
task_label: {task_label}
project: {project}
report_file: {report_path}

## Task

{task}

## Reporting

Appending a status line is the only thing that wakes the lead:

```sh
echo "done: <short result>; report: {report_path}" >> {status_path}
```

The states are `done:`, `blocked:`, `needs-decision:` and `failed:`, all in that form. `working:` lines are recorded but wake nobody — use them sparingly, for durable phase changes.

Deliverables go in the report file; the lead reads it, not your terminal.

Stay open after `done:` — it means you have something to hand back, not that you are exiting. The lead closes you with `niles close {id}`.

Report uncertainty as uncertainty. `blocked:` and `needs-decision:` are far cheaper than a confident wrong answer.
"#;

const ROLE_WORKER_TEMPLATE: &str = include_str!("../templates/role_worker.md");
const ROLE_REVIEWER_TEMPLATE: &str = include_str!("../templates/role_reviewer.md");
const ROLE_SECURITY_TEMPLATE: &str = include_str!("../templates/role_security.md");

/// Which role a spawned worker is playing.
///
/// A worker's brief is the shared reporting contract plus exactly one of these fragments. The
/// split exists so a role is never handed instructions addressed to a different one — most
/// importantly, so only the worker is told to run the project's checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WorkerRole {
    /// Owns the change: implements it and runs the gate.
    Worker,
    /// Owns correctness, idiom and economy. Does not run the gate and does not audit.
    Reviewer,
    /// Owns the adversarial pass, commissioned only when the change is a security boundary.
    Security,
}

impl WorkerRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
            Self::Security => "security",
        }
    }

    /// The full brief for this role: the shared contract, then the role's own fragment.
    pub(crate) fn brief(self) -> String {
        format!("{SHARED_CONTRACT}\n{}", self.fragment())
    }

    pub(crate) fn fragment(self) -> &'static str {
        match self {
            Self::Worker => ROLE_WORKER_TEMPLATE,
            Self::Reviewer => ROLE_REVIEWER_TEMPLATE,
            Self::Security => ROLE_SECURITY_TEMPLATE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [WorkerRole; 3] = [
        WorkerRole::Worker,
        WorkerRole::Reviewer,
        WorkerRole::Security,
    ];

    /// The whole point of the split: the gate belongs to exactly one role.
    #[test]
    fn only_the_worker_is_told_to_run_the_gate() {
        assert!(WorkerRole::Worker.fragment().contains("You own the gate"));
        for other in [WorkerRole::Reviewer, WorkerRole::Security] {
            assert!(
                other.fragment().contains("Do not run the gate"),
                "{} should be told not to run the gate",
                other.as_str()
            );
            assert!(!other.fragment().contains("You own the gate"));
        }
    }

    /// Security is its own pass. Fusing it into code review is what made small changes
    /// accrete hardening they did not need (#119).
    #[test]
    fn only_the_security_role_audits() {
        let security = WorkerRole::Security.fragment();
        assert!(security.contains("Name the attacker first"));
        assert!(security.contains("it is not a finding"));

        let reviewer = WorkerRole::Reviewer.fragment();
        assert!(reviewer.contains("Do not do a security review"));
        for audit_only in [
            "Name the attacker first",
            "amplification",
            "recursion depth",
        ] {
            assert!(
                !reviewer.contains(audit_only),
                "reviewer fragment should leave {audit_only:?} to the security pass"
            );
        }
    }

    /// The reviewer's lens is bounded, so it cannot wander into an architecture rewrite.
    #[test]
    fn reviewer_lens_is_correctness_idiom_and_economy() {
        let reviewer = WorkerRole::Reviewer.fragment();

        for lens in [
            "**Correctness.**",
            "**Idiom.**",
            "**Economy.**",
            "**Tests.**",
        ] {
            assert!(reviewer.contains(lens), "reviewer is missing {lens}");
        }
        assert!(reviewer.contains("Could this have been done in less code?"));
        assert!(reviewer.contains("Redundant cases, verbose setup"));
    }

    /// Doctrine addressed to another role is what the split exists to prevent.
    #[test]
    fn no_fragment_carries_another_role_doctrine() {
        let worker = WorkerRole::Worker.fragment();

        for foreign in ["attacker", "Review the delta", "hardening"] {
            assert!(
                !worker.contains(foreign),
                "worker fragment should not mention {foreign:?}"
            );
        }
    }

    /// The contract is shared, so every role carries it exactly once.
    #[test]
    fn every_brief_carries_the_shared_contract_once() {
        for role in ALL {
            let brief = role.brief();
            assert_eq!(
                brief.matches("## Reporting").count(),
                1,
                "{} brief should carry the reporting contract once",
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

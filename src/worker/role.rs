use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// The part of a worker brief that does not depend on the role: what it is, where its report
/// goes, and how it wakes the lead. Kept here rather than in a fourth template so the three
/// role files hold only what differs between roles, and the contract has one home.
const SHARED_CONTRACT: &str = r#"# Niles {role} brief

id: {id}
task_label: {task_label}
project: {project}
report_file: {report_path}{design_record}

## Task

{task}

## Reporting

Appending a status line is the only thing that wakes the lead:

```sh
echo "done: <short result>; report: {report_path}" >> {status_path}
```

The states are `done:`, `blocked:`, `needs-decision:` and `failed:`, all in that form. `working:` lines are recorded but wake nobody — use them sparingly, for durable phase changes.

Deliverables go in the report file; the lead reads it, not your terminal.

Work from this brief and the code. Do not read other agents' briefs, reports or notes under `.niles/` unless this brief or a message from the lead names one.

Stay open after `done:` — it means you have something to hand back, not that you are exiting. The lead closes you with `niles close {id}`.

Report uncertainty as uncertainty. `blocked:` and `needs-decision:` are far cheaper than a confident wrong answer.
"#;

const ROLE_WORKER_TEMPLATE: &str = include_str!("../templates/role_worker.md");
const ROLE_REVIEWER_TEMPLATE: &str = include_str!("../templates/role_reviewer.md");
const ROLE_SECURITY_TEMPLATE: &str = include_str!("../templates/role_security.md");
const ROLE_RESEARCH_TEMPLATE: &str = include_str!("../templates/role_research.md");
const ROLE_DESIGN_TEMPLATE: &str = include_str!("../templates/role_design.md");

/// Which role a spawned worker is playing.
///
/// A worker's brief is the shared reporting contract plus exactly one of these fragments. The
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
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
            Self::Security => "security",
            Self::Research => "research",
        }
    }

    /// The full brief for this role: the shared contract, then the role's own fragment.
    pub(crate) fn brief(self) -> String {
        format!("{SHARED_CONTRACT}\n{}", self.fragment())
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

    /// The whole point of the split: the gate belongs to exactly one role.
    #[test]
    fn only_the_worker_is_told_to_run_the_gate() {
        assert!(WorkerRole::Worker.fragment().contains("You own the gate"));
        for other in [
            WorkerRole::Reviewer,
            WorkerRole::Security,
            WorkerRole::Research,
            WorkerRole::Design,
        ] {
            assert!(
                other.fragment().contains("Do not run the gate"),
                "{} should be told not to run the gate",
                other.as_str()
            );
            assert!(!other.fragment().contains("You own the gate"));
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

    /// Reviewers judge the issue's own words, and prove the tests would notice a wrong guard.
    #[test]
    fn reviewers_judge_the_issue_and_mutate_its_guards() {
        for role in [WorkerRole::Reviewer, WorkerRole::Security] {
            let fragment = role.fragment();
            for phrase in [
                "the issue as written",
                "copy of the tree under the system temp directory",
                "choose mutations from the behaviour the change claims",
                "only when it changes behaviour the issue or record asks for",
                "only when it kills a mutation the existing suite lets survive",
                "name the existing test covering the guarantee",
                "order a wrong implementation would get right by accident",
                "database or library primitives tests the dependency",
                "Findings name the gap and mutation, never step-by-step test scripts",
                "without sleeps, wall-clock dependence or probe apps where a direct call suffices",
                "Behaviour every entry point shares gets a case in the repo's existing convention test",
                "not a prescribed fix",
            ] {
                assert!(fragment.contains(phrase), "{}: {phrase}", role.as_str());
            }
            for obsolete in [
                "probe that demonstrates a guarantee is reported as a required test",
                "Each surviving mutation is a test finding",
                "Each surviving mutation is a finding",
            ] {
                assert!(!fragment.contains(obsolete), "{obsolete}");
            }
        }
        let reviewer = WorkerRole::Reviewer.fragment();
        for phrase in [
            "test the requirement itself",
            "comparison, bound, lock acquisition, constraint and counter in the diff",
            "skip scaffolding that exists only to implement a reviewer-prescribed fix",
            "expected literals copied from editable content or config",
            "a full pass on that commit, including economy and tests, before push",
            "an unexplained material overrun is an economy finding",
            "cites in the code the pinned dependency's source or docs",
        ] {
            assert!(reviewer.contains(phrase), "{phrase}");
        }
        let security = WorkerRole::Security.fragment();
        for phrase in [
            "For every new or changed authorisation or tenancy predicate",
            "guarantees count as asked-for behaviour even when the issue or record is silent",
            "a surviving mutation or a working attack",
            "never one that passes both before and after",
        ] {
            assert!(security.contains(phrase), "{phrase}");
        }
    }

    #[test]
    fn worker_builds_to_the_record_without_a_size_stop() {
        let fragment = WorkerRole::Worker.fragment();
        assert!(fragment.contains("the record wins"));
        assert!(!fragment.contains("size estimate"));
        for phrase in [
            "cite in the code the pinned dependency's source or docs",
            "Overlapping reviewer and security findings get one test per gap",
            "why at the site as a comment where the repo allows one, and in the PR body",
            "Your `done:` states production and test lines added",
            "without a record, report the counts and say there is no target",
        ] {
            assert!(fragment.contains(phrase), "{phrase}");
        }
    }

    #[test]
    fn designers_record_the_design_and_its_constraints() {
        let fragment = WorkerRole::Design.fragment();
        for phrase in [
            "Done-when map",
            "race accepted",
            "never edit the tree",
            "mark each deliberate departure from a repo rule and its why",
        ] {
            assert!(
                fragment.contains(phrase),
                "missing design instruction: {phrase}"
            );
        }
    }

    #[test]
    fn design_routing_is_optional_and_critique_is_bounded() {
        let lead = include_str!("../templates/lead_brief.md");
        for phrase in [
            "without a `code`, `design` or `policy` tag goes back to its author to tag or drop",
            "author's own caveats, quoted",
            "all reviewer and security `code` findings word for word to the worker in one message",
            "For work that had a reviewer",
            "a full pass on that commit, including economy and tests, before push",
            "An operator ruling settles whether it is wanted, not how it is built",
            "poke holes in it unprompted",
            "recommend do, narrow or close, then wait for the operator's call",
            "ask the operator: \"Do you want a design?\" Wait for the answer",
            "If yes, run design; if no, send the issue to a worker",
            "Do not infer the answer from the issue",
            "Spawn one designer by default",
            "A second designer runs only on operator request",
            "send both designers the same message naming both report paths",
            "put every contested point to the operator",
            "Do not run another critique or require agreement handbacks from idle designers",
            "You may flag scope creep by quoting the issue's own wording",
            "You still do not rule on design disputes",
        ] {
            assert!(lead.contains(phrase), "missing lead instruction: {phrase}");
        }
        let designer = WorkerRole::Design.fragment();
        assert!(designer.contains("exactly one critique"));
        assert!(designer.contains("No further critique rounds or agreement handbacks"));
        assert!(!lead.contains("issue leaves the destination open"));
        assert!(!lead.contains("critique and rulings repeat"));
        assert!(!designer.contains("through the same loop"));
    }

    /// The shared contract is the only channel that tells a worker how to spell a status line, so
    /// it must name every non-`Closed` wake state exactly as `wake.rs` parses it. `Closed` is
    /// niles-owned and withheld from workers, so it is deliberately absent here.
    #[test]
    fn shared_contract_names_every_worker_wake_state() {
        use crate::wake::WakeKind;

        for kind in [
            WakeKind::Done,
            WakeKind::Failed,
            WakeKind::Blocked,
            WakeKind::NeedsDecision,
            WakeKind::Working,
        ] {
            let token = format!("{kind}:");
            assert!(
                SHARED_CONTRACT.contains(&token),
                "SHARED_CONTRACT must instruct workers to emit {token}"
            );
        }
    }
}

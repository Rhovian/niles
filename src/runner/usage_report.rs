use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};

use crate::{
    state::{RunState, StepKind, StepRecord, StepStatus},
    usage::{self, UsageAgent, UsageDisplay, UsageRollup, UsageSnapshotInput, UsageSubject},
};

pub(in crate::runner) fn print_usage_blocks(state: &RunState, run_dir: &Utf8Path) {
    let steps_dir = run_dir.join("steps");
    println!(
        "usage[{}]{{index,status,wall,turns,total,input,cache_create,cache_read,cached,output,reasoning}}:",
        state.steps.len()
    );

    let now = Utc::now();
    let mut rollup = UsageRollup::default();
    for step in &state.steps {
        let usage = step_usage(state, step, &steps_dir, now);
        rollup.add(&usage);
        println!(
            "  {},{},{},{},{},{},{},{},{},{},{}",
            step.index,
            usage.status,
            usage::format_wall(usage.wall_seconds),
            usage::format_optional(usage.turns),
            usage::format_optional(usage.totals.total),
            usage::format_optional(usage.totals.input),
            usage::format_optional(usage.totals.cache_create),
            usage::format_optional(usage.totals.cache_read),
            usage::format_optional(usage.totals.cached),
            usage::format_optional(usage.totals.output),
            usage::format_optional(usage.totals.reasoning)
        );
    }

    println!(
        "run_usage{{steps,total,input,cache_create,cache_read,cached,output,reasoning,wall}}:"
    );
    println!(
        "  {},{},{},{},{},{},{},{},{}",
        rollup.available_subjects,
        usage::format_optional(rollup.totals.total),
        usage::format_optional(rollup.totals.input),
        usage::format_optional(rollup.totals.cache_create),
        usage::format_optional(rollup.totals.cache_read),
        usage::format_optional(rollup.totals.cached),
        usage::format_optional(rollup.totals.output),
        usage::format_optional(rollup.totals.reasoning),
        usage::format_rollup_wall(rollup.wall_seconds)
    );
}

fn step_usage(
    state: &RunState,
    step: &StepRecord,
    steps_dir: &Utf8Path,
    now: DateTime<Utc>,
) -> UsageDisplay {
    let wall_seconds = step_wall_seconds(step, now);
    if matches!(step.status, StepStatus::Pending) {
        return UsageDisplay::pending(wall_seconds);
    }

    if let Some(path) = existing_usage_path(step, steps_dir) {
        return match usage::read_usage_snapshot(&path) {
            Ok(snapshot) => UsageDisplay::from_snapshot(&snapshot, false),
            Err(err) => {
                eprintln!("warning: failed to read usage snapshot {path}: {err:#}");
                UsageDisplay::unavailable(wall_seconds)
            }
        };
    }

    if step.usage_attribution.is_some() {
        let snapshot = usage::compute_usage_snapshot(&UsageSnapshotInput {
            subject: UsageSubject::RunStep {
                run_id: state.id.clone(),
                index: step.index,
                role: step.role.clone(),
                label: step.label.clone(),
            },
            agent: UsageAgent {
                spec: step.label.clone(),
                family: step.agent_family.clone(),
                model: step.model.clone(),
                effort: step.effort.clone(),
            },
            attribution: step.usage_attribution.clone(),
            started_at: step.started_at,
            finished_at: now,
            output_path: usage::step_usage_path(steps_dir, step.index, &step.label),
        });
        return UsageDisplay::from_snapshot(&snapshot, true);
    }

    if matches!(step.status, StepStatus::Running) && matches!(step.kind, StepKind::Agent) {
        UsageDisplay::pending(wall_seconds)
    } else {
        UsageDisplay::unavailable(wall_seconds)
    }
}

fn existing_usage_path(step: &StepRecord, steps_dir: &Utf8Path) -> Option<Utf8PathBuf> {
    if let Some(path) = &step.usage
        && path.exists()
    {
        return Some(path.clone());
    }

    let fallback = usage::step_usage_path(steps_dir, step.index, &step.label);
    if fallback.exists() {
        return Some(fallback);
    }
    None
}

fn step_wall_seconds(step: &StepRecord, now: DateTime<Utc>) -> Option<i64> {
    let started_at = step.started_at?;
    let finished_at = match step.finished_at {
        Some(finished_at) => finished_at,
        None => {
            if matches!(step.status, StepStatus::Running) {
                now
            } else {
                return None;
            }
        }
    };
    Some((finished_at - started_at).num_seconds().max(0))
}

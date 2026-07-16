use anyhow::{Context, Result, bail};

use crate::{
    agent_window,
    store::{read_state, resolve_run_dir},
    tmux::WindowTarget,
};

use super::meta::read_meta;

pub(crate) const DEFAULT_PEEK_LINES: usize = 2000;
enum PaneTarget {
    Worker {
        id: String,
        target: WindowTarget,
    },
    RunStep {
        run: String,
        index: usize,
        window_name: String,
    },
}
pub fn peek(
    id: Option<String>,
    run: Option<String>,
    index: Option<usize>,
    lines: usize,
) -> Result<()> {
    let target = resolve_peek_target(id, run, index)?;
    print!("{}", target.capture(lines)?);
    Ok(())
}

pub fn send(
    run: Option<String>,
    index: Option<usize>,
    target_and_message: Vec<String>,
) -> Result<()> {
    if target_and_message.is_empty() {
        bail!("send requires a message");
    }

    let (target, message) = resolve_send_target(run, index, target_and_message)?;
    let message = message.join(" ");
    target.send(&message)?;
    println!("sent: {}", target.label());
    Ok(())
}

impl PaneTarget {
    fn capture(&self, lines: usize) -> Result<String> {
        match self {
            PaneTarget::Worker { target, .. } => agent_window::capture_target(target, lines),
            PaneTarget::RunStep { window_name, .. } => {
                agent_window::capture_window(window_name, lines)
            }
        }
    }

    fn send(&self, message: &str) -> Result<()> {
        match self {
            PaneTarget::Worker { target, .. } => agent_window::send_target(target, message),
            PaneTarget::RunStep { window_name, .. } => {
                agent_window::send_window(window_name, message)
            }
        }
    }

    fn label(&self) -> String {
        match self {
            PaneTarget::Worker { id, .. } => id.clone(),
            PaneTarget::RunStep { run, index, .. } => format!("{run} step {index}"),
        }
    }
}

fn resolve_peek_target(
    id: Option<String>,
    run: Option<String>,
    index: Option<usize>,
) -> Result<PaneTarget> {
    let has_step_target = run.is_some() || index.is_some();
    match (id, has_step_target) {
        (Some(_), true) => bail!("use either a worker id or --run <id> --index <N>, not both"),
        (Some(id), false) => worker_target(id),
        (None, true) => run_step_target(run, index),
        (None, false) => bail!("peek requires a worker id or --run <id> --index <N>"),
    }
}

fn resolve_send_target(
    run: Option<String>,
    index: Option<usize>,
    target_and_message: Vec<String>,
) -> Result<(PaneTarget, Vec<String>)> {
    if run.is_some() || index.is_some() {
        return Ok((run_step_target(run, index)?, target_and_message));
    }

    let mut parts = target_and_message.into_iter();
    let id = parts
        .next()
        .context("send requires a worker id or --run <id> --index <N>")?;
    let message = parts.collect::<Vec<_>>();
    if message.is_empty() {
        bail!("send requires a message");
    }
    Ok((worker_target(id)?, message))
}

fn worker_target(id: String) -> Result<PaneTarget> {
    let meta = read_meta(&id)?;
    let target = WindowTarget::parse(&meta.window)
        .with_context(|| format!("worker {id} metadata has invalid tmux window target"))?;
    Ok(PaneTarget::Worker { id, target })
}

fn run_step_target(run: Option<String>, index: Option<usize>) -> Result<PaneTarget> {
    let run = run.context("run-step target requires --run <id>")?;
    let index = index.context("run-step target requires --index <N>")?;
    if index == 0 {
        bail!("step index must be >= 1");
    }

    let run_dir = resolve_run_dir(&run)?;
    let state = read_state(&run_dir)?;
    let run_id = state.id.clone();
    let step = state
        .steps
        .iter()
        .find(|step| step.index == index)
        .with_context(|| format!("step {index} not found in run {run_id}"))?;
    let window_name = step
        .window
        .clone()
        .with_context(|| format!("step {index} in run {run_id} has no recorded window"))?;

    Ok(PaneTarget::RunStep {
        run: run_id,
        index,
        window_name,
    })
}

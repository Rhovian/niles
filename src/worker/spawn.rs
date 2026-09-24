use std::fs;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    agent_window, agents,
    config::spec::load_project_config_from,
    store,
    tmux::{self, WindowTarget},
    util::{absolute_existing_file, current_dir_utf8, remove_dir_all_if_exists, render_template},
    wake, watch, workspace_manifest,
};

use super::{
    archive::archive_worker_dir,
    list::UNLABELED_TASK_LABEL,
    meta::{WorkerMeta, report_path, write_meta},
    resolve::resolve_live_worker_if_exists,
    role::WorkerRole,
    snapshot::status_log_len,
    validation::{validate_id, validate_task_label},
};

pub fn spawn(
    id: String,
    role: WorkerRole,
    task_label: Option<String>,
    agent: Option<String>,
    brief: Option<Utf8PathBuf>,
    task: Vec<String>,
    checkin: Option<String>,
) -> Result<()> {
    validate_id(&id)?;
    if let Some(label) = &task_label {
        validate_task_label(label)?;
    }
    if brief.is_none() && task.is_empty() {
        bail!("spawn requires either --brief or task text");
    }

    let project = current_dir_utf8()?;
    let agent = resolve_agent(&project, role, agent)?;
    let config = load_project_config_from(&project)?;
    // The one launch decision, resolved before any worker state is written: unknown agent names
    // and models the family does not offer are rejected here, by the same resolver the lead uses.
    let agent_config = agents::config_for(&config.agents, &agent)?;
    let invocation = agents::invocation(&agent, agent_config, agents::InvocationDefaults::Worker)?;
    let agent_spec = &invocation.spec;
    if resolve_live_worker_if_exists(&id)?.is_some() {
        bail!("worker id '{id}' already exists");
    }
    // Resolved before any worker state is written: a spawn that cannot place a window should
    // leave no half-built worker directory behind.
    let session = tmux::current_session()?;

    let dir = store::workspace_worker_dir(&project, &id)?;
    if dir.exists() {
        archive_worker_dir(&id, &dir, Utc::now())?;
    }
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;

    let brief_path = match brief {
        Some(path) => absolute_existing_file(&path, "brief")?,
        None => {
            let path = dir.join("brief.md");
            write_brief(&BriefInputs {
                dir: &dir,
                path: &path,
                id: &id,
                role,
                task_label: task_label.as_deref(),
                project: &project,
                task: &task.join(" "),
            })?;
            path
        }
    };

    let launch_path = dir.join("launch.sh");
    let status_path = wake::status_log_path(&dir);
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&status_path)
        .with_context(|| format!("failed to create {status_path}"))?;

    let window_name = agent_window::worker_window_name(&id);
    // The status log exists and is empty; read it as the check-in baseline *before* the window is
    // launched, so a line the worker writes on its way up can still answer this dispatch.
    let armed_len = status_log_len(&dir)?;
    let target = match spawn_worker_window(
        &session,
        &window_name,
        &project,
        &invocation,
        &brief_path,
        &launch_path,
        &status_path,
    ) {
        Ok(target) => target,
        Err(err) => {
            if let Err(cleanup_err) = cleanup_failed_spawn(&dir, None) {
                return Err(err).context(format!(
                    "failed to launch worker {id}; additionally failed to clean up partial worker at {dir}: {cleanup_err}"
                ));
            }
            return Err(err).context(format!(
                "failed to launch worker {id}; cleaned up partial worker at {dir}"
            ));
        }
    };

    let meta = WorkerMeta {
        id: id.clone(),
        agent,
        agent_family: agent_spec.tier().map(|tier| tier.family),
        model: agent_spec.model().map(str::to_owned),
        effort: agent_spec.effort().map(str::to_owned),
        task_label,
        created_at: Some(Utc::now()),
        project: project.clone(),
        window: target.render(),
        brief: brief_path,
        launch: launch_path,
    };
    if let Err(err) =
        tag_worker_window(&target, &project, &id).and_then(|()| write_meta(&dir, &meta))
    {
        if let Err(cleanup_err) = cleanup_failed_spawn(&dir, Some(&target)) {
            return Err(err).context(format!(
                "failed to finish launching worker {id}; additionally failed to clean up launched worker at {target}: {cleanup_err}"
            ));
        }
        return Err(err).context(format!(
            "failed to finish launching worker {id}; cleaned up launched worker at {target}"
        ));
    }

    // The lead arms the check-in and is the only one who does: a worker can disarm it by
    // reporting, never by staying quiet about it.
    let armed = watch::arm_checkin(&dir, checkin.as_deref(), armed_len, Utc::now())?;

    println!("spawned: {id}");
    println!("window: {window_name}");
    println!("agent: {}", meta.agent);
    print_worker_tier(&meta);
    if let Some(label) = &meta.task_label {
        println!("task: {label}");
    }
    println!("brief: {}", meta.brief);
    // wait first: it is the command the lead reaches for next, and it was the one omission here.
    println!("wait: niles wait {id}");
    println!("peek: niles peek {id}");
    println!("report: niles report {id}");
    println!("send: niles send {id} <message>");
    println!("close: niles close {id}");
    if let Some(label) = &meta.task_label {
        println!("close_task: niles close --task {label}");
    }
    println!("workers: niles workers");
    match armed {
        Some(delay) => {
            println!("checkin: {}", watch::describe_delay(delay));
            println!("quiet: niles quiet {id}");
        }
        None => println!("checkin: off"),
    }

    Ok(())
}

fn resolve_agent(project: &Utf8Path, role: WorkerRole, agent: Option<String>) -> Result<String> {
    if let Some(agent) = agent {
        return Ok(agent);
    }

    let path = workspace_manifest::manifest_path(project);
    let role_name = role.as_str();
    let manifest = workspace_manifest::load(project)
        .with_context(|| format!("cannot resolve agent for role '{role_name}' from {path}"))?
        .with_context(|| {
            format!("cannot resolve agent for role '{role_name}': manifest {path} does not exist; specify --agent or configure the manifest")
        })?;
    let agent = match role {
        WorkerRole::Worker => manifest.worker,
        WorkerRole::Reviewer => manifest.reviewer,
        WorkerRole::Security => manifest.security,
    };
    if agent.trim().is_empty() {
        bail!(
            "no agent configured for role '{role_name}' in manifest {path}; specify --agent or configure the role"
        );
    }
    Ok(agent)
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;

fn spawn_worker_window(
    session: &tmux::SessionName,
    window_name: &str,
    project: &Utf8Path,
    invocation: &agents::AgentInvocation,
    brief_path: &Utf8Path,
    launch_path: &Utf8Path,
    status_path: &Utf8Path,
) -> Result<WindowTarget> {
    agent_window::spawn_agent_window_in_session(
        session,
        window_name,
        project,
        invocation,
        &agent_window::WorkerPaths {
            brief: brief_path,
            launch: launch_path,
            status: status_path,
        },
    )
}

fn tag_worker_window(target: &WindowTarget, project: &Utf8Path, id: &str) -> Result<()> {
    tmux::set_window_option(target, "@niles-project", project.as_str())?;
    tmux::set_window_option(target, "@niles-worker-id", id)?;
    // Keep the pane after the agent exits. Whatever killed it — a trust prompt, a crash — is on
    // that pane, and destroying the window destroys the only evidence of why.
    tmux::set_window_option(target, "remain-on-exit", "on")
}

fn print_worker_tier(meta: &WorkerMeta) {
    if let Some(family) = &meta.agent_family {
        println!("agent_family: {family}");
    }
    if let Some(model) = &meta.model {
        println!("model: {model}");
    }
    if let Some(effort) = &meta.effort {
        println!("effort: {effort}");
    }
}

/// Everything the worker brief interpolates.
struct BriefInputs<'a> {
    dir: &'a Utf8Path,
    path: &'a Utf8Path,
    id: &'a str,
    role: WorkerRole,
    task_label: Option<&'a str>,
    project: &'a Utf8Path,
    task: &'a str,
}

fn write_brief(inputs: &BriefInputs<'_>) -> Result<()> {
    let &BriefInputs {
        dir,
        path,
        id,
        role,
        task_label,
        project,
        task,
    } = inputs;
    let status_path = wake::status_log_path(dir);
    let report_file = report_path(dir);
    let task_label = match task_label {
        Some(task_label) => task_label,
        None => UNLABELED_TASK_LABEL,
    };
    // Every worker gets the shared contract plus exactly one role fragment, so a worker is not
    // handed doctrine addressed to a role it is not playing.
    let body = render_template(
        &role.brief(),
        &[
            ("{id}", id),
            ("{role}", role.as_str()),
            ("{task_label}", task_label),
            ("{project}", project.as_str()),
            ("{status_path}", status_path.as_str()),
            ("{report_path}", report_file.as_str()),
            ("{task}", task),
        ],
    );
    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

fn cleanup_failed_spawn(dir: &Utf8Path, target: Option<&WindowTarget>) -> Result<()> {
    let mut failures = Vec::new();
    if let Some(target) = target
        && let Err(err) = agent_window::close_target(target)
    {
        failures.push(format!("failed to kill tmux window {target}: {err:#}"));
    }
    if let Err(err) = remove_dir_all_if_exists(dir) {
        failures.push(format!(
            "failed to remove partial worker dir {dir}: {err:#}"
        ));
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("; "))
    }
}

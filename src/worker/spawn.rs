use std::fs;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    agent_window, agents,
    config::spec::load_project_config_from,
    session, store,
    tmux::{self, WindowTarget},
    usage,
    util::{
        absolute_existing_dir, absolute_existing_file, current_dir_utf8, remove_dir_all_if_exists,
        render_template,
    },
    wake,
};

use super::{
    archive::archive_worker_dir,
    list::UNLABELED_TASK_LABEL,
    meta::{WorkerMeta, report_path, write_meta},
    resolve::resolve_live_worker_if_exists,
    validation::{validate_id, validate_task_label},
};

const WORKER_BRIEF_TEMPLATE: &str = include_str!("../templates/worker_brief.md");

pub fn spawn(
    id: String,
    task_label: Option<String>,
    project: Utf8PathBuf,
    agent: String,
    brief: Option<Utf8PathBuf>,
    task: Vec<String>,
    allow_cli_mismatch: bool,
) -> Result<()> {
    validate_id(&id)?;
    if let Some(label) = &task_label {
        validate_task_label(label)?;
    }
    if brief.is_none() && task.is_empty() {
        bail!("spawn requires either --brief or task text");
    }

    let current_workspace = current_dir_utf8()?;
    let requested_project = absolute_existing_dir(&project, "project")?;
    require_current_workspace_project(&current_workspace, &requested_project)?;
    let project = current_workspace;
    let config = load_project_config_from(&project)?;
    let agent_config = agents::config_for(&config.agents, &agent)?;
    let agent_spec = agents::capabilities::validate_agent(
        &project,
        &agent,
        agent_config,
        agents::InvocationDefaults::Worker,
    )?;
    agents::version::preflight_agent(
        &agent,
        agent_config,
        agents::InvocationDefaults::Worker,
        allow_cli_mismatch,
    )?;
    if resolve_live_worker_if_exists(&id)?.is_some() {
        bail!("worker id '{id}' already exists");
    }

    let dir = store::workspace_worker_dir(&project, &id)?;
    if dir.exists() {
        archive_worker_dir(&id, &dir, Utc::now())?;
    }
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;

    let brief_path = match brief {
        Some(path) => absolute_existing_file(&path, "brief")?,
        None => {
            let path = dir.join("brief.md");
            write_brief(
                &dir,
                &path,
                &id,
                task_label.as_deref(),
                &project,
                &agent,
                &task.join(" "),
            )?;
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
    let launched_at = Utc::now();
    let usage_attribution =
        usage::attribution_for_family(agent_spec.family(), &project, launched_at, Some(1));
    let session = session::resolve_workspace_tmux_session(&project)?;
    tmux::ensure_session_exists(&session)?;
    let target = match spawn_worker_window(
        &session,
        &window_name,
        &project,
        &agent,
        &brief_path,
        &launch_path,
        &usage_attribution,
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
        usage_attribution: Some(usage_attribution),
        usage: None,
        task_label,
        created_at: Some(launched_at),
        project: project.clone(),
        window: target.render(),
        brief: brief_path,
        launch: launch_path,
        status: Some(status_path),
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

    println!("spawned: {id}");
    println!("window: {window_name}");
    println!("agent: {}", meta.agent);
    print_worker_tier(&meta);
    if let Some(label) = &meta.task_label {
        println!("task: {label}");
    }
    println!("brief: {}", meta.brief);
    println!("peek: niles peek {id}");
    println!("report: niles report {id}");
    println!("send: niles send {id} <message>");
    println!("close: niles worker-close {id}");
    if let Some(label) = &meta.task_label {
        println!("close_task: niles worker-close --task {label}");
    }
    println!("workers: niles workers");

    Ok(())
}

fn require_current_workspace_project(
    current_workspace: &Utf8Path,
    requested_project: &Utf8Path,
) -> Result<()> {
    let current = fs::canonicalize(current_workspace)
        .with_context(|| format!("failed to canonicalize current workspace {current_workspace}"))?;
    let requested = fs::canonicalize(requested_project)
        .with_context(|| format!("failed to canonicalize project {requested_project}"))?;
    if current != requested {
        bail!("spawn --project must be the current workspace; cd there and spawn");
    }
    Ok(())
}

fn spawn_worker_window(
    session: &tmux::SessionName,
    window_name: &str,
    project: &Utf8Path,
    agent: &str,
    brief_path: &Utf8Path,
    launch_path: &Utf8Path,
    usage_attribution: &usage::UsageAttribution,
) -> Result<WindowTarget> {
    agent_window::spawn_agent_window_in_session(
        session,
        window_name,
        project,
        agent,
        project,
        brief_path,
        launch_path,
        Some(usage_attribution),
    )
}

fn tag_worker_window(target: &WindowTarget, project: &Utf8Path, id: &str) -> Result<()> {
    tmux::set_window_option(target, "@niles-project", project.as_str())?;
    tmux::set_window_option(target, "@niles-worker-id", id)
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

fn write_brief(
    dir: &Utf8Path,
    path: &Utf8Path,
    id: &str,
    task_label: Option<&str>,
    project: &Utf8Path,
    agent: &str,
    task: &str,
) -> Result<()> {
    let status_path = wake::status_log_path(dir);
    let report_file = report_path(dir);
    let wake_examples = wake::worker_contract_examples(&status_path);
    let task_label = match task_label {
        Some(task_label) => task_label,
        None => UNLABELED_TASK_LABEL,
    };
    let body = render_template(
        WORKER_BRIEF_TEMPLATE,
        &[
            ("{id}", id),
            ("{task_label}", task_label),
            ("{project}", project.as_str()),
            ("{agent}", agent),
            ("{status_path}", status_path.as_str()),
            ("{report_path}", report_file.as_str()),
            ("{task}", task),
            ("{wake_examples}", &wake_examples),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_brief_mentions_followup_review_scope() {
        assert!(WORKER_BRIEF_TEMPLATE.contains("follow-up review after a fix"));
    }

    #[test]
    fn worker_brief_mentions_gate_policy() {
        assert!(WORKER_BRIEF_TEMPLATE.contains("runs the gate"));
        assert!(WORKER_BRIEF_TEMPLATE.contains("times out mid-run"));
        assert!(WORKER_BRIEF_TEMPLATE.contains("do not re-run deterministic gates"));
        assert!(WORKER_BRIEF_TEMPLATE.contains("audit the runner's transcript gate output"));
        assert!(
            WORKER_BRIEF_TEMPLATE
                .contains("Never use `working:`/`done:` to imply unrun gates passed")
        );
    }

    #[test]
    fn worker_brief_mentions_modularity_standard() {
        assert!(WORKER_BRIEF_TEMPLATE.contains("split it by responsibility"));
    }
}

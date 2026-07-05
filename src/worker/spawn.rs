use std::fs;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    agent_window, agents,
    config::spec::load_project_config_from,
    store,
    util::{
        absolute_existing_dir, absolute_existing_file, remove_dir_all_if_exists,
        remove_file_if_exists, render_template,
    },
    wake,
};

use super::{
    archive::archive_worker_dir,
    list::UNLABELED_TASK_LABEL,
    meta::{WorkerMeta, meta_path, report_path, write_meta},
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

    let project = absolute_existing_dir(&project, "project")?;
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

    store::register_worker_location(&id, &project, &dir)?;

    let window_name = agent_window::worker_window_name(&id);
    let target = match agent_window::spawn_agent_window(
        &window_name,
        &project,
        &agent,
        &project,
        &brief_path,
        &launch_path,
    ) {
        Ok(target) => target,
        Err(err) => {
            if let Err(cleanup_err) = cleanup_failed_spawn(&id, &project, &dir) {
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
        project,
        window: target.clone(),
        brief: brief_path,
        launch: launch_path,
        status: Some(status_path),
    };
    write_meta(&dir, &meta)?;

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

fn cleanup_failed_spawn(id: &str, project: &Utf8Path, dir: &Utf8Path) -> Result<()> {
    store::unregister_worker_location(id, None, Some(project), Some(dir))?;
    remove_file_if_exists(&meta_path(dir))?;
    remove_dir_all_if_exists(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_brief_mentions_followup_review_scope() {
        assert!(WORKER_BRIEF_TEMPLATE.contains("follow-up review after a fix"));
    }

    #[test]
    fn worker_brief_mentions_modularity_standard() {
        assert!(WORKER_BRIEF_TEMPLATE.contains("split it by responsibility"));
    }
}

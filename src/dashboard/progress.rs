use anyhow::Result;
#[cfg(test)]
use camino::Utf8Path;

use crate::{
    state::{RunState, StepRecord, StepStatus},
    store,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunProgress {
    pub(crate) run_id: String,
    pub(crate) short_run_id: String,
    pub(crate) run_status: String,
    pub(crate) step_total: usize,
    pub(crate) step: Option<RunProgressStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunProgressStep {
    pub(crate) index: usize,
    pub(crate) role: Option<String>,
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) status: String,
}

#[cfg(test)]
pub(crate) fn collect(workspace: &Utf8Path) -> Result<Option<RunProgress>> {
    let Some(run_dir) = store::resolve_latest_run_dir_from(workspace)? else {
        return Ok(None);
    };
    let state = store::read_state(&run_dir)?;
    from_state(&state)
}

pub(crate) fn from_state(state: &RunState) -> Result<Option<RunProgress>> {
    let step = match focus_step_index(state) {
        Some(index) => Some(progress_step(store::selected_step(state, Some(index))?)),
        None => None,
    };

    Ok(Some(RunProgress {
        run_id: state.id.clone(),
        short_run_id: short_run_id(&state.id),
        run_status: state.status.to_string(),
        step_total: state.steps.len(),
        step,
    }))
}

pub(crate) fn progress_line(progress: &RunProgress) -> String {
    match &progress.step {
        Some(step) => {
            let mut step_label = String::new();
            if let Some(role) = &step.role {
                step_label.push_str(role);
                step_label.push(' ');
            }
            step_label.push_str(&step.kind);
            step_label.push(' ');
            step_label.push_str(&step.label);
            step_label.push(' ');
            step_label.push_str(&step.status);

            format!(
                "run: {} {} | step {}/{} | {}",
                progress.short_run_id,
                progress.run_status,
                step.index,
                progress.step_total,
                step_label
            )
        }
        None => format!(
            "run: {} {} | steps 0/0",
            progress.short_run_id, progress.run_status
        ),
    }
}

fn progress_step(step: &StepRecord) -> RunProgressStep {
    RunProgressStep {
        index: step.index,
        role: step.role.clone(),
        kind: step.kind.to_string(),
        label: step.label.clone(),
        status: step.status.to_string(),
    }
}

fn focus_step_index(state: &RunState) -> Option<usize> {
    state
        .steps
        .iter()
        .find(|step| matches!(step.status, StepStatus::Failed))
        .or_else(|| {
            state
                .steps
                .iter()
                .find(|step| matches!(step.status, StepStatus::Running))
        })
        .or_else(|| {
            state
                .steps
                .iter()
                .rev()
                .find(|step| matches!(step.status, StepStatus::Completed))
        })
        .or_else(|| state.steps.last())
        .map(|step| step.index)
}

fn short_run_id(run_id: &str) -> String {
    let limit = 12;
    if run_id.chars().count() <= limit {
        return run_id.to_owned();
    }
    run_id.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::OsString,
        fs,
        sync::{Mutex, MutexGuard},
        time::{SystemTime, UNIX_EPOCH},
    };

    use camino::Utf8PathBuf;
    use chrono::Utc;

    use crate::state::{RunStatus, StepKind};

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn assembles_run_progress_line_from_running_step() {
        let state = test_state(vec![
            test_step(1, StepStatus::Completed),
            test_step(2, StepStatus::Running),
            test_step(3, StepStatus::Pending),
        ]);

        let progress = from_state(&state).unwrap().unwrap();

        assert_eq!(
            progress_line(&progress),
            "run: 20260706T000 running | step 2/3 | reviewer agent codex running"
        );
    }

    #[test]
    fn collect_returns_none_when_no_run_exists() {
        let root = temp_dir("no-run");
        let _env = ScopedEnv::new(&root.join("niles-home"), &root.join("home"));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        let progress = collect(&workspace).unwrap();

        assert_eq!(progress, None);
        fs::remove_dir_all(root).unwrap();
    }

    fn test_state(steps: Vec<StepRecord>) -> RunState {
        let now = "2026-07-06T00:00:00Z".parse().unwrap();
        RunState {
            id: "20260706T000000000000000Z".to_owned(),
            goal: "test goal".to_owned(),
            workspace: None,
            config_root: None,
            task_file: None,
            created_at: now,
            updated_at: now,
            status: RunStatus::Running,
            steps,
        }
    }

    fn test_step(index: usize, status: StepStatus) -> StepRecord {
        StepRecord {
            index,
            role: Some("reviewer".to_owned()),
            kind: StepKind::Agent,
            label: "codex".to_owned(),
            agent_family: Some("codex".to_owned()),
            model: None,
            effort: None,
            usage_attribution: None,
            usage: None,
            status,
            started_at: Some(Utc::now()),
            finished_at: None,
            exit_code: None,
            stdout: None,
            stderr: None,
            diff: None,
            context: None,
            window: None,
        }
    }

    struct ScopedEnv {
        _lock: MutexGuard<'static, ()>,
        previous_home: Option<OsString>,
        previous_niles_home: Option<OsString>,
    }

    impl ScopedEnv {
        fn new(niles_home: &Utf8Path, home: &Utf8Path) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let previous_home = env::var_os("HOME");
            let previous_niles_home = env::var_os("NILES_HOME");
            set_env("NILES_HOME", niles_home.as_str());
            set_env("HOME", home.as_str());
            Self {
                _lock: lock,
                previous_home,
                previous_niles_home,
            }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            restore_env("HOME", &self.previous_home);
            restore_env("NILES_HOME", &self.previous_niles_home);
        }
    }

    fn set_env(key: &str, value: &str) {
        unsafe {
            env::set_var(key, value);
        }
    }

    fn restore_env(key: &str, value: &Option<OsString>) {
        unsafe {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
    }

    fn temp_dir(label: &str) -> Utf8PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
            "niles-dashboard-progress-{label}-{}-{nanos}",
            std::process::id()
        )))
        .unwrap();
        fs::create_dir_all(&path).unwrap();
        path
    }
}

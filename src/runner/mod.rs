mod exec;
mod lifecycle;
mod report;
mod usage_report;

pub(crate) use exec::{exec_step, step, step_close};
pub(crate) use lifecycle::{resume, run, step_add};
pub(crate) use report::{diff, log, show, status, watch};

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

use crate::{config::spec::TaskSpec, store::resolve_run_dir};

pub(in crate::runner) const IMPLICIT_TASK_WORKSPACE: &str = ".";

pub(in crate::runner) fn spec_workspace(spec: &TaskSpec) -> &Utf8Path {
    match spec.workspace.as_deref() {
        Some(workspace) => workspace,
        None => Utf8Path::new(IMPLICIT_TASK_WORKSPACE),
    }
}

pub struct RunSelector(String);

impl RunSelector {
    pub fn new(run: String) -> Self {
        Self(run)
    }

    pub(super) fn resolve(&self) -> Result<Utf8PathBuf> {
        resolve_run_dir(&self.0)
    }
}

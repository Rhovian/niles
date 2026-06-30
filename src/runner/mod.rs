mod exec;
mod lifecycle;
mod report;

pub(crate) use exec::{exec_step, step, step_close};
pub(crate) use lifecycle::{ask, resume, run, run_manifest, step_add};
pub(crate) use report::{diff, log, show, status, watch};

use anyhow::Result;
use camino::Utf8PathBuf;

use crate::store::resolve_run_dir;

pub struct RunSelector(String);

impl RunSelector {
    pub fn new(run: String) -> Self {
        Self(run)
    }

    pub(super) fn resolve(&self) -> Result<Utf8PathBuf> {
        resolve_run_dir(&self.0)
    }
}

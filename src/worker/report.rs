use std::{fs, io::ErrorKind};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use super::{
    archive::final_pane_path,
    meta::report_path,
    resolve::{latest_archive, resolve_live_worker_if_exists},
};

pub fn report(id: String) -> Result<()> {
    if let Some(worker_dir) = resolve_live_worker_if_exists(&id)? {
        return print_report(&id, &worker_dir, false);
    }

    let Some(archive) = latest_archive(&id)? else {
        bail!("no report found for worker '{id}': no live worker or archive found");
    };
    print_report(&id, &archive, true)
}

fn print_report(id: &str, dir: &Utf8Path, archived: bool) -> Result<()> {
    let path = report_path(dir);
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let final_pane = final_pane_path(dir);
            if final_pane.exists() {
                bail!(
                    "no report found for worker '{id}' at {path}; final pane snapshot is available at {final_pane}"
                );
            }
            bail!(
                "no report found for worker '{id}' at {path}; workers should write substantial deliverables to report.md"
            );
        }
        Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
    };
    if archived {
        eprintln!("serving archived report from {path} (archive: {dir})");
    }
    print!("{body}");
    Ok(())
}

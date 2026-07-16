use anyhow::Result;
use camino::Utf8Path;

use crate::util::read_dir_utf8_paths;

pub(super) fn startup_context(workspace: &Utf8Path) -> Result<String> {
    worker_context(workspace)
}

fn worker_context(workspace: &Utf8Path) -> Result<String> {
    let worker_dir = workspace.join(".niles").join("worker");
    let mut ids = read_dir_utf8_paths(&worker_dir)?
        .into_iter()
        .filter(|path| path.extension() == Some("json"))
        .filter_map(|path| {
            path.file_stem()
                .map(|stem| stem.to_owned())
                .filter(|stem| !stem.is_empty())
        })
        .collect::<Vec<_>>();
    ids.sort();
    if ids.is_empty() {
        Ok("worker: none".to_owned())
    } else {
        Ok(format!("worker: {}", ids.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::temp_test_path;
    use super::*;

    use std::fs;

    #[test]
    fn startup_context_reports_workers_only() {
        let root = temp_test_path("startup-context-workers");
        let worker_dir = root.join(".niles/worker");
        fs::create_dir_all(&worker_dir).unwrap();
        fs::write(worker_dir.join("beta.json"), "{}").unwrap();
        fs::write(worker_dir.join("alpha.json"), "{}").unwrap();

        let context = startup_context(&root).unwrap();

        assert_eq!(context, "worker: alpha, beta");
        fs::remove_dir_all(root).unwrap();
    }
}

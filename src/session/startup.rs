use anyhow::Result;
use camino::Utf8Path;

use crate::{
    schema::{self, ArtifactKind},
    store,
    util::read_dir_utf8_paths,
};

pub(super) fn startup_context(workspace: &Utf8Path) -> Result<String> {
    let lines = [latest_run_context(workspace)?, worker_context(workspace)?];
    Ok(lines.join("\n"))
}

fn latest_run_context(workspace: &Utf8Path) -> Result<String> {
    let Some(run_dir) = store::resolve_latest_run_dir_from(workspace)? else {
        return Ok("latest_run: none".to_owned());
    };

    let state_path = run_dir.join("state.json");
    let state = schema::read_json::<crate::state::RunState>(&state_path, ArtifactKind::RunState)?;
    Ok(format!(
        "latest_run: id={} status={} goal={:?}",
        state.id, state.status, state.goal
    ))
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

    use crate::schema;
    use chrono::Utc;

    #[test]
    fn latest_run_context_reads_typed_state_fields() {
        let root = temp_test_path("latest-run-context-valid");
        let run_dir = root.join(".niles/runs/run-1");
        fs::create_dir_all(&run_dir).unwrap();
        let now = Utc::now();
        let state = crate::state::RunState {
            id: "run-1".to_owned(),
            goal: "Ship typed context".to_owned(),
            workspace: Some(root.clone()),
            config_root: None,
            task_file: None,
            created_at: now,
            updated_at: now,
            status: crate::state::RunStatus::Running,
            steps: Vec::new(),
        };
        schema::write_json(&run_dir.join("state.json"), &state).unwrap();

        let context = latest_run_context(&root).unwrap();

        assert_eq!(
            context,
            r#"latest_run: id=run-1 status=running goal="Ship typed context""#
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn latest_run_context_errors_when_state_missing_required_field() {
        let root = temp_test_path("latest-run-context-invalid");
        let run_dir = root.join(".niles/runs/run-1");
        fs::create_dir_all(&run_dir).unwrap();
        let now = Utc::now().to_rfc3339();
        let state = serde_json::json!({
            "id": "run-1",
            "created_at": now,
            "updated_at": now,
            "status": "running",
            "steps": []
        });
        schema::write_json(&run_dir.join("state.json"), &state).unwrap();

        let err = latest_run_context(&root).unwrap_err();
        let message = format!("{err:#}");

        assert!(message.contains("missing field `goal`"));
        assert!(!message.contains("unknown"));

        fs::remove_dir_all(root).unwrap();
    }
}

use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::fs;

pub(crate) fn write(path: &Utf8Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

pub(crate) fn pi_link(dir: &Utf8Path) -> Value {
    write(&dir.join("session.jsonl"), crate::telemetry::fixtures::PI);
    json!({ "family": "pi", "session_dir": dir })
}

pub(crate) fn archive(
    workspace: &Utf8Path,
    id: &str,
    created: &str,
    closed: &str,
    link: Option<Value>,
) -> Utf8PathBuf {
    let niles = crate::store::paths::project_dir(workspace).unwrap();
    let closed: DateTime<Utc> = closed.parse().unwrap();
    let dir = niles.join(format!(
        "worker/archive/{id}-{}",
        crate::util::timestamp_id(&closed)
    ));
    let meta = json!({
        "id": id, "skill": "worker", "agent": "pi:tencent/hy3:high",
        "created_at": created, "project": workspace,
        "window": format!("demo:{id}"), "launch": niles.join(format!("worker/{id}/launch.sh")),
        "brief": niles.join(format!("worker/{id}/brief.md")), "session_link": link,
    });
    write(&dir.join("meta.json"), &meta.to_string());
    dir
}

pub(crate) fn lead(workspace: &Utf8Path, started: &str, link: Value) -> Utf8PathBuf {
    let niles = crate::store::paths::project_dir(workspace).unwrap();
    let at: DateTime<Utc> = started.parse().unwrap();
    let id = crate::util::timestamp_id(&at);
    let dir = niles.join("sessions").join(&id);
    let meta = json!({
        "id": id, "entry_skill": "coordinator", "agent": "pi:tencent/hy3:high", "created_at": started,
        "workspace": workspace, "brief": dir.join("lead.md"), "session_link": link,
    });
    write(&dir.join("session.json"), &meta.to_string());
    dir
}

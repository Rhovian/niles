#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::*;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

struct Worker {
    env: TestEnv,
    dir: PathBuf,
}
impl Worker {
    fn new(rounds: Option<u8>, skills: &[&str]) -> Self {
        let env = TestEnv::new("niles-stage");
        let dir = write_worker(&env.root, "staged", "@1", None, b"");
        let mut meta = read_json(&dir.join("meta.json"));
        meta["on_stage"] = serde_json::json!(skills);
        meta["gates"] = rounds
            .map(|rounds| serde_json::json!({"rounds": rounds}))
            .into();
        fs::write(dir.join("meta.json"), meta.to_string()).unwrap();
        Self { env, dir }
    }
    fn command(&self, kind: &str) -> Command {
        let mut command = self
            .env
            .niles(&self.env.root, &["agent", "status", kind, "ready"]);
        command
            .env("NILES_AGENT_DIR", &self.dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
    fn succeeds(&self, kind: &str) -> String {
        let output = self.command(kind).output().unwrap();
        assert_command_success(kind, &output);
        stdout_of(&output)
    }
    fn fails(&self, kind: &str, cause: &str) {
        assert_failure_contains(kind, &self.command(kind).output().unwrap(), cause);
    }
    fn report(&self) {
        fs::write(self.dir.join("report.md"), "deliverable").unwrap();
    }
}
fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn skill(env: &TestEnv, name: &str, bytes: usize) {
    let dir = env.root.join(".niles/skills").join(name);
    fs::create_dir_all(&dir).unwrap();
    let body = format!(
        "---\nname: {name}\ndescription: Use when done\n---\n{}",
        "x".repeat(bytes)
    );
    fs::write(dir.join("SKILL.md"), body).unwrap();
}

#[test]
fn configured_rounds_require_stage_and_reset_after_done() {
    for limit in [1, 2] {
        let worker = Worker::new(Some(limit), &["reviewer"]);
        worker.fails("stage", "missing report");
        assert!(!worker.dir.join("gates.json").exists());
        worker.report();
        worker.fails("done", "stage first");
        for _ in 0..limit {
            let text = worker.succeeds("stage");
            assert!(text.contains("on_stage_skill: reviewer (built-in)"));
            assert!(!text.contains("---"));
            assert!(text.contains("report `done`"));
        }
        worker.fails("stage", "round limit");
        worker.succeeds("done");
        worker.fails("done", "stage first");
        worker.succeeds("stage");
        let log = fs::read_to_string(worker.dir.join("status.log")).unwrap();
        assert!(log.contains("stage: ready; gates: none; report:"));
        assert!(!log.contains("on_stage_skill:"));
    }
}

#[test]
fn unconfigured_done_creates_no_lock_and_stage_is_rejected() {
    let worker = Worker::new(None, &[]);
    worker.report();
    worker.fails("stage", "no stage configuration");
    worker.succeeds("done");
    assert!(!worker.dir.join("stage.lock").exists());
}

#[test]
fn postwork_only_defaults_to_one_round_and_other_statuses_are_ungated() {
    let worker = Worker::new(None, &["reviewer"]);
    for kind in ["blocked", "failed", "needs-decision", "working"] {
        worker.succeeds(kind);
    }
    worker.report();
    worker.succeeds("stage");
    worker.fails("stage", "round limit 1");
}

#[test]
fn racing_stages_complete_exactly_one_round() {
    let names = vec!["postwork"; 256];
    let worker = Worker::new(Some(1), &names);
    skill(&worker.env, "postwork", 1);
    worker.report();
    let children: Vec<_> = (0..8)
        .map(|_| worker.command("stage").spawn().unwrap())
        .collect();
    let outputs: Vec<_> = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .collect();
    assert_eq!(outputs.iter().filter(|out| out.status.success()).count(), 1);
    for output in outputs.iter().filter(|out| !out.status.success()) {
        assert!(stderr_of(output).contains("round limit 1"));
    }
    let rounds = read_json(&worker.dir.join("gates.json"));
    assert_eq!(rounds.as_array().unwrap().len(), 1);
}

#[test]
fn persisted_round_without_status_is_counted_and_reset_by_done() {
    let worker = Worker::new(Some(1), &["reviewer"]);
    worker.report();
    worker.succeeds("stage");
    fs::write(worker.dir.join("status.log"), "").unwrap();
    worker.fails("stage", "round limit");
    worker.succeeds("done");
    worker.succeeds("stage");
}

#[test]
fn metadata_rounds_are_parsed_and_metadata_errors_do_not_disable_staging() {
    let worker = Worker::new(Some(1), &["reviewer"]);
    worker.report();
    let path = worker.dir.join("meta.json");
    let mut meta = read_json(&path);
    for value in [0, 11] {
        meta["gates"]["rounds"] = value.into();
        fs::write(&path, meta.to_string()).unwrap();
        worker.fails("stage", "gates.rounds must be 1..=10");
        worker.fails("done", "gates.rounds must be 1..=10");
    }
    fs::write(&path, "invalid json").unwrap();
    worker.fails("done", "meta.json");
    fs::remove_file(path).unwrap();
    worker.fails("done", "worker metadata missing");
    assert!(!worker.dir.join("stage.lock").exists());
    assert_eq!(fs::read(worker.dir.join("status.log")).unwrap(), b"");
}

#[test]
fn resolution_and_total_body_cap_leave_no_round() {
    let worker = Worker::new(Some(2), &["postwork", "otherpost"]);
    worker.report();
    assert!(!worker.command("stage").output().unwrap().status.success());
    for name in ["postwork", "otherpost"] {
        skill(&worker.env, name, 16_385);
    }
    worker.fails("stage", "32768");
    assert!(!worker.dir.join("gates.json").exists());
    assert_eq!(fs::read(worker.dir.join("status.log")).unwrap(), b"");
}

#[test]
fn spawn_snapshot_provenance_and_cap() {
    let env = TestEnv::with_manifest("niles-stage-spawn", "claude");
    let manifest = state_dir(&env.root).join("manifest.yaml");
    let text = fs::read_to_string(&manifest).unwrap();
    let configured = text.replace(
        "worker:",
        "worker:\n  on_stage: [reviewer]\n  gates: {rounds: 2}",
    );
    fs::write(&manifest, &configured).unwrap();
    let output = env.spawn("worker", &["job", "task"]);
    assert_command_success("spawn", &output);
    let provenance: Vec<_> = stdout_of(&output)
        .lines()
        .filter(|line| line.starts_with("on_stage_skill:"))
        .map(str::to_owned)
        .collect();
    assert_eq!(provenance, ["on_stage_skill: reviewer (built-in)"]);
    let workers = state_dir(&env.root).join("worker");
    let dir = workers.join("job");
    let meta = read_json(&dir.join("meta.json"));
    assert_eq!(meta["on_stage"], serde_json::json!(["reviewer"]));
    assert_eq!(meta["gates"]["rounds"], 2);
    assert!(
        fs::read_to_string(dir.join("launch.sh"))
            .unwrap()
            .contains("Your setup has a staging step")
    );
    fs::write(&manifest, text).unwrap();
    let worker = Worker { env, dir };
    worker.report();
    worker.succeeds("stage");
    skill(&worker.env, "reviewer", 32_769);
    fs::write(&manifest, configured).unwrap();
    let output = worker.env.spawn("worker", &["oversized", "task"]);
    assert_failure_contains("spawn body cap", &output, "32768");
    assert!(!workers.join("oversized").exists());
}

use super::*;
use serde_json::{Value, json};

fn gated(rounds: u8, post: Value) -> Worker {
    let worker = Worker::new(Some(rounds), &[]);
    let path = worker.dir.join("meta.json");
    let mut meta = read_json(&path);
    meta["gates"]["post"] = post;
    fs::write(path, meta.to_string()).unwrap();
    worker.report();
    worker
}

#[test]
fn passing_retry_preserves_order_literal_argv_and_worker_cwd() {
    let worker = gated(
        2,
        json!([
            {"name":"first", "run":["sh", "-c", "printf '%s' \"$1\" > literal; test -f fixed", "sh", "; $(touch injected)"]},
            {"run":["sh", "-c", "test -f literal && pwd"]}
        ]),
    );
    let tree = worker.env.root.join("tree");
    fs::create_dir(&tree).unwrap();
    let path = worker.dir.join("meta.json");
    let mut meta = read_json(&path);
    meta["tree"] = json!(tree);
    meta["on_stage"] = json!(["reviewer"]);
    fs::write(path, meta.to_string()).unwrap();
    worker.succeeds("stage");
    worker.fails("done", "gates failing");
    fs::write(tree.join("fixed"), "").unwrap();
    let text = worker.succeeds("stage");
    assert!(text.contains("first pass\nlog: "));
    assert!(text.contains("run-2 pass\nlog: "));
    assert!(text.find("first pass").unwrap() < text.find("run-2 pass").unwrap());
    assert!(text.find("run-2 pass").unwrap() < text.find("on_stage_skill:").unwrap());
    assert_eq!(
        fs::read_to_string(tree.join("literal")).unwrap(),
        "; $(touch injected)"
    );
    assert!(!tree.join("injected").exists());
    let output = fs::read_to_string(worker.dir.join("gate-2-run-2.log")).unwrap();
    assert_eq!(output.trim(), tree.to_str().unwrap());
    worker.succeeds("done");
    let log = fs::read_to_string(worker.dir.join("status.log")).unwrap();
    assert!(log.contains("gates: pass; report:"));
    assert!(log.contains("done: ready; gates: passed (2); report:"));
}

#[test]
fn failed_rounds_gate_done_and_only_names_reach_status() {
    let worker = gated(
        2,
        json!([
            {"name":"bad", "run":["sh", "-c", "echo private-output >&2; exit 7"]},
            {"name":"missing", "run":["/nonexistent-niles-gate"]}
        ]),
    );
    let text = worker.succeeds("stage");
    assert!(text.contains("bad fail (exit 7)\nlog: "));
    assert!(text.contains("private-output"));
    assert!(text.contains("missing fail (could not start: "));
    assert!(text.contains("round 1 of 2"));
    worker.fails("done", "gates failing; fix and stage again (round 1 of 2)");
    worker.succeeds("blocked");
    let text = worker.succeeds("stage");
    assert!(text.contains("report `done`: the failed gates will be shown to the lead"));
    worker.succeeds("done");
    let log = fs::read_to_string(worker.dir.join("status.log")).unwrap();
    assert!(log.contains("gates: failed bad, missing (round limit reached)"));
    assert!(!log.contains("private-output") && !log.contains("could not start"));
    assert!(worker.dir.join("gate-2-bad.log").exists());
}

#[test]
fn timeout_kills_the_process_group_and_continues_to_the_next_gate() {
    let worker = gated(
        1,
        json!([
            {"name":"slow", "timeout":{"secs":0,"nanos":100000000}, "run":["sh", "-c", "(sleep 0.3; touch survived) & echo started; wait"]},
            {"name":"after", "run":["sh", "-c", "sleep 0.4; test ! -e survived"]}
        ]),
    );
    let text = worker.succeeds("stage");
    assert!(text.contains("slow fail (timeout after 100ms)\nlog: "));
    assert!(text.contains("started"));
    assert!(text.contains("after pass\nlog: "));
    assert!(!worker.env.root.join("survived").exists());
}

#[test]
fn capped_log_is_drained_and_failure_tail_obeys_both_limits() {
    let worker = gated(
        1,
        json!([
            {"name":"lines", "run":["sh", "-c", "head -c 1100000 /dev/zero; i=0; while [ $i -lt 60 ]; do echo line-$i; i=$((i+1)); done; exit 9"]},
            {"name":"bytes", "run":["sh", "-c", "head -c 6000 /dev/zero | LC_ALL=C tr '\\000' '\\377'; exit 1"]}
        ]),
    );
    let text = worker.succeeds("stage");
    assert!(text.contains("lines fail (exit 9)\nlog: "));
    assert_eq!(
        fs::metadata(worker.dir.join("gate-1-lines.log"))
            .unwrap()
            .len(),
        1_048_576
    );
    let (_, output) = text.split_once(".log\n").unwrap();
    let (tail, _) = output.split_once("\nbytes fail").unwrap();
    assert_eq!(tail.lines().count(), 40);
    assert!(tail.starts_with("line-20\n") && tail.ends_with("line-59"));
    let (_, output) = text.rsplit_once(".log\n").unwrap();
    let (tail, _) = output.split_once("report `done`").unwrap();
    assert!(tail.contains('�') && tail.len() <= 4096);
    let log = fs::read_to_string(worker.dir.join("status.log")).unwrap();
    assert!(!log.contains("line-") && !log.contains('�'));
}

#[test]
fn killed_stage_records_no_round() {
    let worker = gated(
        1,
        json!([{"run":["sh", "-c", "echo started > started; sleep 0.2"]}]),
    );
    let mut child = worker.command("stage").spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !worker.env.root.join("started").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!worker.dir.join("gates.json").exists());
    worker.fails("done", "stage first");
    worker.succeeds("stage");
}

#[test]
fn signal_death_prints_the_native_exit_status() {
    use std::os::unix::process::ExitStatusExt;
    let worker = gated(1, json!([{"run":["sh", "-c", "kill -TERM $$"]}]));
    let status = std::process::ExitStatus::from_raw(libc::SIGTERM);
    assert!(
        worker
            .succeeds("stage")
            .contains(&format!("run-1 fail ({status})\nlog: "))
    );
}

use super::support::*;

#[test]
fn corrupt_cursor_fails_loudly_and_names_the_file() {
    let workspace = temp_workspace("niles-wait-corrupt");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: ready\n");
    atomic_write(worker_dir.join("status.cursor"), "not-a-number\n").unwrap();

    let output = run_wait(&workspace, &["--interval", "50ms", "--timeout", "0"]);

    assert_failure_contains("wait with corrupt cursor", &output, "invalid wake cursor");
    let stderr = stderr_of(&output);
    // The operator has to be told which file to delete, or the worker is wedged.
    assert!(stderr.contains("status.cursor"), "stderr: {stderr}");
    assert!(stderr.contains("remove it to resume"), "stderr: {stderr}");
}

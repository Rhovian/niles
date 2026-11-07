use super::support::*;

#[test]
fn corrupt_cursor_fails_loudly_and_names_the_file() {
    let workspace = temp_workspace("niles-wait-corrupt");
    let worker_dir = worker_with_status(&workspace, "auth-fix", b"done: ready\n");
    fs::write(worker_dir.join("status.cursor"), "not-a-number\n").unwrap();

    let output = run_wait(&workspace, &["--interval", "50ms", "--timeout", "0"]);

    assert_failure_contains("wait with corrupt cursor", &output, "invalid wake cursor");
    let stderr = stderr_of(&output);
    // The operator has to be told which file to delete, or the worker is wedged.
    assert!(stderr.contains("status.cursor"), "stderr: {stderr}");
    assert!(stderr.contains("remove it to resume"), "stderr: {stderr}");
}

#[test]
fn caps_an_enormous_status_line_instead_of_flooding_the_manager() {
    let workspace = temp_workspace("niles-wait-huge");
    let mut status = b"done: ".to_vec();
    status.extend(std::iter::repeat_n(b'x', 64 * 1024));
    status.push(b'\n');
    worker_with_status(&workspace, "auth-fix", &status);

    let output = run_wait(&workspace, &["--interval", "50ms", "--timeout", "0"]);

    assert_command_success("wait with an enormous line", &output);
    let stdout = stdout_of(&output);
    assert!(stdout.starts_with("auth-fix done: xxx"));
    assert!(stdout.contains("(truncated)"), "stdout was not capped");
    assert!(stdout.len() <= 4096, "stdout was {} bytes", stdout.len());
}

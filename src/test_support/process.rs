use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

pub fn output_with_stdin(mut command: Command, input: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

pub fn write_executable(path: impl AsRef<Path>, body: &str) {
    // Keep the write fd out of the test process: concurrent forks can inherit it
    // and cause ETXTBSY when another test execs the script, even after we close it.
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "/bin/cat > \"$1\" && /bin/chmod 755 \"$1\"", "sh"])
        .arg(path.as_ref());
    let output = output_with_stdin(command, body.as_bytes());
    assert!(output.status.success(), "{output:?}");
}

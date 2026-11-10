#![cfg(not(feature = "remote"))]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]

mod common;
use common::{TestEnv, stderr_of};

#[test]
fn disabled_remote_is_a_recognized_command_with_install_instructions() {
    let env = TestEnv::new("remote-disabled");
    let output = env.run(&["remote"]);
    assert!(!output.status.success());
    assert!(stderr_of(&output).contains("built without remote; reinstall with --features remote"));
    assert!(output.stdout.is_empty());
}

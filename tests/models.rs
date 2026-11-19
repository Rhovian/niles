#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]

mod common;

use common::*;
use std::fs;
use std::os::unix::fs::symlink;

#[test]
fn models_prints_cli_catalogues() {
    let env = TestEnv::new("niles-models-test");
    atomic_write(
        env.root.join(".catalogue-repo-marker"),
        "must not load repository configuration",
    )
    .unwrap();
    fs::create_dir_all(state_dir(&env.root)).unwrap();
    atomic_write(state_dir(&env.root).join("manifest.yaml"), "invalid: [").unwrap();
    fs::rename(env.bin.join("codex"), env.bin.join("catalogue-dispatch")).unwrap();
    symlink("catalogue-dispatch", env.bin.join("codex")).unwrap();
    let output = env
        .niles(&env.root, &["models", "--setup", "invalid:selection"])
        .env("PATH", "bin:/usr/bin:/bin")
        .output()
        .unwrap();
    assert_command_success("models", &output);
    let stdout = stdout_of(&output);
    assert!(
        stdout.starts_with("models[11]{family,model,effort,context_window}:\n"),
        "{stdout}"
    );
    assert_eq!(
        stdout.lines().skip(1).map(str::trim).collect::<Vec<_>>(),
        [
            "codex,gpt-6.2-sol,medium,500000",
            "codex,gpt-6.2-sol,high,500000",
            "claude,haiku,high,unknown",
            "pi,provider/fallback,off,789",
            "pi,provider/fallback,minimal,789",
            "pi,provider/fallback,low,789",
            "pi,provider/fallback,medium,789",
            "pi,provider/fallback,high,789",
            "pi,provider/future,low,456",
            "pi,provider/future,high,456",
            "pi,provider/plain,off,100",
        ]
    );
    fs::remove_file(env.bin.join("codex")).unwrap();
    for (script, cause) in [
        (
            "#!/bin/sh\necho catalogue-denied >&2\nexit 9\n",
            "catalogue-denied",
        ),
        (
            "#!/bin/sh\nprintf '%s' '{\"private_payload\":\"do-not-dump-response\"}'\n",
            "missing field `models`",
        ),
    ] {
        write_executable(&env.bin.join("codex"), script);
        let output = env.run(&["models"]);
        assert_failure_contains("catalogue failure", &output, cause);
        assert!(!stderr_of(&output).contains("do-not-dump-response"));
        let available = stdout_of(&output);
        assert!(available.starts_with("models[9]"), "{available}");
        assert!(
            available.contains("claude,haiku,high") && available.contains("pi,provider/future,low")
        );
        assert!(!available.contains("codex,gpt-6.2-sol"));
    }
}

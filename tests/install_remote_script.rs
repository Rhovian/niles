#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{fs, process::Command};

use common::{TestEnv, assert_command_success, path_with_bin, stdout_of, write_executable};

#[test]
fn builds_shell_before_installing_from_any_directory() {
    let root = fs::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap();
    for fail_build in [false, true] {
        let env = TestEnv::new("install-remote");
        let log = env.root.join("install.log");
        write_executable(
            &env.bin.join("pnpm"),
            "#!/bin/sh\nprintf '%s: pnpm %s\\n' \"$PWD\" \"$*\" >> \"$INSTALL_LOG\"\n\
             if [ \"$1\" = build ] && [ \"$FAIL_BUILD\" = true ]; then exit 1; fi\n",
        );
        write_executable(
            &env.bin.join("cargo"),
            "#!/bin/sh\nprintf '%s: cargo %s\\n' \"$PWD\" \"$*\" >> \"$INSTALL_LOG\"\n",
        );
        let output = Command::new(root.join("scripts/install-remote.sh"))
            .current_dir(&env.root)
            .env("PATH", path_with_bin(&env.bin))
            .env("INSTALL_LOG", &log)
            .env("FAIL_BUILD", fail_build.to_string())
            .output()
            .unwrap();
        let mut expected = format!(
            "{0}/web: pnpm install --frozen-lockfile\n{0}/web: pnpm build\n",
            root.display()
        );
        if fail_build {
            assert_eq!(output.status.code(), Some(1));
            assert!(output.stdout.is_empty());
        } else {
            assert_command_success("install remote", &output);
            expected.push_str(&format!(
                "{}: cargo install --path {} --locked --features remote\n",
                env.root.display(),
                root.display()
            ));
            assert_eq!(
                stdout_of(&output),
                format!("Web shell: {}/web/dist\n", root.display())
            );
        }
        assert_eq!(fs::read_to_string(log).unwrap(), expected);
    }
}

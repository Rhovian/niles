use super::{CUSTOM_URL, environment, manifest, support, terminal};
use crate::common::{atomic_write, write_executable};
use std::fs;

#[test]
fn failed_custom_setup_can_retry_the_same_url() {
    for failure in ["empty body", "starter fetch", "custom fetch"] {
        let env = environment();
        fs::remove_dir_all(env.home.join(".niles/skills/design")).unwrap();
        let body = if failure == "empty body" {
            ""
        } else {
            "Coordinate work."
        };
        if failure != "custom fetch" {
            support::import_fixture(
                &env,
                "custom",
                &format!("---\nname: custom\ndescription: Custom\n---\n{body}\n"),
            );
        }
        let optional = if failure == "starter fetch" {
            b"\x1b[B \r".as_slice()
        } else {
            b"\r"
        };
        let steps = |optional| {
            let mut steps = CUSTOM_URL.to_vec();
            steps
                .iter_mut()
                .find(|(prompt, _)| *prompt == "starter")
                .unwrap()
                .1 = optional;
            steps
        };
        let (output, text) = terminal(&env, &["start"], &steps(optional), |_| {});
        assert!(!output.status.success(), "{failure}: {output:?} {text}");
        assert!(!manifest(&env).exists());
        assert!(
            !env.home.join(".niles/skills/custom/SKILL.ref").exists(),
            "{failure} left a custom installation"
        );
        // The repository fixes its empty body at a new commit; the operator enters the same URL.
        if failure == "empty body" {
            atomic_write(
                env.home.join("skill.md"),
                "---\nname: custom\ndescription: Custom\n---\nCoordinate work.\n",
            )
            .unwrap();
            let git = env.bin.join("git");
            write_executable(
                &git,
                &fs::read_to_string(&git)
                    .unwrap()
                    .replace(&"a".repeat(40), &"b".repeat(40)),
            );
        }
        if failure == "custom fetch" {
            support::import_fixture(
                &env,
                "custom",
                "---\nname: custom\ndescription: Custom\n---\nCoordinate work.\n",
            );
        }
        let (output, text) = terminal(&env, &["start"], &steps(b"\r"), |_| {});
        assert!(
            output.status.success(),
            "retry {failure}: {output:?} {text}"
        );
        assert!(manifest(&env).exists());
        assert!(env.home.join(".niles/skills/custom/SKILL.ref").exists());
    }
}

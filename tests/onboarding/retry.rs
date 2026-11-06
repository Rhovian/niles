use super::{environment, manifest, support, terminal};
use std::fs;

#[test]
fn failed_custom_setup_can_retry_the_same_url() {
    for failure in ["empty body", "starter fetch"] {
        let env = environment();
        fs::remove_dir_all(env.home.join(".niles/skills/design")).unwrap();
        let body = if failure == "empty body" {
            ""
        } else {
            "Coordinate work."
        };
        support::import_fixture(
            &env,
            "custom",
            &format!("---\nname: custom\ndescription: Custom\n---\n{body}\n"),
        );
        let optional = if failure == "starter fetch" {
            b"\x1b[B \r".as_slice()
        } else {
            b"\r"
        };
        let steps = |optional| {
            [
                ("work?", b"\r".as_slice()),
                ("lead's", b"\x1b[B\r"),
                ("URL", b"github.com/other/repository/skills/custom\r"),
                ("TIERS", b"\r"),
                ("starter", optional),
                ("Warning:", b"\r"),
            ]
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
            fs::write(
                env.home.join("skill.md"),
                "---\nname: custom\ndescription: Custom\n---\nCoordinate work.\n",
            )
            .unwrap();
            let git = env.bin.join("git");
            fs::write(
                &git,
                fs::read_to_string(&git)
                    .unwrap()
                    .replace(&"a".repeat(40), &"b".repeat(40)),
            )
            .unwrap();
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

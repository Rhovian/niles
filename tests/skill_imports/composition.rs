use super::*;

const SOURCES: [(&str, &str); 3] = [
    ("lead", "lead"),
    ("hub-spoke", "skills/hub-spoke"),
    ("guide", "skills/guide"),
];

#[test]
fn composed_imports_pin_recursive_dependencies_and_update_them_in_the_same_origin() {
    let pack = Pack::new("imports-composition");
    for name in ["lead", "hub-spoke"] {
        fs::remove_dir_all(pack.env.home.join(".niles/skills").join(name)).unwrap();
    }
    document(
        &pack.repo,
        "lead",
        "lead",
        &[
            "Load `niles agent skill hub-spoke` twice: `niles agent skill hub-spoke`.",
            "Run `niles agent skill --help` for usage.",
            "use niles agent skill to load a role",
            "Templates: `niles agent skill <name>`, `niles agent skill {name}`,",
            "`niles agent skill NAME` and `niles agent skill $name`.",
            "Whitespace: `niles agent skill   `.",
        ]
        .join("\n\n"),
    );
    document(
        &pack.repo,
        "skills/hub-spoke",
        "hub-spoke",
        "niles agent skill guide",
    );
    document(
        &pack.repo,
        "skills/guide",
        "guide",
        "niles agent skill lead",
    );
    let first = pack.commit("composition");
    document(
        &pack.repo,
        "skills/hub-spoke",
        "hub-spoke",
        "Updated.\n\n```sh\nniles agent skill guide\n```",
    );
    let second = pack.commit("update required skill");
    for (global, revision, sha) in [
        (false, first.as_str(), first.as_str()),
        (false, "HEAD", second.as_str()),
        (true, first.as_str(), first.as_str()),
    ] {
        let reference = format!("acme/pack/lead@{revision}");
        let mut args = vec!["skills", "import", &reference];
        if global {
            args.push("--global");
        }
        let output = pack.run(&args);
        assert_command_success("composed import", &output);
        assert_eq!(stdout_of(&output).matches("skill: ").count(), 3);
        let root = if global {
            &pack.env.home
        } else {
            &pack.env.root
        };
        for (name, path) in SOURCES {
            let pointer = root.join(format!(".niles/skills/{name}/SKILL.ref"));
            assert_eq!(
                fs::read_to_string(pointer).unwrap(),
                format!("github.com/acme/pack/{path}@{sha}\n"),
            );
        }
    }
    document(
        &pack.repo,
        "skills/guide",
        "guide",
        "niles agent skill absent",
    );
    pack.commit("missing transitive dependency");
    assert_failure_contains(
        "missing required skill",
        &pack.import("acme/pack/lead"),
        "must be a Git directory",
    );
    for (name, path) in SOURCES {
        assert_eq!(
            fs::read_to_string(pack.pointer(name)).unwrap(),
            format!("github.com/acme/pack/{path}@{second}\n"),
        );
    }
}

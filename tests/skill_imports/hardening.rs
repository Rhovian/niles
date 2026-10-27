use super::*;

#[test]
fn ambiguous_revisions_invalid_paths_symlinks_and_oversize_are_rejected() {
    let pack = Pack::new("imports-hardening");
    git(&pack.remote, &["branch", "v1", &pack.sha]);
    assert_failure_contains(
        "ambiguous branch/tag",
        &pack.import("acme/pack/skills/pdf@v1"),
        "ambiguous",
    );
    for path in ["absent/pdf", "not-a-dir"] {
        assert_failure_contains(
            "not directory",
            &pack.import(&format!("acme/pack/{path}")),
            "must be a Git directory",
        );
    }
    fs::remove_file(pack.repo.join("skills/pdf/SKILL.md")).unwrap();
    let secret = pack.env.root.join("secret");
    fs::write(&secret, "PRIVATE_BYTES_MUST_NOT_BE_READ").unwrap();
    std::os::unix::fs::symlink(&secret, pack.repo.join("skills/pdf/SKILL.md")).unwrap();
    git(
        &pack.repo,
        &[
            "clone",
            "-q",
            "--no-checkout",
            pack.remote.to_str().unwrap(),
            "gitlink",
        ],
    );
    pack.commit("symlink and gitlink");
    assert!(git(&pack.repo, &["ls-tree", "HEAD", "--", "gitlink"]).starts_with("160000 commit "));
    assert_failure_contains(
        "gitlink",
        &pack.import("acme/pack/gitlink"),
        "must be a Git directory",
    );
    let output = pack.import("acme/pack/skills/pdf");
    assert_failure_contains(
        "symlink becomes plain file",
        &output,
        "frontmatter must start",
    );
    assert!(!stderr_of(&output).contains("PRIVATE_BYTES"));
    fs::remove_file(pack.repo.join("skills/pdf/SKILL.md")).unwrap();
    document(&pack.repo, "skills/pdf", "pdf", "valid");
    let file = fs::File::create(pack.repo.join("skills/pdf/large")).unwrap();
    file.set_len(8 * 1024 * 1024 + 1).unwrap();
    pack.commit("large");
    assert_failure_contains(
        "size cap",
        &pack.import("acme/pack/skills/pdf"),
        "exceeds 8 MiB",
    );
    assert!(!pack.pointer("pdf").exists());
}

#[test]
fn global_and_workspace_imports_share_complete_cache_files() {
    let pack = Pack::new("imports-roots");
    let reference = format!("acme/pack/skills/pdf@{}", pack.sha);
    assert_command_success(
        "global import",
        &pack.run(&["skills", "import", &reference, "--global"]),
    );
    assert_command_success("workspace import", &pack.import(&reference));
    assert_eq!(
        fs::read_to_string(pack.env.home.join(".niles/skills/pdf/SKILL.ref")).unwrap(),
        format!("github.com/{reference}\n")
    );
    assert_eq!(
        fs::read_to_string(pack.pointer("pdf")).unwrap(),
        format!("github.com/{reference}\n")
    );
    assert_eq!(
        fs::read_to_string(pack.cache("pdf").join("scripts/check.sh")).unwrap(),
        "#!/bin/sh\nprintf support\n"
    );
    let parent = pack
        .env
        .home
        .join(".niles/cache/skills/github.com/acme/pack");
    assert!(fs::read_dir(parent).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_str()
            .unwrap()
            .starts_with(".fetch-")
    }));
}

#[test]
fn cache_reads_are_strict_and_remote_frontmatter_is_checked() {
    let pack = Pack::new("imports-cache-files");
    assert_command_success("import", &pack.import("acme/pack/skills/pdf"));
    let cached = pack.cache("pdf").join("SKILL.md");
    let outside = pack.env.root.join("outside");
    fs::rename(&cached, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &cached).unwrap();
    assert_failure_contains(
        "cache symlink",
        &pack.import(&format!("acme/pack/skills/pdf@{}", pack.sha)),
        "must be regular",
    );
    fs::remove_file(&cached).unwrap();
    fs::rename(outside, cached).unwrap();
    document(&pack.repo, "skills/pdf", "wrong", "invalid name");
    pack.commit("name mismatch");
    assert_failure_contains(
        "name mismatch",
        &pack.import("acme/pack/skills/pdf"),
        "frontmatter name must equal",
    );
    document(&pack.repo, "skills/pdf", "pdf", &"x".repeat(131073));
    pack.commit("oversized document");
    assert_failure_contains(
        "document cap",
        &pack.import("acme/pack/skills/pdf"),
        "maximum is 131072",
    );
}

#[test]
fn transport_never_prompts_or_runs_configured_checkout_hooks() {
    let pack = Pack::new("imports-transport");
    let executable = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert_command_success("resolve git from PATH", &executable);
    let real_git = stdout_of(&executable);
    write_executable(
        &pack.env.bin.join("git"),
        "#!/bin/sh\n[ \"$GIT_TERMINAL_PROMPT\" = 0 ] || exit 73\nexec \"$NILES_TEST_REAL_GIT\" \"$@\"\n",
    );
    let hooks = pack.env.root.join("hooks");
    fs::create_dir(&hooks).unwrap();
    let sentinel = pack.env.root.join("hook-ran");
    write_executable(
        &hooks.join("post-checkout"),
        &format!("#!/bin/sh\ntouch '{}'\n", sentinel.display()),
    );
    let config = pack.env.home.join(".gitconfig");
    let body = fs::read_to_string(&config).unwrap();
    fs::write(
        config,
        format!("{body}\n[core]\n hooksPath = {}\n", hooks.display()),
    )
    .unwrap();
    let output = pack
        .command(&["skills", "import", "acme/pack/skills/pdf"])
        .env("NILES_TEST_REAL_GIT", real_git.trim())
        .output()
        .unwrap();
    assert_command_success("hardened transport", &output);
    assert!(!sentinel.exists());
}

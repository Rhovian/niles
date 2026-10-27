use super::*;

#[test]
fn root_and_nested_skills_have_distinct_keys_without_git_metadata() {
    let pack = Pack::new("imports-root-cache");
    let root = pack.env.home.join(format!(
        ".niles/cache/skills/github.com/acme/pack/{}/@skill",
        pack.sha
    ));
    assert_command_success("root first", &pack.import("acme/pack@v1"));
    assert!(root.join("SKILL.md").is_file());
    assert!(root.join("skills/pdf/scripts/check.sh").is_file());
    assert!(!root.join(".git").exists());
    assert_command_success("nested after root", &pack.import("acme/pack/skills/pdf@v1"));
    assert!(pack.cache("pdf").join("SKILL.md").is_file());
    assert!(!pack.cache("pdf").join(".git").exists());
    assert!(!pack.env.home.join(".niles/cache/skills/.ready").exists());
}

#[test]
fn inherited_git_repository_paths_cannot_redirect_the_checkout() {
    let pack = Pack::new("imports-git-env");
    let head = fs::read(pack.repo.join(".git/HEAD")).unwrap();
    let index = fs::read(pack.repo.join(".git/index")).unwrap();
    let file = fs::read(pack.repo.join("SKILL.md")).unwrap();
    let output = pack
        .command(&["skills", "import", "acme/pack/skills/pdf"])
        .env("GIT_DIR", pack.repo.join(".git"))
        .env("GIT_WORK_TREE", &pack.repo)
        .env("GIT_INDEX_FILE", pack.repo.join(".git/index"))
        .env("GIT_COMMON_DIR", pack.repo.join(".git"))
        .env("GIT_OBJECT_DIRECTORY", pack.repo.join(".git/objects"))
        .env("GIT_SHALLOW_FILE", pack.repo.join(".git/shallow"))
        .output()
        .unwrap();
    assert_command_success("isolated checkout", &output);
    assert_eq!(fs::read(pack.repo.join(".git/HEAD")).unwrap(), head);
    assert_eq!(fs::read(pack.repo.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(pack.repo.join("SKILL.md")).unwrap(), file);
    assert!(!pack.repo.join(".git/shallow").exists());
}

#[test]
fn failed_revision_resolution_does_not_create_the_global_skills_root() {
    let pack = Pack::new("imports-resolution-cwd");
    fs::remove_dir_all(pack.env.home.join(".niles")).unwrap();
    assert_failure_contains(
        "missing revision",
        &pack.import("acme/pack/skills/pdf@missing"),
        "missing or ambiguous",
    );
    assert!(!pack.env.home.join(".niles").exists());
}

#[test]
fn invalid_existing_pointer_requires_explicit_removal() {
    let pack = Pack::new("imports-pointer-repair");
    let pointer = pack.pointer("pdf");
    fs::create_dir_all(pointer.parent().unwrap()).unwrap();
    fs::write(&pointer, "invalid stored pointer\n").unwrap();
    let output = pack.import("acme/pack/skills/pdf");
    assert_failure_contains("invalid previous pointer", &output, "before re-importing");
    assert!(stderr_of(&output).contains(pointer.to_str().unwrap()));
    assert_eq!(
        fs::read_to_string(&pointer).unwrap(),
        "invalid stored pointer\n"
    );
    fs::remove_file(&pointer).unwrap();
    assert_command_success("explicit recovery", &pack.import("acme/pack/skills/pdf"));
}

#[test]
fn incomplete_cache_is_not_served_and_directory_removal_allows_refetch() {
    let pack = Pack::new("imports-incomplete");
    let reference = format!("acme/pack/skills/pdf@{}", pack.sha);
    assert_command_success("initial import", &pack.import(&reference));
    fs::remove_file(pack.cache("pdf").join("SKILL.md")).unwrap();
    assert_failure_contains(
        "incomplete entry",
        &pack.import(&reference),
        "remove incomplete cache entry",
    );
    assert!(pack.cache("pdf").join("scripts/check.sh").is_file());
    fs::remove_dir_all(pack.cache("pdf")).unwrap();
    assert_command_success("refetch complete entry", &pack.import(&reference));
    assert!(pack.cache("pdf").join("SKILL.md").is_file());
}

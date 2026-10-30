#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Pack {
    env: TestEnv,
    repo: PathBuf,
    remote: PathBuf,
    sha: String,
}
impl Pack {
    fn new(label: &str) -> Self {
        let env = TestEnv::with_manifest(label, "claude");
        let repo = env.root.join("source");
        let remote = env.root.join("remotes/acme/pack.git");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(remote.parent().unwrap()).unwrap();
        git(&repo, &["init", "-q", "--initial-branch=main"]);
        document(&repo, "", "pack", "root version one");
        document(&repo, "skills/pdf", "pdf", "pdf version one");
        document(&repo, "skills/guide", "guide", "guide version one");
        document(&repo, "odd[dir]/guide", "guide", "literal pattern guide");
        fs::create_dir_all(repo.join("skills/pdf/scripts")).unwrap();
        fs::write(
            repo.join("skills/pdf/scripts/check.sh"),
            "#!/bin/sh\nprintf support\n",
        )
        .unwrap();
        fs::write(repo.join("not-a-dir"), "file").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "initial"]);
        let sha = git(&repo, &["rev-parse", "HEAD"]).trim().to_owned();
        git(&repo, &["tag", "-a", "v1", "-m", "version one"]);
        git(
            &repo,
            &["clone", "-q", "--bare", ".", remote.to_str().unwrap()],
        );
        git(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        // The executable always names github.com. Only this isolated test HOME
        // rewrites that URL base to a local bare repository; CI needs no network.
        fs::write(
            env.home.join(".gitconfig"),
            format!(
                "[url \"file://{}/\"]\n insteadOf = https://github.com/\n",
                env.root.join("remotes").display()
            ),
        )
        .unwrap();
        Self {
            env,
            repo,
            remote,
            sha,
        }
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = self.env.niles(&self.env.root, args);
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.env.home.join(".gitconfig"));
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    fn import(&self, reference: &str) -> Output {
        self.run(&["skills", "import", reference])
    }
    fn pointer(&self, name: &str) -> PathBuf {
        self.env
            .root
            .join(format!(".niles/skills/{name}/SKILL.ref"))
    }
    fn cache(&self, name: &str) -> PathBuf {
        self.env.home.join(format!(
            ".niles/cache/skills/github.com/acme/pack/{}/skills/{name}/@skill",
            self.sha
        ))
    }
    fn spawn(&self, args: &[&str]) -> Output {
        let mut command = self.command(&["spawn", "--skill", "worker"]);
        command.args(args).output().unwrap()
    }
    fn discover(&self, id: &str, args: &[&str]) -> Output {
        let mut command = self.command(args);
        command.env(
            "NILES_AGENT_DIR",
            state_dir(&self.env.root).join("worker").join(id),
        );
        command.output().unwrap()
    }
    fn update(&self) -> String {
        document(&self.repo, "skills/pdf", "pdf", "pdf version two");
        self.commit("update")
    }
    fn commit(&self, message: &str) -> String {
        git(&self.repo, &["add", "."]);
        git(&self.repo, &["commit", "-qm", message]);
        git(&self.repo, &["push", "-q", "origin", "main"]);
        git(&self.repo, &["rev-parse", "HEAD"]).trim().to_owned()
    }
}
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert_command_success("fixture git", &output);
    stdout_of(&output)
}
fn document(repo: &Path, path: &str, name: &str, body: &str) {
    let dir = repo.join(path);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Use when needed\n---\n{body}"),
    )
    .unwrap();
}

#[test]
fn import_pins_updates_and_preserves_supporting_files_and_other_skills() {
    let pack = Pack::new("imports-pin");
    let output = pack.import("acme/pack/skills/pdf@v1");
    assert_command_success("import annotated tag", &output);
    let canonical = format!("github.com/acme/pack/skills/pdf@{}\n", pack.sha);
    assert_eq!(fs::read_to_string(pack.pointer("pdf")).unwrap(), canonical);
    assert!(stdout_of(&output).contains(&format!(
        "import github.com/acme/pack/skills/pdf@{}",
        &pack.sha[..7]
    )));
    assert!(stdout_of(&output).contains(pack.cache("pdf").to_str().unwrap()));
    assert_eq!(
        fs::read_to_string(pack.cache("pdf").join("scripts/check.sh")).unwrap(),
        "#!/bin/sh\nprintf support\n"
    );
    assert_command_success("literal path", &pack.import("acme/pack/odd[dir]/guide@v1"));
    let root = pack.env.home.join(format!(
        ".niles/cache/skills/github.com/acme/pack/{}/@skill",
        pack.sha
    ));
    assert_command_success("root alongside nested skill", &pack.import("acme/pack@v1"));
    assert!(root.join("SKILL.md").is_file());
    assert!(!root.join(".git").exists());
    assert!(pack.cache("pdf").join("SKILL.md").is_file());
    let offline = pack.remote.with_extension("offline");
    fs::rename(&pack.remote, &offline).unwrap();
    assert_command_success(
        "global import reuses complete cache",
        &pack.run(&[
            "skills",
            "import",
            &format!("acme/pack/skills/pdf@{}", pack.sha),
            "--global",
        ]),
    );
    assert_eq!(
        fs::read_to_string(pack.env.home.join(".niles/skills/pdf/SKILL.ref")).unwrap(),
        canonical
    );
    fs::rename(offline, &pack.remote).unwrap();
    git(&pack.remote, &["tag", "topic/HEAD", &pack.sha]);
    let new = pack.update();
    let output = pack.import("acme/pack/skills/pdf");
    assert_command_success("reimport HEAD", &output);
    assert!(stdout_of(&output).contains(&format!("{} → {new}", pack.sha)));
    assert!(
        fs::read_to_string(pack.pointer("pdf"))
            .unwrap()
            .contains(&new)
    );
    assert!(
        fs::read_to_string(pack.cache("pdf").join("SKILL.md"))
            .unwrap()
            .contains("version one")
    );
    // An abbreviated SHA is not sent to fetch as if it were a commit.
    assert_failure_contains(
        "abbreviated SHA",
        &pack.import(&format!("acme/pack/skills/pdf@{}", &new[..7])),
        "missing or ambiguous",
    );
}

#[test]
fn resting_refs_load_offline_and_work_in_all_instruction_lists_and_entry() {
    let pack = Pack::new("imports-privilege");
    assert_command_success(
        "global import",
        &pack.run(&["skills", "import", "acme/pack/skills/pdf", "--global"]),
    );
    let pointer = pack.env.home.join(".niles/skills/pdf/SKILL.ref");
    let external = pack.env.root.join("global-pointer");
    fs::rename(&pointer, &external).unwrap();
    std::os::unix::fs::symlink(&external, &pointer).unwrap();
    let manifest = state_dir(&pack.env.root).join("manifest.yaml");
    let body = fs::read_to_string(&manifest).unwrap().replace(
        "worker: {}\n",
        "worker:\n  skills: [pdf]\n  always: [pdf]\n  on_from: [pdf]\n  on_stage: [pdf]\n",
    );
    fs::write(&manifest, body).unwrap();
    fs::remove_dir_all(pack.env.home.join(".niles/cache")).unwrap();
    let hidden = pack.remote.with_extension("offline");
    fs::rename(&pack.remote, &hidden).unwrap();
    assert_command_success("manifest load stays offline", &pack.run(&["models"]));
    fs::rename(hidden, &pack.remote).unwrap();
    let output = pack.spawn(&["job", "--with", "pdf", "task"]);
    assert_command_success("refetch on cache miss and inject", &output);
    for provenance in [
        "menu_skill",
        "always_skill",
        "on_from_skill",
        "on_stage_skill",
        "with_skill",
    ] {
        assert!(stdout_of(&output).contains(&format!("{provenance}: pdf (import ")));
    }
    let launch =
        fs::read_to_string(state_dir(&pack.env.root).join("worker/job/launch.sh")).unwrap();
    assert_eq!(launch.matches("pdf version one").count(), 1);
    fs::write(
        state_dir(&pack.env.root).join("worker/job/report.md"),
        "report",
    )
    .unwrap();
    let staged = pack.discover("job", &["agent", "status", "stage", "ready"]);
    assert_command_success("imported stage skill", &staged);
    assert!(stdout_of(&staged).contains("pdf version one"));
    let listing = pack.run(&["skills"]);
    assert_command_success("listing", &listing);
    fs::write(
        &manifest,
        "entry: pdf\ntiers: {standard: {primary: claude:sonnet:high, secondaries: []}}\npdf: {}\n",
    )
    .unwrap();
    assert!(stdout_of(&pack.run(&["skills"])).contains("skill: pdf (import github.com/"));
    let output = pack.run(&["start", "--keep-skills"]);
    assert_command_success("imported entry", &output);
    assert!(stderr_of(&output).contains("skill: pdf (import github.com/"));
}

#[test]
fn ephemeral_menu_remains_pinned_and_does_not_install_or_inject() {
    let pack = Pack::new("imports-ephemeral");
    let output = pack.spawn(&["job", "--menu", "acme/pack/skills/pdf@main", "task"]);
    assert_command_success("ephemeral", &output);
    assert!(!pack.pointer("pdf").exists());
    assert!(!pack.env.home.join(".niles/skills/pdf").exists());
    let dir = state_dir(&pack.env.root).join("worker/job");
    let meta = read_json(dir.join("meta.json"));
    assert_eq!(meta["menu"], serde_json::json!(["pdf"]));
    assert_eq!(
        meta["imports"]["pdf"],
        format!("github.com/acme/pack/skills/pdf@{}", pack.sha)
    );
    assert!(
        !fs::read_to_string(dir.join("launch.sh"))
            .unwrap()
            .contains("pdf version one")
    );
    let brief = fs::read_to_string(dir.join("brief.md")).unwrap();
    assert!(brief.contains(pack.cache("pdf").to_str().unwrap()));
    pack.update();
    // A chain shadow cannot replace the worker's snapshot pin.
    document(&pack.env.root.join(".niles"), "skills/pdf", "pdf", "shadow");
    fs::remove_dir_all(pack.cache("pdf")).unwrap();
    let output = pack.discover("job", &["agent", "skill", "pdf"]);
    assert_command_success("pinned discovery refetch", &output);
    assert!(stdout_of(&output).contains("pdf version one"));
    assert!(!stdout_of(&output).contains("version two"));
    let output = pack.discover("job", &["agent", "skills"]);
    assert_command_success("pinned menu rows", &output);
    assert!(stdout_of(&output).contains(&pack.sha[..7]));
    assert_failure_contains(
        "with is name-only",
        &pack.spawn(&["invalid", "--with", "acme/pack/skills/pdf", "task"]),
        "invalid skill name",
    );
}

#[test]
fn collisions_conflicts_and_origin_rules_fail_before_worker_creation() {
    let pack = Pack::new("imports-collisions");
    let ref_worker = "acme/pack/skills/worker";
    for flags in [
        vec!["--menu", ref_worker],
        vec!["--menu", "pdf", "--menu", "acme/pack/skills/pdf"],
        vec!["--menu", "acme/pack/skills/pdf", "--with", "pdf"],
    ] {
        let mut args = vec!["invalid"];
        args.extend(flags);
        args.push("task");
        assert_failure_contains("collision", &pack.spawn(&args), "collides");
        assert!(!state_dir(&pack.env.root).join("worker/invalid").exists());
    }
    let references: Vec<_> = (0..17)
        .map(|n| format!("acme/pack/skills/m{n}@{}", pack.sha))
        .collect();
    let mut args = vec!["invalid"];
    for reference in &references {
        args.extend(["--menu", reference]);
    }
    args.push("task");
    assert_failure_contains("inline ref menu cap", &pack.spawn(&args), "maximum is 16");
    assert_command_success("import", &pack.import("acme/pack/skills/pdf"));
    document(&pack.env.root.join(".niles"), "skills/pdf", "pdf", "local");
    let output = pack.import("acme/pack/skills/pdf");
    assert_failure_contains(
        "local import refusal",
        &output,
        "local skill already exists",
    );
    let manifest = state_dir(&pack.env.root).join("manifest.yaml");
    let body = fs::read_to_string(&manifest).unwrap();
    fs::write(&manifest, format!("{body}pdf: {{}}\n")).unwrap();
    let output = pack.run(&["models"]);
    assert_failure_contains("dual documents", &output, "has both");
    assert!(stderr_of(&output).contains("SKILL.md") && stderr_of(&output).contains("SKILL.ref"));
    fs::remove_file(pack.env.root.join(".niles/skills/pdf/SKILL.md")).unwrap();
    let pointer = pack.pointer("pdf");
    let outside = pack.env.root.join("pointer");
    fs::rename(&pointer, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &pointer).unwrap();
    assert_failure_contains(
        "workspace pointer symlink",
        &pack.run(&["models"]),
        "without symlinks",
    );
}

#[path = "skill_imports/hardening.rs"]
mod hardening;

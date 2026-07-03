use std::{fs, path::PathBuf, process::Command};

fn main() {
    track_git_head();

    println!(
        "cargo:rustc-env=NILES_BUILD_GIT_HASH={}",
        command_output("git", &["rev-parse", "--short=12", "HEAD"])
            .unwrap_or_else(|| "unknown".to_owned())
    );
    println!(
        "cargo:rustc-env=NILES_BUILD_HEAD_TIMESTAMP={}",
        command_output("git", &["show", "-s", "--format=%cI", "HEAD"])
            .unwrap_or_else(|| "unknown".to_owned())
    );
    println!(
        "cargo:rustc-env=NILES_BUILD_TIMESTAMP={}",
        command_output("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"])
            .unwrap_or_else(|| "unknown".to_owned())
    );
}

fn track_git_head() {
    println!("cargo:rerun-if-changed=.git");
    let Some(head_path) = git_path("HEAD") else {
        return;
    };
    println!("cargo:rerun-if-changed={}", head_path.display());
    let Ok(head) = fs::read_to_string(&head_path) else {
        return;
    };

    let Some(reference) = head.trim().strip_prefix("ref: ") else {
        return;
    };
    if let Some(reference_path) = git_path(reference) {
        println!("cargo:rerun-if-changed={}", reference_path.display());
    }
}

fn git_path(path: &str) -> Option<PathBuf> {
    command_output("git", &["rev-parse", "--git-path", path]).map(PathBuf::from)
}

fn command_output(binary: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(binary).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

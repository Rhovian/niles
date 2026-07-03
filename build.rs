use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_else(|| ".".into()));
    track_package_inputs(&manifest_dir);

    let repo = GitRepo::for_manifest(&manifest_dir);
    if let Some(repo) = &repo {
        repo.track_head();
    }

    println!(
        "cargo:rustc-env=NILES_BUILD_GIT_HASH={}",
        repo.as_ref()
            .and_then(GitRepo::build_hash)
            .unwrap_or_else(|| "unknown".to_owned())
    );
    println!(
        "cargo:rustc-env=NILES_BUILD_HEAD_TIMESTAMP={}",
        repo.as_ref()
            .and_then(|repo| repo.output(&["show", "-s", "--format=%cI", "HEAD"]))
            .unwrap_or_else(|| "unknown".to_owned())
    );
    println!(
        "cargo:rustc-env=NILES_BUILD_TIMESTAMP={}",
        command_output(Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]))
            .map(|value| value.trim().to_owned())
            .unwrap_or_else(|| "unknown".to_owned())
    );
}

fn track_package_inputs(manifest_dir: &Path) {
    for relative in [
        "build.rs",
        "Cargo.toml",
        "Cargo.lock",
        "src",
        "tests",
        "examples",
        "docs",
        "README.md",
    ] {
        let path = manifest_dir.join(relative);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

struct GitRepo {
    manifest_dir: PathBuf,
}

impl GitRepo {
    fn for_manifest(manifest_dir: &Path) -> Option<Self> {
        let top_level = command_output(
            Command::new("git")
                .arg("-C")
                .arg(manifest_dir)
                .args(["rev-parse", "--show-toplevel"]),
        )?;
        paths_equal(manifest_dir, Path::new(top_level.trim())).then(|| Self {
            manifest_dir: manifest_dir.to_path_buf(),
        })
    }

    fn build_hash(&self) -> Option<String> {
        let hash = self.output(&["rev-parse", "--short=12", "HEAD"])?;
        let dirty = self
            .output_allow_empty(&["status", "--porcelain"])
            .is_some_and(|status| !status.trim().is_empty());
        Some(if dirty { format!("{hash}-dirty") } else { hash })
    }

    fn track_head(&self) {
        let Some(head_path) = self.git_path("HEAD") else {
            return;
        };
        if head_path.exists() {
            println!("cargo:rerun-if-changed={}", head_path.display());
        }
        let Ok(head) = fs::read_to_string(&head_path) else {
            return;
        };

        let Some(reference) = head.trim().strip_prefix("ref: ") else {
            return;
        };
        if let Some(reference_path) = self.git_path(reference)
            && reference_path.exists()
        {
            println!("cargo:rerun-if-changed={}", reference_path.display());
        }
        if let Some(packed_refs) = self.git_path("packed-refs")
            && packed_refs.exists()
        {
            println!("cargo:rerun-if-changed={}", packed_refs.display());
        }
    }

    fn git_path(&self, path: &str) -> Option<PathBuf> {
        let path = self.output(&["rev-parse", "--git-path", path])?;
        let path = PathBuf::from(path);
        Some(if path.is_absolute() {
            path
        } else {
            self.manifest_dir.join(path)
        })
    }

    fn output(&self, args: &[&str]) -> Option<String> {
        let value = self.output_allow_empty(args)?;
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    }

    fn output_allow_empty(&self, args: &[&str]) -> Option<String> {
        command_output(
            Command::new("git")
                .arg("-C")
                .arg(&self.manifest_dir)
                .args(args),
        )
    }
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    let Ok(left) = left.canonicalize() else {
        return false;
    };
    let Ok(right) = right.canonicalize() else {
        return false;
    };
    left == right
}

fn command_output(command: &mut Command) -> Option<String> {
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

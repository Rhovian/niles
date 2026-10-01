use std::{fs, process::Command};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use chrono::{DateTime, Utc};

use crate::{
    build_info, schema,
    util::{current_dir_utf8, print_structured_rows},
};

const UNKNOWN_SOURCE_METADATA: &str = "unknown";
const SOURCE_DIFFERS: &str = "unknown (source HEAD differs from binary build)";

pub(crate) fn doctor() -> Result<()> {
    let workspace = current_dir_utf8()?;
    println!("binary: niles {}", build_info::CLAP_VERSION);
    println!("version: {}", build_info::VERSION);
    println!("git_hash: {}", build_info::GIT_HASH);
    println!("built_at: {}", build_info::BUILD_TIMESTAMP);
    println!("schema: {}", schema::CURRENT_SCHEMA);
    println!("workspace: {workspace}");

    let observations = schema::scan_workspace(&workspace)?;

    let has_schema_problem = observations
        .iter()
        .any(|observation| observation.status.is_problem());
    if observations.is_empty() {
        println!("schemas: none");
    } else {
        let rows = observations
            .iter()
            .map(|observation| {
                [
                    observation.kind.label().to_owned(),
                    display_path(&workspace, &observation.path),
                    observation.status.summary().to_owned(),
                ]
            })
            .collect::<Vec<_>>();
        print_structured_rows("schemas", ["kind", "path", "status"], &rows);
    }

    print_dev_mode(&workspace)?;
    if has_schema_problem {
        bail!("doctor found non-current or unreadable Niles artifacts");
    }
    Ok(())
}

fn print_dev_mode(workspace: &Utf8Path) -> Result<()> {
    if !is_niles_source_tree(workspace)? {
        println!("dev_mode: no");
        return Ok(());
    }

    println!("dev_mode: yes");
    let source_hash = git_output(workspace, &["rev-parse", "--short=12", "HEAD"]);
    let source_time = git_output(workspace, &["show", "-s", "--format=%cI", "HEAD"]);
    let dirty = worktree_dirty(workspace);
    println!(
        "source_head: {}",
        match source_hash.as_deref() {
            Some(value) => value,
            None => UNKNOWN_SOURCE_METADATA,
        }
    );
    println!(
        "source_head_time: {}",
        match source_time.as_deref() {
            Some(value) => value,
            None => UNKNOWN_SOURCE_METADATA,
        }
    );
    println!("binary_head: {}", build_info::GIT_HASH);
    println!("binary_head_time: {}", build_info::BUILD_HEAD_TIMESTAMP);
    println!(
        "working_tree: {}",
        match dirty {
            Some(true) => "dirty",
            Some(false) => "clean",
            None => "unknown",
        }
    );
    println!(
        "stale: {}",
        stale_status(source_hash.as_deref(), source_time.as_deref(), dirty)
    );
    Ok(())
}

fn is_niles_source_tree(workspace: &Utf8Path) -> Result<bool> {
    let cargo_toml = workspace.join("Cargo.toml");
    if !cargo_toml.is_file() || !workspace.join("src/main.rs").is_file() {
        return Ok(false);
    }
    let body =
        fs::read_to_string(&cargo_toml).with_context(|| format!("failed to read {cargo_toml}"))?;
    Ok(body.lines().any(|line| line.trim() == r#"name = "niles""#))
}

fn stale_status(
    source_hash: Option<&str>,
    source_time: Option<&str>,
    dirty: Option<bool>,
) -> String {
    if dirty == Some(true) {
        return "unknown (working tree dirty)".to_owned();
    }
    if dirty.is_none() {
        return "unknown (working tree status unavailable)".to_owned();
    }
    if build_info::GIT_HASH.ends_with("-dirty") {
        return "unknown (binary was built from a dirty tree)".to_owned();
    }
    if source_hash == Some(build_info::GIT_HASH) {
        return "no".to_owned();
    }

    match (
        source_time.and_then(parse_time),
        parse_time(build_info::BUILD_TIMESTAMP),
    ) {
        (Some(source), Some(build)) if source > build => {
            "yes (source HEAD is newer than this binary)".to_owned()
        }
        _ => SOURCE_DIFFERS.to_owned(),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "doctor staleness is advisory; malformed git/build timestamps make stale status unknown rather than failing doctor"
)]
fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn git_output(workspace: &Utf8Path, args: &[&str]) -> Option<String> {
    let value = git_output_raw(workspace, args)?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[expect(
    clippy::disallowed_methods,
    reason = "doctor dev-mode git metadata is advisory; git launch failure should be reported as unknown metadata"
)]
fn git_output_raw(workspace: &Utf8Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace.as_str())
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn worktree_dirty(workspace: &Utf8Path) -> Option<bool> {
    Some(
        !git_output_raw(workspace, &["status", "--porcelain"])?
            .trim()
            .is_empty(),
    )
}

fn display_path(workspace: &Utf8Path, path: &Utf8Path) -> String {
    if let Ok(relative) = path.strip_prefix(workspace) {
        return relative.to_string();
    }
    path.to_string()
}

use std::{fs, process::Command};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};

use crate::{
    build_info, schema,
    util::{absolute_path, current_dir_utf8},
};

pub(crate) fn doctor() -> Result<()> {
    let workspace = current_dir_utf8()?;
    println!("binary: {}", build_info::identity());
    println!("version: {}", build_info::VERSION);
    println!("git_hash: {}", build_info::GIT_HASH);
    println!("built_at: {}", build_info::BUILD_TIMESTAMP);
    println!("schema: {}", schema::CURRENT_SCHEMA);
    println!("workspace: {workspace}");

    let observations = schema::scan_workspace(&workspace)?;
    if observations.is_empty() {
        println!("schemas: none");
    } else {
        println!("schemas[{}]{{kind,path,status}}:", observations.len());
        for observation in observations {
            println!(
                "  {},{},{}",
                observation.kind.label(),
                display_path(&workspace, &observation.path),
                observation.status.summary()
            );
        }
    }

    print_dev_mode(&workspace)?;
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
    println!(
        "source_head: {}",
        source_hash.as_deref().unwrap_or("unknown")
    );
    println!(
        "source_head_time: {}",
        source_time.as_deref().unwrap_or("unknown")
    );
    println!("binary_head: {}", build_info::GIT_HASH);
    println!("binary_head_time: {}", build_info::BUILD_HEAD_TIMESTAMP);
    println!(
        "stale: {}",
        stale_status(source_hash.as_deref(), source_time.as_deref())
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

fn stale_status(source_hash: Option<&str>, source_time: Option<&str>) -> String {
    if source_hash == Some(build_info::GIT_HASH) {
        return "no".to_owned();
    }

    let Some(source_time) = source_time.and_then(parse_time) else {
        return "unknown (source HEAD differs from binary build)".to_owned();
    };
    let Some(build_time) = parse_time(build_info::BUILD_TIMESTAMP) else {
        return "unknown (source HEAD differs from binary build)".to_owned();
    };
    if source_time > build_time {
        "yes (source HEAD is newer than this binary)".to_owned()
    } else {
        "unknown (source HEAD differs from binary build)".to_owned()
    }
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn git_output(workspace: &Utf8Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace.as_str())
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn display_path(workspace: &Utf8Path, path: &Utf8Path) -> String {
    let workspace = absolute_path(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    if let Ok(relative) = path.strip_prefix(&workspace) {
        return relative.to_string();
    }
    path.to_string()
}

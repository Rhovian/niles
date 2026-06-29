use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::Serialize;

pub fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            slug.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            slug.push('-');
            last_was_separator = true;
        }
    }

    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "step".to_owned()
    } else {
        slug.to_owned()
    }
}

pub fn timestamp_id(now: &DateTime<Utc>) -> String {
    format!(
        "{}{:09}Z",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    )
}

pub fn utf8_path(path: PathBuf, description: &str) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path)
        .map_err(|path| anyhow::anyhow!("{description} is not UTF-8: {}", path.display()))
}

pub fn current_dir_utf8() -> Result<Utf8PathBuf> {
    utf8_path(
        env::current_dir().context("failed to read current directory")?,
        "current directory",
    )
}

pub fn absolute_path(path: &Utf8Path) -> Result<Utf8PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    Ok(current_dir_utf8()?.join(path))
}

pub fn absolute_existing_dir(path: &Utf8Path, description: &str) -> Result<Utf8PathBuf> {
    let path = absolute_path(path)?;
    if !path.is_dir() {
        bail!("{description} path is not a directory: {path}");
    }
    Ok(path)
}

pub fn absolute_existing_file(path: &Utf8Path, description: &str) -> Result<Utf8PathBuf> {
    let path = absolute_path(path)?;
    if !path.is_file() {
        bail!("{description} path is not a file: {path}");
    }
    Ok(path)
}

pub fn write_json_pretty<T>(path: &Utf8Path, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    fs::write(path, serde_json::to_string_pretty(value)?)
        .with_context(|| format!("failed to write {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_path_keeps_absolute_paths() {
        let path = Utf8Path::new("/tmp/niles-absolute-path-test");

        assert_eq!(absolute_path(path).unwrap(), path);
    }

    #[test]
    fn absolute_path_joins_relative_paths_to_current_dir() {
        let cwd = current_dir_utf8().unwrap();

        assert_eq!(
            absolute_path(Utf8Path::new("relative/path")).unwrap(),
            cwd.join("relative/path")
        );
    }

    #[test]
    fn absolute_existing_dir_accepts_existing_directories() {
        let dir = temp_test_path("dir");
        fs::create_dir_all(&dir).unwrap();

        assert_eq!(absolute_existing_dir(&dir, "project").unwrap(), dir);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn absolute_existing_dir_rejects_files() {
        let dir = temp_test_path("dir-file");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("brief.md");
        fs::write(&file, "brief").unwrap();

        let err = absolute_existing_dir(&file, "project").unwrap_err();
        assert!(err.to_string().contains("project path is not a directory"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn absolute_existing_file_accepts_existing_files() {
        let dir = temp_test_path("file");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("brief.md");
        fs::write(&file, "brief").unwrap();

        assert_eq!(absolute_existing_file(&file, "brief").unwrap(), file);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn absolute_existing_file_rejects_directories() {
        let dir = temp_test_path("file-dir");
        fs::create_dir_all(&dir).unwrap();

        let err = absolute_existing_file(&dir, "brief").unwrap_err();
        assert!(err.to_string().contains("brief path is not a file"));

        fs::remove_dir_all(&dir).unwrap();
    }

    fn temp_test_path(label: &str) -> Utf8PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        utf8_path(
            env::temp_dir().join(format!("niles-util-{label}-{}-{nanos}", std::process::id())),
            "test temp path",
        )
        .unwrap()
    }

    #[test]
    fn slugify_normalizes_labels_for_step_filenames() {
        assert_eq!(slugify("Hello World!"), "hello-world");
        assert_eq!(slugify("Already_OK-123"), "already_ok-123");
    }

    #[test]
    fn slugify_collapses_and_trims_separators() {
        assert_eq!(slugify("--Hello!!!World--"), "hello-world");
        assert_eq!(slugify("a / b : c"), "a-b-c");
    }

    #[test]
    fn slugify_uses_step_for_empty_slugs() {
        assert_eq!(slugify("!!!"), "step");
        assert_eq!(slugify(""), "step");
    }

    #[test]
    fn timestamp_id_uses_utc_timestamp_with_padded_nanoseconds() {
        let now = DateTime::<Utc>::from_timestamp(0, 42).expect("valid timestamp");

        assert_eq!(timestamp_id(&now), "19700101T000000000000042Z");
    }
}

use std::{
    env, fs,
    io::{ErrorKind, Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Serialize, de::DeserializeOwned};

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

pub fn absolute_path_from(base: &Utf8Path, path: &Utf8Path) -> Utf8PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

pub fn absolute_existing_dir(path: &Utf8Path, description: &str) -> Result<Utf8PathBuf> {
    absolute_existing_path(path, description, "directory", Utf8Path::is_dir)
}

pub fn absolute_existing_file(path: &Utf8Path, description: &str) -> Result<Utf8PathBuf> {
    absolute_existing_path(path, description, "file", Utf8Path::is_file)
}

fn absolute_existing_path(
    path: &Utf8Path,
    description: &str,
    kind: &str,
    exists_as_kind: fn(&Utf8Path) -> bool,
) -> Result<Utf8PathBuf> {
    let path = absolute_path(path)?;
    if !exists_as_kind(&path) {
        bail!("{description} path is not a {kind}: {path}");
    }
    Ok(path)
}

pub fn read_optional_to_string(
    path: &Utf8Path,
    read_context: impl FnOnce(&Utf8Path) -> String,
) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(body) => Ok(Some(body)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| read_context(path)),
    }
}

pub fn read_optional_json<T>(
    path: &Utf8Path,
    read_context: impl FnOnce(&Utf8Path) -> String,
    parse_context: impl FnOnce(&Utf8Path) -> String,
) -> Result<Option<T>>
where
    T: DeserializeOwned,
{
    let Some(body) = read_optional_to_string(path, read_context)? else {
        return Ok(None);
    };

    serde_json::from_str(&body)
        .map(Some)
        .with_context(|| parse_context(path))
}

pub fn write_json_pretty<T>(path: &Utf8Path, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    fs::write(path, serde_json::to_string_pretty(value)?)
        .with_context(|| format!("failed to write {path}"))
}

pub fn append_line(
    path: &Utf8Path,
    line: &str,
    open_context: impl FnOnce(&Utf8Path) -> String,
    inspect_context: impl FnOnce(&Utf8Path) -> String,
    write_context: impl Fn(&Utf8Path) -> String,
) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .with_context(|| open_context(path))?;
    if needs_leading_newline(&mut file).with_context(|| inspect_context(path))? {
        file.write_all(b"\n").with_context(|| write_context(path))?;
    }
    writeln!(file, "{line}").with_context(|| write_context(path))
}

fn needs_leading_newline(file: &mut fs::File) -> Result<bool> {
    if file.metadata()?.len() == 0 {
        return Ok(false);
    }

    file.seek(SeekFrom::End(-1))?;
    let mut byte = [0];
    file.read_exact(&mut byte)?;
    Ok(byte[0] != b'\n')
}

pub fn remove_file_if_exists(path: &Utf8Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove {path}")),
    }
}

pub fn remove_dir_all_if_exists(path: &Utf8Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove {path}")),
    }
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
    fn absolute_path_from_joins_relative_paths_to_base() {
        let base = Utf8Path::new("/tmp/niles-base");

        assert_eq!(
            absolute_path_from(base, Utf8Path::new("relative/path")),
            base.join("relative/path")
        );
    }

    #[test]
    fn absolute_path_from_keeps_absolute_paths() {
        let base = Utf8Path::new("/tmp/niles-base");
        let path = Utf8Path::new("/tmp/niles-absolute-path-test");

        assert_eq!(absolute_path_from(base, path), path);
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

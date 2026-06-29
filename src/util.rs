use std::fs;

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::{DateTime, Utc};
use serde::Serialize;

pub fn slugify(value: &str) -> String {
    let mut slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();

    while slug.contains("--") {
        slug = slug.replace("--", "-");
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

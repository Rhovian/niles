use camino::Utf8PathBuf;

pub(in crate::schema) fn temp_test_path(label: &str) -> Utf8PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
        "niles-schema-{label}-{}-{nanos}",
        std::process::id()
    )))
    .unwrap()
}

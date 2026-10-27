use super::*;

#[test]
fn cleanup_failure_preserves_fetch_cause_and_reports_both() {
    let temporary = tempfile::tempdir().unwrap();
    let staging = Utf8Path::from_path(temporary.path())
        .unwrap()
        .join("missing");
    let error = finish_fetch(&staging, Err(anyhow::anyhow!("original fetch error"))).unwrap_err();
    assert_eq!(error.root_cause().to_string(), "original fetch error");
    let text = format!("{error:#}");
    assert!(text.contains("failed to clean skill staging directory"));
    assert!(text.contains(staging.as_str()));
    let error = finish_fetch(&staging, Ok(())).unwrap_err();
    assert!(format!("{error:#}").contains("failed to clean skill staging directory"));
}

#[test]
fn a_complete_competing_entry_wins_the_rename_race() {
    let temporary = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(temporary.path()).unwrap();
    let staged = root.join("staged");
    let target = root.join("target");
    for dir in [&staged, &target] {
        fs::create_dir(dir).unwrap();
    }
    fs::write(staged.join("SKILL.md"), "staged").unwrap();
    fs::write(target.join("SKILL.md"), "winner").unwrap();
    install(&staged, &target).unwrap();
    assert_eq!(
        fs::read_to_string(target.join("SKILL.md")).unwrap(),
        "winner"
    );
    assert!(staged.join("SKILL.md").is_file());
}

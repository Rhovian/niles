use super::*;

#[test]
fn names_refs_hosts_paths_and_revisions_parse_once() {
    assert!(matches!(
        "pdf".parse::<MenuItem>().unwrap(),
        MenuItem::Name(_)
    ));
    for (input, name, path, rev) in [
        ("acme/pdf", "pdf", "", "HEAD"),
        ("github.com/acme/pdf", "pdf", "", "HEAD"),
        ("acme/pack/skills/pdf@main", "pdf", "skills/pdf", "main"),
        ("acme/pack/pdf@v1.0", "pdf", "pdf", "v1.0"),
        (
            "acme/pack/pdf@feature/branch",
            "pdf",
            "pdf",
            "feature/branch",
        ),
        (
            "acme/pack/pdf@0123456789012345678901234567890123456789",
            "pdf",
            "pdf",
            "0123456789012345678901234567890123456789",
        ),
    ] {
        let MenuItem::Ref(reference) = input.parse().unwrap() else {
            panic!("{input}")
        };
        assert_eq!(reference.name.as_str(), name);
        assert_eq!(reference.path, path);
        assert_eq!(reference.rev, rev);
        assert!(reference.to_string().starts_with("github.com/acme/"));
    }
    for input in [
        "gitlab.com/acme/pdf",
        "github.com/pdf",
        "acme//pdf",
        "acme/pack/./pdf",
        "acme/pack/../pdf",
        "acme/pack/bad name",
        "acme/pack/pdf@",
        "acme/pack/pdf@refs/heads/x\n",
        "acme/pack/pdf\0",
        "https://github.com/acme/pdf",
    ] {
        assert!(input.parse::<MenuItem>().is_err(), "{input}");
    }
}

#[test]
fn resting_refs_and_metadata_require_canonical_full_sha() {
    let canonical = "github.com/acme/pack/pdf@0123456789012345678901234567890123456789";
    let pinned: PinnedRef = canonical.to_owned().try_into().unwrap();
    assert_eq!(pinned.to_string(), canonical);
    let non_hex = format!("github.com/acme/pdf@{}", "g".repeat(40));
    for text in [
        "github.com/acme/pack/pdf@main",
        "acme/pack/pdf@0123456789012345678901234567890123456789",
        &non_hex,
    ] {
        assert!(serde_json::from_value::<PinnedRef>(serde_json::json!(text)).is_err());
    }
}

#[test]
fn pointer_documents_are_exactly_one_line() {
    let line = "github.com/acme/pdf@0123456789012345678901234567890123456789";
    for suffix in ["", "\n", "\r\n"] {
        assert!(PinnedRef::document(&format!("{line}{suffix}")).is_ok());
    }
    for suffix in ["\n\n", "\nsecond line", " ", "\r"] {
        assert!(PinnedRef::document(&format!("{line}{suffix}")).is_err());
    }
}

#[test]
fn typed_pin_conversion_and_sha_replacement_have_explicit_case_rules() {
    let upper = "ABCDEF0123456789ABCDEF0123456789ABCDEF01";
    let reference: GithubRef = format!("acme/pack/pdf@{upper}").parse().unwrap();
    let pinned = PinnedRef::try_from(reference).unwrap();
    assert_eq!(pinned.reference().rev, upper.to_ascii_lowercase());
    let floating: GithubRef = "acme/pack/pdf@main".parse().unwrap();
    assert!(PinnedRef::try_from(floating.clone()).is_err());
    assert_eq!(floating.with_sha(upper.to_owned()).unwrap(), pinned);
    let error = PinnedRef::document(&format!("github.com/acme/pack/pdf@{upper}")).unwrap_err();
    assert!(error.to_string().contains("lowercase SHA"));
}

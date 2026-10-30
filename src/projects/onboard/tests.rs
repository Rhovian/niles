use super::*;
#[test]
fn generated_defaults_match_literal_expectations() {
    for name in crate::store::parse_yaml::<Vec<skills::SkillName>>(defaults::SKILLS).unwrap() {
        defaults::reference(&name).unwrap();
        let path = format!("{}/skills/{name}/SKILL.md", env!("CARGO_MANIFEST_DIR"));
        assert!(Utf8Path::new(&path).exists());
    }
    let roster = crate::agents::ModelRoster::builtin().unwrap();
    for installed in "claude;codex;pi;claude codex;claude pi;codex pi;claude codex pi".split(';') {
        let generated =
            defaults::manifest(&installed.split_whitespace().collect::<Vec<_>>()).unwrap();
        manifest::check(&generated, &roster).unwrap();
        assert!(generated.bindings.is_empty());
    }
    let cases: std::collections::BTreeMap<String, String> =
        crate::store::parse_yaml(include_str!("../../test_support/onboarding.yaml")).unwrap();
    for (installed, expected) in cases {
        let generated =
            defaults::manifest(&installed.split_whitespace().collect::<Vec<_>>()).unwrap();
        let mut actual = Vec::new();
        for (tier, agents) in generated.tiers.iter() {
            let families = std::iter::once(&agents.primary)
                .chain(&agents.secondaries)
                .map(|agent| agent.split_once(':').unwrap().0)
                .collect::<Vec<_>>()
                .join(",");
            actual.push(format!("{}={families}", tier.as_str()));
        }
        assert_eq!(actual.join("\n") + "\n", expected, "{installed}");
    }
}

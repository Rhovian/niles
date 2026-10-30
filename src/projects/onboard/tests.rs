use super::*;
#[test]
fn generated_defaults_match_literal_expectations() {
    let roster = crate::agents::ModelRoster::builtin().unwrap();
    for installed in "claude;codex;pi;claude codex;claude pi;codex pi;claude codex pi".split(';') {
        let generated =
            defaults::manifest(&installed.split_whitespace().collect::<Vec<_>>()).unwrap();
        manifest::check(&generated, &roster).unwrap();
    }
    let cases: std::collections::BTreeMap<String, String> =
        crate::store::parse_yaml(include_str!("../../test_support/onboarding.yaml")).unwrap();
    for (installed, expected) in cases {
        let generated =
            defaults::manifest(&installed.split_whitespace().collect::<Vec<_>>()).unwrap();
        let mut actual = Vec::new();
        for (name, binding) in generated.bindings {
            defaults::reference(&name).unwrap();
            let path = format!("{}/skills/{name}/SKILL.md", env!("CARGO_MANIFEST_DIR"));
            assert!(Utf8Path::new(&path).exists());
            let mut row = name.to_string();
            for (tier, agents) in binding.tiers() {
                let families = std::iter::once(&agents.primary)
                    .chain(&agents.secondaries)
                    .map(|agent| agent.split_once(':').unwrap().0)
                    .collect::<Vec<_>>()
                    .join(",");
                row.push_str(&format!(" {}={families}", tier.as_str()));
            }
            actual.push(row);
        }
        assert_eq!(actual.join("\n") + "\n", expected, "{installed}");
    }
}

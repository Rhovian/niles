use super::*;

#[test]
fn retier_acknowledgements_require_applied_values_and_session_scope() {
    let steps = agents::switch_steps("claude", "opus", "low").unwrap();
    let reply = &steps[2].acknowledgement;
    for line in [
        "Set model to Sonnet 5.5 for this session only with low effort",
        "Set model to Opus 5.5 and saved as your default with low effort",
        "Set model to Opus 5.5 for this session only with high effort",
        "❯ Set model to Opus 5.5 for this session only with low effort",
    ] {
        assert_eq!(acknowledgements(line, reply), 0);
    }
    let line = "  ⎿ Set model to Opus 5.5 for this session only with low effort";
    let before = acknowledgements(line, reply);
    assert_eq!(before, 1);
    assert_eq!(acknowledgements(line, reply), before);
    assert_eq!(
        acknowledgements(&format!("{line}\n{line}"), reply),
        before + 1
    );
    assert_eq!(steps[1].keys, ["Down"]);
    assert_eq!(steps[2].keys, ["Left", "Left", "Left", "Left", "s"]);
}

#[test]
fn retier_inline_sequences_confirm_model_before_effort() {
    for (family, command) in [("hermes", "/reasoning low"), ("pi", "/thinking low")] {
        let steps = agents::switch_steps(family, "tencent/hy3", "low").unwrap();
        assert!(steps[0].text.starts_with("/model tencent/hy3"));
        assert_eq!(steps[1].text, command);
        let acknowledgement = if family == "hermes" {
            "✓ Reasoning effort set to 'low'"
        } else {
            "Thinking level: low"
        };
        assert_eq!(
            acknowledgements(acknowledgement, &steps[1].acknowledgement),
            1
        );
    }
}

#[test]
fn retier_context_seam_refuses_oversize_and_unknown_conversations() {
    assert!(check_context_fit(Some(101), 100).is_err());
    assert!(check_context_fit(None, 100).is_err());
    assert!(check_context_fit(Some(100), 100).is_ok());
}

#[test]
fn retier_context_uses_the_exact_effort_entry() {
    let models = agents::roster::parse(agents::roster::FIXTURE_REGISTRY).unwrap();
    let high = models.entry("codex", "fixture", Some("high")).unwrap();
    assert!(check_context_fit(Some(1500), high.context_window.get()).is_ok());
}

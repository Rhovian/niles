use super::*;

#[test]
fn retier_codex_reads_picker_states_and_selected_labels() {
    for (title, row, picker, name) in [
        ("Select Model", "3. All models", Picker::Quick, "All models"),
        (
            "Select Model and Effort",
            "12. GPT-6.1-Sol (current)  Workhorse",
            Picker::Models,
            "GPT-6.1-Sol",
        ),
        (
            "Select Reasoning Level for GPT-6.1-Sol",
            "2. Medium (default) (current)  Balanced",
            Picker::Effort,
            "Medium",
        ),
        ("Advanced Reasoning", "2. Ultra", Picker::Advanced, "Ultra"),
    ] {
        let screen = format!("old transcript\n  {title}\n\n› {row}\n  enter select · esc back");
        assert_eq!(selection(&screen, "gpt-6.1-sol").unwrap(), (picker, name));
    }
}

#[test]
fn retier_codex_rejects_unknown_or_ambiguous_picker_state() {
    for screen in [
        "› 1. GPT-6.1-Sol",
        "Select Model and Effort\n1. GPT-6.1-Sol",
        "Select Model and Effort\n› 1. GPT-6.1-Sol\n› 2. GPT-6-Sol",
        "Select Model and Effort\n› a. GPT-6.1-Sol",
        "Select Model and Effort\n› . GPT-6.1-Sol",
        "Select Model and Effort\n› 1. ",
        "Select Reasoning Level for GPT-6-Sol\n› 1. Medium",
    ] {
        assert!(selection(screen, "gpt-6.1-sol").is_err(), "{screen}");
    }
}

#[test]
fn retier_codex_confirms_fresh_exact_model_effort_and_session_scope() {
    let reply = (
        "Model changed to gpt-6.1-sol high for this session only".into(),
        String::new(),
    );
    let success = "• Model changed to gpt-6.1-sol high for this session only";
    assert_eq!(super::super::acknowledgements(success, &reply), 1);
    assert_eq!(
        super::super::acknowledgements(&format!("{success}\n{success}"), &reply),
        2
    );
    for line in [
        "• Model changed to gpt-6-sol high for this session only",
        "• Model changed to gpt-6.1-sol xhigh for this session only",
        "• Model changed to gpt-6.1-sol high",
        "› Model changed to gpt-6.1-sol high for this session only",
    ] {
        assert_eq!(super::super::acknowledgements(line, &reply), 0);
    }
}

#[test]
fn retier_codex_update_error_vetoes_even_a_success_acknowledgement() {
    let error = "■ Failed to update thread settings: rejected";
    let success = "• Model changed to gpt-6.1-sol high for this session only";
    assert!(check_update("", &format!("{error}\n{success}")).is_err());
    assert!(check_update(error, &format!("{error}\n{success}")).is_ok());
    assert!(check_update(error, &format!("{error}\n{error}\n{success}")).is_err());
}

#[test]
#[ignore = "requires NILES_TEST_CODEX_PANE pointing to an idle installed Codex 0.162.1 TUI"]
fn retier_codex_live_picker() {
    let pane = tmux::TmuxTarget::pane(&std::env::var("NILES_TEST_CODEX_PANE").unwrap()).unwrap();
    let switch_to = |model, effort| {
        for step in crate::agents::switch_steps("codex", model, effort).unwrap() {
            confirm(&pane, &step).unwrap();
        }
        switch(&pane, model, effort).unwrap();
        println!("confirmed {model}:{effort}");
    };
    for (model, effort) in [
        ("gpt-6.1-sol", "low"),
        ("gpt-6-sol", "xhigh"),
        ("gpt-6.1-sol", "max"),
        ("gpt-6.1-sol", "ultra"),
        ("gpt-6.1-sol", "medium"),
    ] {
        switch_to(model, effort);
    }
    tmux::send_input(&pane, "/plan", &["C-m"]).unwrap();
    assert!(
        tmux::capture_visible_pane(&pane)
            .unwrap()
            .contains("Plan mode")
    );
    for effort in ["high", "medium"] {
        switch_to("gpt-6.1-sol", effort);
    }
}

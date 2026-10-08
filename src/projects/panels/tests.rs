use std::fs;

use super::*;
use crate::{projects::registry::ProjectName, test_support::temp_test_path};

#[test]
fn help_text_depends_on_whether_registry_is_empty() {
    let theme = Theme::parse(None).unwrap();
    assert_eq!(
        help_text(true, &theme),
        FIRST_RUN
            .replacen("niles", &theme.style(StyleKey::Heading).paint("niles"), 1)
            .replace(
                "Get started",
                &theme.style(StyleKey::Heading).paint("Get started")
            )
    );
    assert!(help_text(false, &theme).contains(&format!(
        "{} running   {} waiting",
        theme.state(State::Running).1.paint("⣾"),
        theme.state(State::Waiting).1.paint("⚠")
    )));
}

#[test]
fn config_prints_models_files_errors_and_project_overrides() {
    let theme = Theme::parse(None).unwrap();
    let paint = |key, text: &str| theme.style(key).paint(text);
    let entries = ["missing", "valid", "invalid", "override"].map(|name| registry::Entry {
        name: ProjectName::parse(name).unwrap(),
        path: temp_test_path(&format!("panel-{name}")),
    });
    let builtin = ModelRoster::builtin().unwrap().rows();
    let mut expected = format!(
        "{}\n{}",
        paint(StyleKey::Heading, "MODELS"),
        models_table(&builtin, &theme)
    );
    for (index, entry) in entries.iter().enumerate() {
        expected += &format!(
            "\n{}  {}\n",
            paint(StyleKey::Heading, entry.name.as_str()),
            paint(StyleKey::Muted, entry.path.as_str())
        );
        if index == 0 {
            expected += &format!("{} {MANIFEST}\n", paint(StyleKey::Lost, "✗"));
            continue;
        }
        fs::create_dir_all(entry.path.join(".niles")).unwrap();
        fs::write(
            entry.path.join(MANIFEST),
            "lead: claude\nworker: codex\nreviewer: lead\nsecurity: claude\n",
        )
        .unwrap();
        if index == 2 {
            fs::write(entry.path.join(MANIFEST), "bad\x1b\nkey: true\n").unwrap();
            let error = format!("{:#}", workspace_manifest::load(&entry.path).unwrap_err());
            expected += &format!(
                "{} {}\n",
                paint(StyleKey::Lost, "✗"),
                workspace_manifest::clamp(&error, ERROR_WIDTH)
            );
            continue;
        }
        fs::write(
            entry.path.join("niles.yaml"),
            if index == 3 {
                "models: { codex: { gpt-5.5: { efforts: [high] } } }\n"
            } else {
                "agents: {}\n"
            },
        )
        .unwrap();
        for (role, family) in [
            ("lead    ", "claude"),
            ("worker  ", "codex "),
            ("reviewer", "lead  "),
            ("security", "claude"),
        ] {
            expected += &format!(
                "{}  {}  {}  {}\n",
                paint(StyleKey::Accent, role),
                Style::new().paint(family),
                paint(StyleKey::Heading, "-"),
                paint(StyleKey::Muted, "-")
            );
        }
        expected += &format!(
            "{} {MANIFEST}   {} niles.yaml\n",
            paint(StyleKey::Running, "✓"),
            paint(StyleKey::Running, "✓")
        );
        if index == 3 {
            expected += &format!(
                "{}\n{}",
                paint(StyleKey::Muted, "models (project overrides)"),
                models_table(&[["codex", "gpt-5.5", "high"].map(str::to_owned)], &theme)
            );
        }
    }
    let output = config_panel(&entries, &theme).unwrap();
    assert_eq!(output, expected);
    assert_eq!(
        output.matches(&paint(StyleKey::Heading, "MODELS")).count(),
        1
    );
    assert_eq!(output.matches("models (project overrides)").count(), 1);
    assert!(!output.contains("bad\x1b\nkey"));
    for entry in &entries[1..] {
        fs::remove_dir_all(&entry.path).unwrap();
    }
}

#[test]
fn models_table_aligns_clamps_and_groups_families() {
    let theme = Theme::parse(None).unwrap();
    let rows = [
        ["pi".into(), "é".into(), "high".into()],
        ["pi".into(), "a\x1b\nb".into(), "".into()],
        ["codex".into(), "x".repeat(41), "low".into()],
    ];
    let expected = [
        ("pi   ", format!("é{}", " ".repeat(40)), "high"),
        ("     ", format!("ab…{}", " ".repeat(38)), "    "),
        ("codex", format!("{}…", "x".repeat(40)), "low "),
    ]
    .map(|(family, model, efforts)| {
        format!(
            "{}  {}  {}\n",
            theme.style(StyleKey::Accent).paint(family),
            Style::new().paint(&model),
            theme.style(StyleKey::Muted).paint(efforts)
        )
    })
    .concat();
    assert_eq!(models_table(&rows, &theme), expected);
}

#[test]
fn telemetry_aligns_projects_known_unknown_usage_costs_and_lost_windows() {
    let theme = Theme::parse(None).unwrap();
    let paint = |key, text: &str| theme.style(key).paint(text);
    let known = |id: &str, state, lost| SessionUsage {
        id: id.into(),
        role: "lead",
        agent: "claude".into(),
        window_gone: lost,
        usage: Some(crate::telemetry::Usage {
            input_tokens: 10_000,
            output_tokens: 2_000,
            cache_read_tokens: 0,
            cache_write_tokens: None,
            reasoning_tokens: None,
            last_turn_at: None,
            state: Some(state),
            estimated_cost_usd: Some(0.03),
        }),
    };
    let projects = vec![
        (
            "api".into(),
            vec![known(
                "2026-10-08T12-34-56.123456",
                SessionState::Working,
                false,
            )],
        ),
        ("empty".into(), vec![]),
        (
            "web".into(),
            vec![
                SessionUsage {
                    id: "parse-long".into(),
                    role: "worker",
                    agent: "codex".into(),
                    usage: None,
                    window_gone: true,
                },
                known("2026-10-08T12-34-57.123456", SessionState::Waiting, false),
            ],
        ),
    ];
    let plain = |text: &str| Style::new().paint(text);
    let expected = format!(
        "  {}  {}  {}  {}  {}  {}\n{}\n  {}  {}  {}  {}  {}  {}\n{}\n  {}  {}  {}  {}  {}  {}  {}\n  {}  {}  {}  {}  {}  {}\n{}\n  {}  {}  {}  {}  {}  {}\n",
        plain(" "),
        plain("          "),
        plain("      "),
        plain("      "),
        paint(StyleKey::Muted, "TOKENS"),
        paint(StyleKey::Muted, " COST"),
        paint(StyleKey::Heading, "api"),
        paint(StyleKey::Running, "●"),
        paint(StyleKey::Accent, "lead      "),
        paint(StyleKey::Muted, "lead  "),
        plain("claude"),
        paint(StyleKey::Heading, "   12k"),
        paint(StyleKey::Muted, "$0.03"),
        paint(StyleKey::Heading, "web"),
        plain(" "),
        paint(StyleKey::Accent, "parse-long"),
        paint(StyleKey::Muted, "worker"),
        plain("codex "),
        paint(StyleKey::Heading, "     ?"),
        paint(StyleKey::Muted, "     "),
        paint(StyleKey::Lost, "window lost"),
        paint(StyleKey::Idle, "○"),
        paint(StyleKey::Accent, "lead      "),
        paint(StyleKey::Muted, "lead  "),
        plain("claude"),
        paint(StyleKey::Heading, "   12k"),
        paint(StyleKey::Muted, "$0.03"),
        paint(StyleKey::Guide, &"─".repeat(46)),
        plain(" "),
        paint(StyleKey::Heading, "total     "),
        plain("      "),
        plain("      "),
        paint(StyleKey::Heading, "   24k"),
        paint(StyleKey::Muted, "$0.06")
    );
    assert_eq!(telemetry(&projects, &theme).unwrap(), expected);
    assert_eq!(
        telemetry(&[], &theme).unwrap(),
        format!("{}\n", paint(StyleKey::Muted, "no live sessions"))
    );
    assert_eq!(
        telemetry(&[("empty".into(), vec![])], &theme).unwrap(),
        format!("{}\n", paint(StyleKey::Muted, "no live sessions"))
    );
}

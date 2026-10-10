use std::fs;

use camino::Utf8Path;
use chrono::{NaiveDate, TimeZone};
use serde_json::json;

use super::{
    aggregate::dashboard,
    collect::{Event, EventKind, Live, Project, Session, project},
    *,
};
use crate::{
    projects::registry::{Entry, ProjectName},
    telemetry::{Usage, fixtures},
    test_support::temp_test_path,
    theme::{StyleKey, StyleRender},
    util::timestamp_id,
    wake::WakeKind,
};

fn local(day: u32, hour: u32) -> DateTime<Local> {
    let time = NaiveDate::from_ymd_opt(2026, 10, day)
        .unwrap()
        .and_hms_opt(hour, 0, 0)
        .unwrap();
    Local.from_local_datetime(&time).unwrap()
}

fn utc(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

#[test]
fn today_starts_at_local_midnight_and_bins_cover_each_window_exactly() {
    let now = local(9, 15);
    for range in Range::ALL {
        let window = Window::new(range, now).unwrap();
        let end = window.bin_start(window.count);
        let mut bucket = window.start;
        let mut bins = Vec::new();
        while bucket < end {
            bins.push(window.bin_of(bucket).unwrap());
            bucket += BUCKET;
        }
        let per_bin = (window.bin.num_seconds() / BUCKET.num_seconds()) as usize;
        assert_eq!(
            bins,
            (0..window.count as usize)
                .flat_map(|bin| [bin].repeat(per_bin))
                .collect::<Vec<_>>()
        );
    }
}

fn write(path: &Utf8Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn pi_link(dir: &Utf8Path) -> serde_json::Value {
    write(&dir.join("session.jsonl"), fixtures::PI);
    json!({ "family": "pi", "session_dir": dir })
}

fn totals(project: &Project) -> Vec<(&str, &str, u64)> {
    project
        .sessions
        .iter()
        .map(|session| {
            let tokens = session.buckets.iter().map(|(_, tokens)| tokens).sum();
            (session.id.as_str(), session.role, tokens)
        })
        .collect()
}

#[test]
fn closed_sessions_are_read_once_and_the_latest_lead_never_cached() {
    let root = temp_test_path("telemetry-collect");
    let entry = Entry {
        name: ProjectName::parse("demo").unwrap(),
        path: root.join("demo"),
    };
    let niles = entry.path.join(".niles");
    let archive = |id: &str, closed: &str, link: Option<serde_json::Value>| {
        let dir = niles.join(format!(
            "worker/archive/{id}-{}",
            timestamp_id(&utc(closed))
        ));
        let unlinked = link.is_none();
        let mut meta = json!({
            "id": id, "role": "worker", "agent": "pi:tencent/hy3:high",
            "created_at": "2026-10-01T07:00:00Z", "project": entry.path,
            "window": format!("demo:{id}"), "launch": niles.join(format!("worker/{id}/launch.sh")),
            "brief": niles.join(format!("worker/{id}/brief.md")), "session_link": link,
        });
        // Archives from before `session_link` also predate `role`.
        if unlinked {
            meta.as_object_mut().unwrap().remove("role");
        }
        write(&dir.join("meta.json"), &meta.to_string());
        dir
    };
    let lead = |started: &str, link: serde_json::Value| {
        let id = timestamp_id(&utc(started));
        let dir = niles.join("sessions").join(&id);
        let meta = json!({
            "id": id, "agent": "pi:tencent/hy3:high", "created_at": started,
            "workspace": entry.path, "brief": dir.join("lead.md"), "session_link": link,
        });
        write(&dir.join("session.json"), &meta.to_string());
        dir
    };
    let transcripts = root.join("transcripts");
    let closed = archive(
        "impl",
        "2026-10-01T17:00:00Z",
        Some(pi_link(&transcripts.join("impl"))),
    );
    let unlinked = archive("nolink", "2026-10-01T18:00:00Z", None);
    let stale = archive(
        "old",
        "2026-09-01T00:00:00Z",
        Some(pi_link(&transcripts.join("old"))),
    );
    let superseded = lead("2026-10-01T06:00:00Z", pi_link(&transcripts.join("lead1")));
    write(
        &superseded.join("watch.log"),
        "2026-10-01T12:00:00Z nudged impl: niles: impl reported (done) — check workers\n\
         2026-10-01T12:01:00Z check-in disarmed: impl answered it\n",
    );
    let latest = lead("2026-10-01T15:00:00Z", pi_link(&transcripts.join("lead2")));
    let since = utc("2026-09-20T00:00:00Z");
    let pi_total = fixtures::pi().total_tokens();

    let first = project(&entry, false, since).unwrap();
    let latest_id = latest.file_name().unwrap();
    let superseded_id = superseded.file_name().unwrap();
    assert_eq!(
        totals(&first),
        [
            (latest_id, "lead", pi_total),
            ("impl", "worker", pi_total),
            (superseded_id, "lead", pi_total),
        ]
    );
    assert!(first.sessions.iter().all(|session| session.live.is_none()));
    let mut events = first
        .events
        .iter()
        .map(|event| (event.at, event.id.as_str(), event.kind))
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.0);
    assert_eq!(
        events,
        [
            (utc("2026-10-01T07:00:00Z"), "impl", EventKind::Spawned),
            (
                utc("2026-10-01T12:00:00Z"),
                "impl",
                EventKind::Reported(WakeKind::Done)
            ),
            (utc("2026-10-01T17:00:00Z"), "impl", EventKind::Closed),
        ]
    );
    for (dir, cached) in [
        (&closed, true),
        (&superseded, true),
        (&latest, false),
        (&unlinked, false),
        (&stale, false),
    ] {
        assert_eq!(dir.join("usage.json").exists(), cached, "{dir}");
    }

    fs::remove_dir_all(&transcripts).unwrap();
    let second = project(&entry, true, since).unwrap();
    assert_eq!(
        totals(&second),
        [
            (latest_id, "lead", 0),
            ("impl", "worker", pi_total),
            (superseded_id, "lead", pi_total),
        ]
    );
    // A running lead is live, and the roster names no window for its model.
    let live = second.sessions[0].live.as_ref().unwrap();
    assert_eq!((live.prompt_tokens, live.context_window), (None, None));
    assert!(!latest.join("usage.json").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_roster_window_is_for_the_launched_model() {
    let models = crate::agents::ModelRoster::builtin().unwrap();
    assert_eq!(
        collect::roster_window("claude:opus:high", &models).unwrap(),
        Some(1_000_000)
    );
    assert_eq!(collect::roster_window("claude", &models).unwrap(), None);
}

fn session(id: &str, role: &'static str, agent: &str, usage: Usage, live: bool) -> Session {
    Session {
        id: id.into(),
        role,
        agent: agent.into(),
        live: live.then_some(Live {
            prompt_tokens: usage.prompt_tokens,
            context_window: usage.context_window,
        }),
        buckets: usage.buckets,
    }
}

fn projects() -> Vec<Project> {
    let mut claude = session(
        "lead-1",
        "lead",
        "claude:opus:high",
        fixtures::claude(),
        true,
    );
    claude.live.as_mut().unwrap().context_window = Some(1_000_000);
    let mut api = vec![
        claude,
        session(
            "impl",
            "worker",
            "codex:gpt-5.5:high",
            fixtures::codex(),
            true,
        ),
    ];
    for index in 0..4 {
        api.push(session(
            &format!("pi-{index}"),
            "worker",
            "pi:tencent/hy3:high",
            fixtures::pi(),
            true,
        ));
    }
    let events = (0..10)
        .map(|hour| Event {
            at: utc("2026-10-01T00:00:00Z") + TimeDelta::hours(hour),
            id: format!("e{hour}"),
            kind: match hour % 3 {
                0 => EventKind::Spawned,
                1 => EventKind::Reported(WakeKind::Done),
                _ => EventKind::Closed,
            },
        })
        .chain([Event {
            at: utc("2026-01-01T00:00:00Z"),
            id: "before".into(),
            kind: EventKind::Spawned,
        }])
        .collect();
    vec![
        Project {
            name: "api".into(),
            sessions: api,
            events,
        },
        Project {
            name: "web".into(),
            sessions: vec![
                session(
                    "rev",
                    "reviewer",
                    "pi:tencent/hy3:high",
                    fixtures::pi(),
                    false,
                ),
                session(
                    "herm",
                    "worker",
                    "hermes:tencent/hy3",
                    fixtures::hermes(),
                    true,
                ),
            ],
            events: Vec::new(),
        },
        Project {
            name: "idle".into(),
            sessions: Vec::new(),
            events: Vec::new(),
        },
    ]
}

#[test]
fn dashboard_totals_projects_models_chart_context_and_events() {
    let mut projects = projects();
    let window = Window::new(Range::Month, local(2, 12)).unwrap();
    let board = dashboard(&projects, window);
    let [claude, codex, pi, hermes] = [
        fixtures::claude(),
        fixtures::codex(),
        fixtures::pi(),
        fixtures::hermes(),
    ]
    .map(|usage| usage.total_tokens());
    let api = claude + codex + 4 * pi;
    let web = pi + hermes;
    assert_eq!(board.projects, [("api", api), ("web", web), ("idle", 0)]);
    assert_eq!(board.bins.iter().sum::<u64>(), api + web);
    assert_eq!((board.sessions, board.spawns, board.wakes), (8, 4, 3));
    let models = board
        .models
        .iter()
        .map(|row| (row.agent, row.roles.clone(), row.tokens))
        .collect::<Vec<_>>();
    let mut expected = vec![
        (
            "pi:tencent/hy3:high",
            vec![("worker", 4), ("reviewer", 1)],
            5 * pi,
        ),
        ("claude:opus:high", vec![("lead", 1)], claude),
        ("codex:gpt-5.5:high", vec![("worker", 1)], codex),
        ("hermes:tencent/hy3", vec![("worker", 1)], hermes),
    ];
    expected.sort_by_key(|row| std::cmp::Reverse(row.2));
    assert_eq!(models, expected);
    let context = board
        .context
        .iter()
        .map(|row| (row.session.id.as_str(), row.percent))
        .collect::<Vec<_>>();
    assert_eq!(
        context,
        [
            ("impl", Some(95)),
            ("lead-1", Some(0)),
            ("pi-0", None),
            ("pi-1", None),
            ("pi-2", None),
            ("pi-3", None),
        ]
    );
    let events = board
        .events
        .iter()
        .map(|(_, event)| event.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(events, ["e9", "e8", "e7", "e6", "e5", "e4", "e3", "e2"]);
    projects[0].sessions[1]
        .live
        .as_mut()
        .unwrap()
        .context_window = Some(0);
    let zero_window = dashboard(&projects, window);
    assert_eq!(
        zero_window
            .context
            .iter()
            .find(|row| row.session.id == "impl")
            .unwrap()
            .percent,
        None
    );
}

#[test]
fn render_lays_out_pairs_by_width_and_warns_on_full_context() {
    let theme = Theme::parse(None).unwrap();
    let projects = projects();
    let board = dashboard(&projects, Window::new(Range::Month, local(2, 12)).unwrap());
    let narrow = render::dashboard(&board, &theme, 80);
    let wide = render::dashboard(&board, &theme, 120);
    let together = |text: &str, left: &str, right: &str| {
        text.lines()
            .any(|line| line.contains(left) && line.contains(right))
    };
    for (left, right) in [("TOKENS BY PROJECT", "BY MODEL"), ("CONTEXT", "EVENTS")] {
        assert!(!together(&narrow, left, right));
        assert!(together(&wide, left, right));
    }
    let warn = theme.style(StyleKey::Waiting).paint("|");
    let warn = warn.split('|').next().unwrap();
    for text in [&narrow, &wide] {
        assert!(text.contains("4 workers · 1 reviewer"));
        assert!(
            text.lines()
                .any(|line| line.contains("95%") && line.contains(warn))
        );
        assert!(
            !text
                .lines()
                .any(|line| line.contains("0%") && line.contains(warn))
        );
        assert!(!text.contains("herm "));
    }
}

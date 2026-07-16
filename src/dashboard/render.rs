use super::{
    progress,
    snapshot::{DashboardRole, DashboardRow, DashboardSnapshot, ManagerLifecycle, PaneSummary},
    status::WakeState,
};

#[derive(Clone, Copy)]
enum ColumnId {
    Window,
    Role,
    Agent,
    Pane,
    Wall,
    Usage,
    Wake,
    LastStatus,
}

struct Column {
    title: &'static str,
    width: usize,
    id: ColumnId,
}

const COLUMNS: &[Column] = &[
    Column {
        title: "window",
        width: 28,
        id: ColumnId::Window,
    },
    Column {
        title: "role",
        width: 22,
        id: ColumnId::Role,
    },
    Column {
        title: "agent",
        width: 24,
        id: ColumnId::Agent,
    },
    Column {
        title: "pane",
        width: 32,
        id: ColumnId::Pane,
    },
    Column {
        title: "wall",
        width: 8,
        id: ColumnId::Wall,
    },
    Column {
        title: "usage",
        width: 18,
        id: ColumnId::Usage,
    },
    Column {
        title: "wake",
        width: 13,
        id: ColumnId::Wake,
    },
    Column {
        title: "last status",
        width: 76,
        id: ColumnId::LastStatus,
    },
];

pub(crate) fn render(snapshot: &DashboardSnapshot) -> String {
    let mut output = String::new();
    output.push_str("Niles home\n\n");
    if let Some(progress) = &snapshot.progress {
        output.push_str(&progress::progress_line(progress));
        output.push_str("\n\n");
    }
    render_header(&mut output);
    render_separator(&mut output);
    for row in &snapshot.rows {
        render_row(&mut output, row);
    }
    output.push('\n');
    render_footer(&mut output, snapshot);
    output
}

fn render_header(output: &mut String) {
    for column in COLUMNS {
        output.push_str(&fit_cell(column.title, column.width));
        output.push(' ');
    }
    output.push('\n');
}

fn render_separator(output: &mut String) {
    for column in COLUMNS {
        output.push_str(&"-".repeat(column.width));
        output.push(' ');
    }
    output.push('\n');
}

fn render_row(output: &mut String, row: &DashboardRow) {
    for column in COLUMNS {
        output.push_str(&fit_cell(&cell(row, column.id), column.width));
        output.push(' ');
    }
    output.push('\n');
}

fn render_footer(output: &mut String, snapshot: &DashboardSnapshot) {
    output.push_str(match snapshot.manager_lifecycle {
        ManagerLifecycle::Live => "manager live",
        ManagerLifecycle::Exited => "manager exited",
        ManagerLifecycle::Unknown => "manager unknown",
    });
    output.push('\n');

    if let Some(target) = &snapshot.manager_target {
        output.push_str("switch: tmux switch-client -t ");
        output.push_str(target);
        output.push('\n');
    } else {
        output.push_str("switch: n/a\n");
    }
    output.push_str("fallback: niles -d\n");
    if snapshot.manager_lifecycle == ManagerLifecycle::Exited {
        output.push_str("respawn: close stale niles-manager if present, then run bare niles\n");
    }
    for message in &snapshot.messages {
        output.push_str("note: ");
        output.push_str(message);
        output.push('\n');
    }
}

fn cell(row: &DashboardRow, column: ColumnId) -> String {
    match column {
        ColumnId::Window => row.window.clone(),
        ColumnId::Role => role_label(row.role).to_owned(),
        ColumnId::Agent => row.agent.clone(),
        ColumnId::Pane => pane_cell(&row.pane),
        ColumnId::Wall => row.wall.clone(),
        ColumnId::Usage => row.usage.render(),
        ColumnId::Wake => wake_label(row.wake).to_owned(),
        ColumnId::LastStatus => row.last_status.clone(),
    }
}

fn role_label(role: DashboardRole) -> &'static str {
    match role {
        DashboardRole::Home => "home",
        DashboardRole::Manager => "manager",
        DashboardRole::Worker => "worker",
        DashboardRole::RunStep => "run-step",
        DashboardRole::UnknownNilesWindow => "unknown-niles-window",
    }
}

fn wake_label(state: WakeState) -> &'static str {
    match state {
        WakeState::Pending => "pending",
        WakeState::Consumed => "consumed",
        WakeState::NoActionable => "no-actionable",
        WakeState::AckInvalid => "ack-invalid",
        WakeState::NotApplicable => "n-a",
    }
}

fn pane_cell(pane: &PaneSummary) -> String {
    if let Some(state) = &pane.state {
        return state.clone();
    }
    let mut value = pane_liveness_label(pane.liveness).to_owned();
    if let Some(pid) = pane.pid {
        value.push_str(" pid=");
        value.push_str(&pid.to_string());
    }
    if let Some(command) = &pane.command {
        value.push_str(" cmd=");
        value.push_str(command);
    }
    value
}

fn pane_liveness_label(liveness: super::snapshot::PaneLiveness) -> &'static str {
    match liveness {
        super::snapshot::PaneLiveness::Live => "live",
        super::snapshot::PaneLiveness::Dead => "dead",
        super::snapshot::PaneLiveness::Missing => "missing",
        super::snapshot::PaneLiveness::Unknown => "unknown",
    }
}

fn fit_cell(value: &str, width: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= width {
        return format!("{value:<width$}");
    }

    if width == 0 {
        return String::new();
    }

    let kept = width.saturating_sub(1);
    let mut truncated = value.chars().take(kept).collect::<String>();
    truncated.push('~');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::{
        snapshot::{DashboardRole, ManagerLifecycle, PaneLiveness},
        usage_cell::UsageCell,
    };

    #[test]
    fn render_includes_manager_lifecycle_and_fallbacks() {
        let snapshot = DashboardSnapshot {
            rows: vec![DashboardRow {
                window: "niles-manager".to_owned(),
                role: DashboardRole::Manager,
                subject: "session".to_owned(),
                agent: "codex".to_owned(),
                pane: PaneSummary {
                    liveness: PaneLiveness::Dead,
                    pid: Some(42),
                    command: Some("codex".to_owned()),
                    state: None,
                },
                wall: "1m".to_owned(),
                usage: UsageCell::unavailable(),
                wake: WakeState::NotApplicable,
                last_status: "-".to_owned(),
            }],
            messages: Vec::new(),
            manager_target: Some("niles:niles-manager".to_owned()),
            manager_lifecycle: ManagerLifecycle::Exited,
            progress: None,
        };

        let output = render(&snapshot);

        assert!(output.contains("manager exited"));
        assert!(output.contains("tmux switch-client -t niles:niles-manager"));
        assert!(output.contains("fallback: niles -d"));
        assert!(output.contains("respawn: close stale niles-manager"));
    }

    #[test]
    fn render_includes_wake_column_tokens() {
        let rows = [
            wake_row("pending-row", WakeState::Pending),
            wake_row("consumed-row", WakeState::Consumed),
            wake_row("no-actionable-row", WakeState::NoActionable),
            wake_row("ack-invalid-row", WakeState::AckInvalid),
            wake_row("not-applicable-row", WakeState::NotApplicable),
        ];
        let snapshot = DashboardSnapshot {
            rows: rows.to_vec(),
            messages: Vec::new(),
            manager_target: None,
            manager_lifecycle: ManagerLifecycle::Unknown,
            progress: None,
        };

        let output = render(&snapshot);

        assert!(output.contains("wake"));
        assert!(output.contains("pending"));
        assert!(output.contains("consumed"));
        assert!(output.contains("no-actionable"));
        assert!(output.contains("ack-invalid"));
        assert!(output.contains("n-a"));
    }

    fn wake_row(window: &str, wake: WakeState) -> DashboardRow {
        DashboardRow {
            window: window.to_owned(),
            role: DashboardRole::Worker,
            subject: "subject".to_owned(),
            agent: "codex".to_owned(),
            pane: PaneSummary {
                liveness: PaneLiveness::Live,
                pid: None,
                command: None,
                state: None,
            },
            wall: "1m".to_owned(),
            usage: UsageCell::pending(),
            wake,
            last_status: "done: ready".to_owned(),
        }
    }
}

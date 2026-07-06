use anyhow::{Context, Result, bail};

const LIST_WINDOWS_FORMAT: &str = "#{window_index}\t#{window_name}\t#{window_active}\t#{pane_pid}\t#{pane_current_command}\t#{pane_dead}";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TmuxWindowSnapshot {
    pub(crate) index: usize,
    pub(crate) name: String,
    pub(crate) active: bool,
    pub(crate) pane_pid: Option<u32>,
    pub(crate) pane_current_command: Option<String>,
    pub(crate) pane_dead: bool,
}

pub(crate) fn format() -> &'static str {
    LIST_WINDOWS_FORMAT
}

pub(crate) fn parse(stdout: &[u8]) -> Result<Vec<TmuxWindowSnapshot>> {
    let text = String::from_utf8_lossy(stdout);
    let mut snapshots = Vec::new();

    for (line_number, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        snapshots.push(parse_line(line_number + 1, line)?);
    }

    Ok(snapshots)
}

fn parse_line(line_number: usize, line: &str) -> Result<TmuxWindowSnapshot> {
    let fields = line.split('\t').collect::<Vec<_>>();
    if fields.len() != 6 {
        bail!(
            "malformed tmux list-windows line {line_number}: expected 6 fields, got {}",
            fields.len()
        );
    }

    Ok(TmuxWindowSnapshot {
        index: parse_usize_field(fields[0], "window index", line_number)?,
        name: fields[1].to_owned(),
        active: parse_tmux_bool(fields[2], "window active", line_number)?,
        pane_pid: parse_optional_pid(fields[3], line_number)?,
        pane_current_command: nonempty_string(fields[4]),
        pane_dead: parse_tmux_bool(fields[5], "pane dead", line_number)?,
    })
}

fn parse_usize_field(value: &str, label: &str, line_number: usize) -> Result<usize> {
    value.parse::<usize>().with_context(|| {
        format!("invalid {label} `{value}` on tmux list-windows line {line_number}")
    })
}

fn parse_optional_pid(value: &str, line_number: usize) -> Result<Option<u32>> {
    if value.is_empty() {
        return Ok(None);
    }

    Ok(Some(value.parse::<u32>().with_context(|| {
        format!("invalid pane pid `{value}` on tmux list-windows line {line_number}")
    })?))
}

fn parse_tmux_bool(value: &str, label: &str, line_number: usize) -> Result<bool> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => bail!("invalid {label} `{value}` on tmux list-windows line {line_number}"),
    }
}

fn nonempty_string(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typed_tmux_fields() {
        let output = b"0\tniles\t1\t12345\tzsh\t0\n1\tniles-manager\t0\t23456\tclaude\t1\n";

        let snapshots = parse(output).unwrap();

        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].name, "niles");
        assert!(snapshots[0].active);
        assert_eq!(snapshots[0].pane_pid, Some(12345));
        assert_eq!(snapshots[0].pane_current_command.as_deref(), Some("zsh"));
        assert!(!snapshots[0].pane_dead);
        assert_eq!(snapshots[1].name, "niles-manager");
        assert!(snapshots[1].pane_dead);
    }

    #[test]
    fn rejects_malformed_field_count() {
        let err = parse(b"0\tniles\t1\n").unwrap_err();

        assert!(err.to_string().contains("expected 6 fields"));
    }

    #[test]
    fn rejects_malformed_typed_fields() {
        let err = parse(b"0\tniles\tmaybe\t123\tzsh\t0\n").unwrap_err();

        assert!(err.to_string().contains("window active"));
    }
}

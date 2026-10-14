use crate::tmux::{self, TmuxTarget};
use anyhow::{Result, bail};
use chrono::{DateTime, TimeDelta, Utc};
use std::{
    collections::HashMap,
    process::{Command, Stdio},
};

const START_TIME_TOLERANCE: TimeDelta = TimeDelta::seconds(1);

pub(crate) fn process_activity(
    target: &TmuxTarget,
    since: Option<DateTime<Utc>>,
) -> Result<Option<bool>> {
    let alive = tmux::pane_alive(target.as_str())?;
    let Some(since) = since.filter(|_| alive == Some(true)) else {
        return Ok(alive);
    };
    let root = tmux::pane_pid(target)?;
    // etime rounds to seconds; the tolerance favors counting work near a turn boundary.
    // POSIX ps supplies PPIDs and elapsed times, not a descendant query; std::process
    // tracks only our children. https://pubs.opengroup.org/onlinepubs/9799919799/utilities/ps.html
    let probe = Command::new("ps")
        .env("LC_ALL", "C")
        .args(["-ax", "-o", "pid=,ppid=,stat=,etime=,comm="])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let probe_pid = probe.id();
    let output = probe.wait_with_output()?;
    if !output.status.success() {
        bail!("ps failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(Some(has_work(
        root,
        [std::process::id(), probe_pid],
        Utc::now() - since,
        &String::from_utf8(output.stdout)?,
    )?))
}

fn has_work(root: u32, excluded: [u32; 2], turn_age: TimeDelta, snapshot: &str) -> Result<bool> {
    let mut children = HashMap::<u32, Vec<(u32, i64, bool)>>::new();
    for line in snapshot.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 5 {
            bail!("invalid ps row: {line}");
        }
        let pid: u32 = fields[0].parse()?;
        let parent = fields[1].parse()?;
        if fields[2].starts_with('Z') || excluded.contains(&pid) {
            continue;
        }
        let (days, clock) = match fields[3].split_once('-') {
            Some((days, clock)) => (days.parse::<i64>()?, clock),
            None => (0, fields[3]),
        };
        let elapsed = clock.split(':').try_fold(0, |total, part| {
            Ok::<_, anyhow::Error>(total * 60 + part.parse::<i64>()?)
        })? + days * 86_400;
        // Unix shell executable names conventionally end in sh; comm excludes arguments.
        let shell = fields[4..].join(" ").ends_with("sh");
        children
            .entry(parent)
            .or_default()
            .push((pid, elapsed, shell));
    }
    let mut pending = vec![(root, false)];
    while let Some((parent, under_shell)) = pending.pop() {
        if let Some(branch) = children.remove(&parent) {
            for (pid, elapsed, shell) in branch {
                let under_shell = under_shell || shell;
                if under_shell && elapsed <= (turn_age + START_TIME_TOLERANCE).num_seconds() {
                    return Ok(true);
                }
                pending.push((pid, under_shell));
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descendants_use_elapsed_time_and_current_liveness_not_row_order() {
        let standing = "3 2 S 02:00 helper\n2 1 S 01:00:00 agent\n4 1 Z 00:01 sh\n7 6 S 00:01 sh\n";
        assert!(!has_work(1, [80, 81], TimeDelta::seconds(59), standing).unwrap());
        assert!(
            has_work(
                1,
                [80, 81],
                TimeDelta::seconds(59),
                &format!("5 3 S 00:39 /bin/zsh\n{standing}")
            )
            .unwrap()
        );
        assert!(
            !has_work(
                1,
                [80, 81],
                TimeDelta::seconds(59),
                &format!("5 3 Z 00:39 /bin/zsh\n{standing}")
            )
            .unwrap()
        );
        assert!(!has_work(1, [80, 81], TimeDelta::seconds(59), "5 1 S 2-00:00:39 sh\n").unwrap());
        assert!(has_work(1, [80, 81], TimeDelta::seconds(59), "5 1 S 01:00 sh\n").unwrap());
        assert!(!has_work(1, [80, 81], TimeDelta::seconds(59), "5 1 S 01:01 sh\n").unwrap());
        assert!(has_work(1, [80, 81], TimeDelta::seconds(59), "broken\n").is_err());
        assert!(has_work(1, [80, 81], TimeDelta::seconds(59), "5 1 S nope sh\n").is_err());
    }

    #[test]
    fn querying_niles_branch_and_probe_are_excluded_but_root_children_are_not() {
        let inspection = "81 80 S 00:00 ps\n82 80 S 00:00 sh\n80 1 S 00:01 niles\n";
        assert!(!has_work(1, [80, 81], TimeDelta::seconds(59), inspection).unwrap());
        assert!(!has_work(80, [80, 81], TimeDelta::seconds(59), "81 80 S 00:00 ps\n").unwrap());
        assert!(has_work(80, [80, 81], TimeDelta::seconds(59), inspection).unwrap());
        assert!(
            has_work(
                1,
                [80, 81],
                TimeDelta::seconds(59),
                &format!("3 1 S 00:01 sh\n{inspection}")
            )
            .unwrap()
        );
    }

    #[test]
    fn lazy_helpers_from_the_reported_codex_tree_are_not_work() {
        let snapshot = "84938 1 S 19:55 sh\n84951 84938 S 19:55 node\n\
                        85064 84951 S 19:55 codex\n87278 85064 S 19:48 codex-code-mode-host\n";
        assert!(!has_work(84938, [80, 81], TimeDelta::seconds(1195), snapshot).unwrap());
        let tree = format!("{snapshot}87279 87278 S 00:01 helper\n");
        assert!(!has_work(84938, [80, 81], TimeDelta::seconds(1195), &tree).unwrap());
    }

    #[test]
    fn shell_commands_and_their_live_descendants_are_work() {
        for shell in ["/bin/zsh", "/bin/bash", "/bin/sh", "/usr/bin/fish"] {
            let tree = format!("2 1 S 20:00 agent\n3 2 S 19:48 helper\n4 3 S 00:39 {shell}\n");
            assert!(has_work(1, [80, 81], TimeDelta::seconds(59), &tree).unwrap());
            let tree = format!("2 1 S 20:00 agent\n3 2 S 02:00 {shell}\n4 3 S 00:39 build\n");
            assert!(has_work(1, [80, 81], TimeDelta::seconds(59), &tree).unwrap());
            let tree = format!("2 1 S 20:00 agent\n3 2 S 02:00 {shell}\n4 3 Z 00:39 build\n");
            assert!(!has_work(1, [80, 81], TimeDelta::seconds(59), &tree).unwrap());
        }
    }
}

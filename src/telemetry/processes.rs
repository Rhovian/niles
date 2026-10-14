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
        .args(["-ax", "-o", "pid=,ppid=,stat=,etime="])
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
    let mut children = HashMap::<u32, Vec<(u32, i64)>>::new();
    for line in snapshot.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 4 {
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
        children.entry(parent).or_default().push((pid, elapsed));
    }
    let mut pending = vec![root];
    while let Some(parent) = pending.pop() {
        if let Some(branch) = children.remove(&parent) {
            for (pid, elapsed) in branch {
                if elapsed <= (turn_age + START_TIME_TOLERANCE).num_seconds() {
                    return Ok(true);
                }
                pending.push(pid);
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
        let standing = "3 2 S 02:00\n2 1 S 01:00:00\n4 1 Z 00:01\n7 6 S 00:01\n";
        assert!(!has_work(1, [80, 81], TimeDelta::seconds(59), standing).unwrap());
        assert!(
            has_work(
                1,
                [80, 81],
                TimeDelta::seconds(59),
                &format!("5 3 S 00:39\n{standing}")
            )
            .unwrap()
        );
        assert!(
            !has_work(
                1,
                [80, 81],
                TimeDelta::seconds(59),
                &format!("5 3 Z 00:39\n{standing}")
            )
            .unwrap()
        );
        assert!(!has_work(1, [80, 81], TimeDelta::seconds(59), "5 1 S 2-00:00:39\n").unwrap());
        assert!(has_work(1, [80, 81], TimeDelta::seconds(59), "5 1 S 01:00\n").unwrap());
        assert!(!has_work(1, [80, 81], TimeDelta::seconds(59), "5 1 S 01:01\n").unwrap());
        assert!(has_work(1, [80, 81], TimeDelta::seconds(59), "broken\n").is_err());
        assert!(has_work(1, [80, 81], TimeDelta::seconds(59), "5 1 S nope\n").is_err());
    }

    #[test]
    fn querying_niles_branch_and_probe_are_excluded_but_root_children_are_not() {
        let inspection = "81 80 S 00:00\n82 80 S 00:00\n80 1 S 00:01\n";
        assert!(!has_work(1, [80, 81], TimeDelta::seconds(59), inspection).unwrap());
        assert!(!has_work(80, [80, 81], TimeDelta::seconds(59), "81 80 S 00:00\n").unwrap());
        assert!(has_work(80, [80, 81], TimeDelta::seconds(59), inspection).unwrap());
        assert!(
            has_work(
                1,
                [80, 81],
                TimeDelta::seconds(59),
                &format!("3 1 S 00:01\n{inspection}")
            )
            .unwrap()
        );
    }
}

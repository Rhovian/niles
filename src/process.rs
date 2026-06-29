use std::{
    fs::{self, File},
    io::{self, Read, Write},
    process::{Command, Stdio},
    thread,
};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::Utc;

use crate::state::{StepKind, StepRecord, StepStatus};

type ReaderHandle = thread::JoinHandle<Result<()>>;

pub fn run_process(
    step_number: usize,
    role: Option<String>,
    kind: StepKind,
    label: &str,
    binary: &str,
    args: &[String],
    stdin: Option<&str>,
    workspace: &Utf8Path,
    steps_dir: &Utf8Path,
) -> Result<StepRecord> {
    let started_at = Utc::now();
    let slug = slugify(label);
    let prefix = format!("{step_number:03}-{slug}");
    let stdout_path = steps_dir.join(format!("{prefix}.stdout.txt"));
    let stderr_path = steps_dir.join(format!("{prefix}.stderr.txt"));
    let diff_path = steps_dir.join(format!("{prefix}.diff"));
    let meta_path = steps_dir.join(format!("{prefix}.json"));

    let mut child = Command::new(binary)
        .args(args)
        .current_dir(workspace)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn `{}`", format_invocation(binary, args)))?;

    let stdout = child.stdout.take().context("failed to open child stdout")?;
    let stderr = child.stderr.take().context("failed to open child stderr")?;
    let stdout_handle = spawn_tee(stdout, stdout_path.clone(), StreamTarget::Stdout);
    let stderr_handle = spawn_tee(stderr, stderr_path.clone(), StreamTarget::Stderr);

    if let Some(input) = stdin {
        let mut child_stdin = child.stdin.take().context("failed to open child stdin")?;
        child_stdin
            .write_all(input.as_bytes())
            .context("failed to write child stdin")?;
    }

    let status = child
        .wait()
        .with_context(|| format!("failed to wait for `{}`", format_invocation(binary, args)))?;
    let finished_at = Utc::now();

    stdout_handle
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader thread panicked"))??;
    stderr_handle
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader thread panicked"))??;
    capture_git_diff(workspace, &diff_path)?;

    let record = StepRecord {
        index: step_number,
        role,
        kind,
        label: label.to_owned(),
        status: if status.success() {
            StepStatus::Completed
        } else {
            StepStatus::Failed
        },
        started_at: Some(started_at),
        finished_at: Some(finished_at),
        exit_code: status.code(),
        stdout: Some(stdout_path),
        stderr: Some(stderr_path),
        diff: Some(diff_path),
    };

    fs::write(&meta_path, serde_json::to_string_pretty(&record)?)
        .with_context(|| format!("failed to write {meta_path}"))?;

    Ok(record)
}

fn capture_git_diff(workspace: &Utf8Path, diff_path: &Utf8Path) -> Result<()> {
    let output = Command::new("git")
        .args(["diff", "--no-ext-diff", "--"])
        .current_dir(workspace)
        .stdin(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() => {
            fs::write(diff_path, output.stdout)
                .with_context(|| format!("failed to write {diff_path}"))?;
        }
        Ok(output) => {
            fs::write(diff_path, Vec::<u8>::new())
                .with_context(|| format!("failed to write {diff_path}"))?;
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                eprintln!("warning: git diff failed: {}", stderr.trim());
            }
        }
        Err(err) => {
            fs::write(diff_path, Vec::<u8>::new())
                .with_context(|| format!("failed to write {diff_path}"))?;
            eprintln!("warning: git diff failed: {err}");
        }
    }

    Ok(())
}

enum StreamTarget {
    Stdout,
    Stderr,
}

fn spawn_tee<R>(mut reader: R, log_path: camino::Utf8PathBuf, target: StreamTarget) -> ReaderHandle
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut log = File::create(&log_path)
            .with_context(|| format!("failed to create stream log {log_path}"))?;
        let mut buffer = [0_u8; 8192];

        loop {
            let read = reader
                .read(&mut buffer)
                .with_context(|| format!("failed to read stream for {log_path}"))?;
            if read == 0 {
                break;
            }

            log.write_all(&buffer[..read])
                .with_context(|| format!("failed to write stream log {log_path}"))?;
            match target {
                StreamTarget::Stdout => {
                    let mut stdout = io::stdout().lock();
                    stdout
                        .write_all(&buffer[..read])
                        .context("failed to write child stdout")?;
                    stdout.flush().context("failed to flush child stdout")?;
                }
                StreamTarget::Stderr => {
                    let mut stderr = io::stderr().lock();
                    stderr
                        .write_all(&buffer[..read])
                        .context("failed to write child stderr")?;
                    stderr.flush().context("failed to flush child stderr")?;
                }
            }
        }

        log.flush()
            .with_context(|| format!("failed to flush stream log {log_path}"))?;
        Ok(())
    })
}

fn slugify(value: &str) -> String {
    let mut slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();

    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }

    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "step".to_owned()
    } else {
        slug.to_owned()
    }
}

fn format_invocation(binary: &str, args: &[String]) -> String {
    std::iter::once(binary)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    process::{Command, Stdio},
    thread,
};

use anyhow::{Context, Result};
use camino::Utf8Path;
use chrono::Utc;

use crate::{
    state::{StepKind, StepRecord, StepStatus},
    util::{slugify, write_json_pretty},
};

type ReaderHandle = thread::JoinHandle<Result<()>>;

pub struct ProcessSpec<'a> {
    pub step_number: usize,
    pub role: Option<String>,
    pub kind: StepKind,
    pub label: &'a str,
    pub binary: &'a str,
    pub args: &'a [String],
    pub stdin: Option<&'a str>,
    pub workspace: &'a Utf8Path,
    pub steps_dir: &'a Utf8Path,
    pub context_path: Option<camino::Utf8PathBuf>,
}

pub fn run_process(spec: ProcessSpec<'_>) -> Result<StepRecord> {
    let started_at = Utc::now();
    let slug = slugify(spec.label);
    let prefix = format!("{:03}-{slug}", spec.step_number);
    let stdout_path = spec.steps_dir.join(format!("{prefix}.stdout.txt"));
    let stderr_path = spec.steps_dir.join(format!("{prefix}.stderr.txt"));
    let diff_path = spec.steps_dir.join(format!("{prefix}.diff"));
    let meta_path = spec.steps_dir.join(format!("{prefix}.json"));

    let mut child = Command::new(spec.binary)
        .args(spec.args)
        .current_dir(spec.workspace)
        .stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "failed to spawn `{}`",
                format_invocation(spec.binary, spec.args)
            )
        })?;

    let stdout = child.stdout.take().context("failed to open child stdout")?;
    let stderr = child.stderr.take().context("failed to open child stderr")?;
    let stdout_handle = spawn_tee(stdout, stdout_path.clone(), StreamTarget::Stdout);
    let stderr_handle = spawn_tee(stderr, stderr_path.clone(), StreamTarget::Stderr);

    if let Some(input) = spec.stdin {
        let mut child_stdin = child.stdin.take().context("failed to open child stdin")?;
        child_stdin
            .write_all(input.as_bytes())
            .context("failed to write child stdin")?;
    }

    let status = child.wait().with_context(|| {
        format!(
            "failed to wait for `{}`",
            format_invocation(spec.binary, spec.args)
        )
    })?;
    let finished_at = Utc::now();

    stdout_handle
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader thread panicked"))??;
    stderr_handle
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader thread panicked"))??;
    capture_git_diff(spec.workspace, &diff_path)?;

    let record = StepRecord {
        index: spec.step_number,
        role: spec.role,
        kind: spec.kind,
        label: spec.label.to_owned(),
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
        context: spec.context_path,
        window: None,
    };

    write_json_pretty(&meta_path, &record)?;

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
            write_empty_diff(diff_path)?;
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                eprintln!("warning: git diff failed: {}", stderr.trim());
            }
        }
        Err(err) => {
            write_empty_diff(diff_path)?;
            eprintln!("warning: git diff failed: {err}");
        }
    }

    Ok(())
}

fn write_empty_diff(diff_path: &Utf8Path) -> Result<()> {
    fs::write(diff_path, Vec::<u8>::new()).with_context(|| format!("failed to write {diff_path}"))
}

enum StreamTarget {
    Stdout,
    Stderr,
}

impl StreamTarget {
    fn write(&self, bytes: &[u8]) -> Result<()> {
        match self {
            StreamTarget::Stdout => write_child_stream(io::stdout().lock(), bytes, "stdout"),
            StreamTarget::Stderr => write_child_stream(io::stderr().lock(), bytes, "stderr"),
        }
    }
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
            target.write(&buffer[..read])?;
        }

        log.flush()
            .with_context(|| format!("failed to flush stream log {log_path}"))?;
        Ok(())
    })
}

fn write_child_stream<W>(mut writer: W, bytes: &[u8], label: &str) -> Result<()>
where
    W: Write,
{
    writer
        .write_all(bytes)
        .with_context(|| format!("failed to write child {label}"))?;
    writer
        .flush()
        .with_context(|| format!("failed to flush child {label}"))
}

fn format_invocation(binary: &str, args: &[String]) -> String {
    std::iter::once(binary)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

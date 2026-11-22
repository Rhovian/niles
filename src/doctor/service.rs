use super::version_line;
use anyhow::{Context, Result, ensure};
use camino::Utf8PathBuf;
use std::fs::{self, Metadata};
use std::num::NonZeroU32;
use std::os::unix::fs::MetadataExt;
use std::process::{Command, Output};
use std::str;

struct ConfiguredService {
    program: Utf8PathBuf,
    pid: Option<NonZeroU32>,
}

impl ConfiguredService {
    fn new(program: Utf8PathBuf, pid: Option<NonZeroU32>) -> Result<Self> {
        ensure!(
            program.is_absolute(),
            "service program must use an absolute path"
        );
        Ok(Self { program, pid })
    }
}

pub(super) fn print_status() -> Result<()> {
    let Some(ConfiguredService { program, pid }) = configured()? else {
        println!("remote service: not installed");
        return Ok(());
    };
    println!("remote service program: {program}");
    let binary = fs::metadata(&program)
        .with_context(|| format!("could not inspect service program {program}"))?;
    let matches = pid
        .map(|pid| uses_binary(pid, &binary))
        .transpose()
        .context("could not compare running remote service to its configured program")?;
    let status = match matches {
        Some(true) => "running binary matches",
        Some(false) => "running binary differs",
        None => "stopped",
    };
    println!("remote service: installed, {status}");
    let version = version_line(&program, "--version");
    let features = version
        .strip_prefix("niles ")
        .and_then(|line| line.rsplit_once(", features: "))
        .and_then(|(_, value)| value.strip_suffix(')'));
    if version.starts_with("version unavailable") {
        println!("remote {version}");
    }
    let status = match features {
        Some("remote") => "enabled",
        Some("none") => "disabled",
        Some(_) | None => "unknown",
    };
    println!("remote: {status}");
    if features == Some("none") {
        println!("remote service repair: reinstall from source with --features remote");
    }
    if matches == Some(false) {
        println!("remote service repair: restart the service");
    }
    Ok(())
}

fn checked_stdout<'a>(output: &'a Output, command: &str) -> Result<&'a str> {
    ensure!(
        output.status.success(),
        "{command} exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    str::from_utf8(&output.stdout).with_context(|| format!("{command} output is not UTF-8"))
}

#[cfg(target_os = "linux")]
fn configured() -> Result<Option<ConfiguredService>> {
    // systemctl show exposes unit state and command properties for scripts:
    // https://www.freedesktop.org/software/systemd/man/254/systemctl.html#show%20PATTERN%E2%80%A6%7CJOB%E2%80%A6
    let output = Command::new("systemctl")
        .args([
            "--user",
            "show",
            "--property=LoadState,MainPID,ExecStart",
            "niles-remote.service",
        ])
        .output()
        .context("could not run systemctl --user")?;
    let properties = checked_stdout(&output, "systemctl --user show")?;
    let property = |key| {
        properties
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once('=')?;
                (name == key).then_some(value)
            })
            .with_context(|| format!("systemctl omitted {key}"))
    };
    if property("LoadState")? == "not-found" {
        return Ok(None);
    }

    let pid = property("MainPID")?
        .parse()
        .context("invalid systemctl MainPID")?;
    let exec = property("ExecStart")?;
    let (program, _) = exec
        .strip_prefix("{ path=")
        .and_then(|exec| exec.split_once(" ; argv[]="))
        .context("systemctl did not report an ExecStart program")?;
    ConfiguredService::new(program.into(), NonZeroU32::new(pid)).map(Some)
}

#[cfg(target_os = "linux")]
fn uses_binary(pid: NonZeroU32, installed: &Metadata) -> Result<bool> {
    let running = fs::metadata(format!("/proc/{pid}/exe"))?;
    Ok(installed.dev() == running.dev() && installed.ino() == running.ino())
}

#[cfg(target_os = "macos")]
fn configured() -> Result<Option<ConfiguredService>> {
    use crate::util;
    use serde::Deserialize;

    let plist = util::home_dir()?.join("Library/LaunchAgents/dev.niles.remote.plist");
    if !plist.try_exists()? {
        return Ok(None);
    }
    // SAFETY: getuid takes no arguments and has no memory preconditions.
    let uid = unsafe { libc::getuid() };
    let output = Command::new("launchctl")
        .args(["print", &format!("gui/{uid}/dev.niles.remote")])
        .output()
        .context("could not run launchctl")?;
    // A missing GUI domain (112) or service (113) leaves the plist authoritative.
    if matches!(output.status.code(), Some(112 | 113)) {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct LaunchAgent {
            program: Option<Utf8PathBuf>,
            program_arguments: Option<Vec<Utf8PathBuf>>,
        }

        let output = Command::new("plutil")
            .args(["-convert", "json", "-o", "-", "--", plist.as_str()])
            .output()
            .context("could not run plutil")?;
        let agent: LaunchAgent = serde_json::from_str(checked_stdout(&output, "plutil")?)
            .context("invalid remote service plist")?;
        let program = agent
            .program
            .or_else(|| agent.program_arguments?.into_iter().next())
            .context("remote service plist has no program")?;
        return ConfiguredService::new(program, None).map(Some);
    }
    let status = checked_stdout(&output, "launchctl print")?;
    let program = status
        .lines()
        .find_map(|line| line.strip_prefix("\tprogram = "))
        .context("launchctl did not report the service program")?;
    let pid = status
        .lines()
        .find_map(|line| line.strip_prefix("\tpid = "))
        .map(str::parse)
        .transpose()
        .context("invalid launchctl service PID")?;
    ConfiguredService::new(program.into(), pid).map(Some)
}

#[cfg(target_os = "macos")]
fn uses_binary(pid: NonZeroU32, installed: &Metadata) -> Result<bool> {
    // lsof 4.91 -F emits file sets with NUL-terminated device and inode fields:
    // https://github.com/lsof-org/lsof-linux/blob/devel/lsof.man
    let output = Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "txt", "-F0Di"])
        .output()
        .context("could not run lsof")?;
    let records = checked_stdout(&output, "lsof")?;
    let device = format!("D0x{:x}", installed.dev());
    let inode = format!("i{}", installed.ino());
    let files = records
        .lines()
        .filter_map(|line| line.strip_prefix("ftxt\0"));
    let mut inspected = false;
    for file in files {
        let mut fields = file.split('\0');
        let dev = fields.clone().find(|field| field.starts_with('D'));
        let ino = fields.find(|field| field.starts_with('i'));
        inspected |= dev.is_some() && ino.is_some();
        if dev == Some(device.as_str()) && ino == Some(inode.as_str()) {
            return Ok(true);
        }
    }
    ensure!(
        inspected,
        "lsof returned no text file identities for PID {pid}"
    );
    Ok(false)
}

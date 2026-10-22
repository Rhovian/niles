use std::{env, fs, os::unix::fs::PermissionsExt, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use super::command;

pub(super) fn check() -> Result<String> {
    let executable = env::current_exe()?.canonicalize()?;
    let path = env::var("PATH").context("PATH is missing")?;
    let selected = resolve("niles", &path)?;
    ensure!(
        selected.canonicalize()? == executable,
        "{} shadows this niles ({}); put its directory first on PATH and run hash -r",
        selected.display(),
        executable.display()
    );
    let _tailscale = resolve("tailscale", &path)?;
    #[cfg(target_os = "macos")]
    macos(&_tailscale)?;
    let version: Version = serde_json::from_str(&command("tailscale", &["version", "--daemon", "--json"])
        .context("repair stale tailscale wrappers (type -a tailscale); reinstall with brew reinstall tailscale on macOS")?)?;
    version.check()?;
    let status: Status = serde_json::from_str(&command("tailscale", &["status", "--json"])?)?;
    let origin = status.origin()?;
    let prefs: Prefs = serde_json::from_str(&command("tailscale", &["debug", "prefs"])?)?;
    let username = command("id", &["-un"])?;
    ensure!(
        prefs.operator_user.as_deref() == Some(username.trim()),
        "Tailscale operator is not the current user; run: sudo tailscale set --operator=\"$(id -un)\""
    );
    #[cfg(target_os = "macos")]
    ensure!(
        command("id", &["-Gn"])?
            .split_whitespace()
            .any(|group| group == "admin"),
        "current user is not a macOS administrator; ask an administrator to run: sudo dseditgroup -o edit -a \"$(id -un)\" -t user admin"
    );
    println!("Preflight passed for {origin}");
    // Only the privileged daemon can reproduce its sudoers check. Never invoke sudo here.
    // https://github.com/tailscale/tailscale/blob/v1.96.2/ipn/ipnserver/actor.go#L238-L249
    println!("Confirm sudo policy if customized: sudo --other-user=\"$(id -un)\" --list tailscale");
    Ok(origin)
}

// std::process resolves PATH but does not expose the selected path for comparison:
// https://doc.rust-lang.org/1.89.0/std/process/struct.Command.html#method.new
fn resolve(name: &str, path: &str) -> Result<PathBuf> {
    for directory in env::split_paths(path) {
        let candidate = directory.join(name);
        match fs::metadata(&candidate) {
            Ok(metadata) if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 => {
                return Ok(candidate);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!(
        "{name} not found on PATH; install it (niles: cargo install --path . --locked --features remote; tailscale on macOS: brew install tailscale)"
    )
}

// Typed projections of upstream JSON; serde owns parsing, upstream has no preflight predicate.
// https://github.com/tailscale/tailscale/blob/v1.96.2/version/prop.go
// https://github.com/tailscale/tailscale/blob/v1.96.2/ipn/ipnstate/ipnstate.go
// https://github.com/tailscale/tailscale/blob/v1.96.2/ipn/prefs.go
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Version {
    major_minor_patch: String,
    daemon_long: String,
}
impl Version {
    fn check(&self) -> Result<()> {
        super::check_version(&self.major_minor_patch)?;
        // DaemonLong includes release build hashes; compare the numeric release component.
        let daemon = self
            .daemon_long
            .split('-')
            .next()
            .context("missing daemon version")?;
        ensure!(
            self.major_minor_patch == daemon,
            "Tailscale client {} != daemon {}; update both; on macOS run: brew upgrade tailscale; sudo brew services restart tailscale",
            self.major_minor_patch,
            self.daemon_long
        );
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Status {
    #[serde(rename = "Self")]
    node: Option<Node>,
    #[serde(default)]
    cert_domains: Option<Vec<String>>,
}
#[derive(Deserialize)]
struct Node {
    #[serde(rename = "DNSName")]
    dns_name: String,
}
impl Status {
    fn origin(&self) -> Result<String> {
        let node = self
            .node
            .as_ref()
            .context("Tailscale did not return a host DNS name; run: tailscale up")?;
        let dns = node.dns_name.trim_end_matches('.');
        let host = url::Host::parse(dns).context(
            "invalid Tailscale DNS name; enable MagicDNS at https://login.tailscale.com/admin/dns",
        )?;
        ensure!(
            self.cert_domains
                .as_ref()
                .is_some_and(|domains| domains.iter().any(|domain| domain == dns)),
            "HTTPS certificates are not enabled for {dns}; enable MagicDNS and HTTPS at https://login.tailscale.com/admin/dns"
        );
        Ok(format!("https://{host}"))
    }
}
#[derive(Deserialize)]
struct Prefs {
    #[serde(rename = "OperatorUser")]
    operator_user: Option<String>,
}

#[cfg(target_os = "macos")]
fn macos(tailscale: &std::path::Path) -> Result<()> {
    for app in [
        PathBuf::from("/Applications/Tailscale.app"),
        PathBuf::from(crate::util::home_dir()?.join("Applications/Tailscale.app")),
    ] {
        ensure!(
            !app.try_exists()?,
            "GUI Tailscale installed at {}; uninstall it using https://tailscale.com/kb/1069/uninstall, then run: brew install tailscale; sudo brew services start tailscale",
            app.display()
        );
    }
    let plist = command("plutil", &["-convert", "json", "-o", "-", "/Library/LaunchDaemons/sh.brew.tailscale.plist"])
        .context("open-source Homebrew tailscaled required; run: brew install tailscale; sudo brew services start tailscale")?;
    let daemon: Daemon = serde_json::from_str(&plist)?;
    daemon.check(tailscale)?;
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
#[derive(Deserialize)]
struct Daemon {
    #[serde(rename = "EnvironmentVariables", default)]
    environment: Option<DaemonEnvironment>,
}
#[cfg(any(target_os = "macos", test))]
#[derive(Deserialize)]
struct DaemonEnvironment {
    #[serde(rename = "PATH")]
    path: Option<String>,
}
#[cfg(any(target_os = "macos", test))]
impl Daemon {
    fn check(&self, tailscale: &std::path::Path) -> Result<()> {
        const LAUNCHD_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
        let configured_path = self
            .environment
            .as_ref()
            .and_then(|environment| environment.path.as_deref());
        let path = match configured_path {
            Some(path) => path,
            None => LAUNCHD_PATH,
        };
        let parent = tailscale
            .parent()
            .context("tailscale has no parent directory")?;
        let dictionary = if self.environment.is_none() {
            "sudo /usr/libexec/PlistBuddy -c 'Add :EnvironmentVariables dict' /Library/LaunchDaemons/sh.brew.tailscale.plist\n"
        } else {
            ""
        };
        let operation = if configured_path.is_some() {
            "Set"
        } else {
            "Add"
        };
        let kind = if configured_path.is_some() {
            ""
        } else {
            " string"
        };
        let daemon_path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", parent.display());
        let edit = format!("{operation} :EnvironmentVariables:PATH{kind} {daemon_path}")
            .replace('\'', "'\\''");
        ensure!(
            env::split_paths(path).any(|directory| directory == parent),
            "tailscaled PATH cannot find {}; run:\n{dictionary}sudo /usr/libexec/PlistBuddy -c '{edit}' /Library/LaunchDaemons/sh.brew.tailscale.plist\nsudo launchctl bootout system/sh.brew.tailscale\nsudo launchctl bootstrap system /Library/LaunchDaemons/sh.brew.tailscale.plist\nDo not use brew services restart: it regenerates the plist.",
            tailscale.display()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;

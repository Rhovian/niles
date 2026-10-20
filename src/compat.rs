use std::{
    fs,
    io::Write,
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::{
    agent_window::shell_quote,
    agents::{self, BriefDelivery, InvocationDefaults, ModelRoster},
    doctor::version_line,
    util::{find_on_path, read_optional_string, utf8_path},
};

const COMPAT_TURN_TIMEOUT: Duration = Duration::from_secs(300);
const COMPAT_POLL_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) fn compat(families: &[String]) -> Result<ExitCode> {
    let profiles = if families.is_empty() {
        agents::known_agent_ids().map(str::to_owned).collect()
    } else {
        families.to_vec()
    }
    .iter()
    .map(|family| {
        agents::profile_for(family).with_context(|| format!("unknown built-in family `{family}`"))
    })
    .collect::<Result<Vec<_>>>()?;
    let models = ModelRoster::builtin()?;
    let mut passed = true;
    println!("family\tversion\tresult\tdetail");
    for profile in profiles {
        let Some(path) = find_on_path(profile.binary) else {
            println!("{}\tmissing\tfail\tnot found on PATH", profile.id);
            passed = false;
            continue;
        };
        let version = version_line(&path, "--version");
        let spec = format!("{}:{}:low", profile.id, profile.default_model);
        let mut invocation =
            agents::invocation(&spec, None, InvocationDefaults::Headless, &models)?;
        let dir = utf8_path(
            std::env::temp_dir().join(format!("niles-compat-{}", Uuid::new_v4())),
            "compat directory",
        )?;
        fs::create_dir(&dir)?;
        fs::write(dir.join("status.log"), "")?;
        invocation
            .env
            .push(("NILES_AGENT_DIR".to_owned(), dir.to_string()));
        let nonce = Uuid::new_v4().to_string();
        let prompt = format!(
            "Run this shell command, then reply with only the word done: niles agent status working {nonce}"
        );
        match invocation.brief {
            BriefDelivery::Arg => invocation.args.push(prompt.clone()),
            BriefDelivery::Flag { path, .. } => {
                let file = dir.join("prompt.md");
                fs::write(&file, &prompt)?;
                invocation.args.extend([path.to_owned(), file.to_string()]);
            }
            BriefDelivery::Stdin => {}
            BriefDelivery::SystemPrompt(flag) => {
                invocation.args.extend([flag.to_owned(), prompt.clone()]);
            }
        }
        eprintln!(
            "{}",
            std::iter::once(path.as_str())
                .chain(invocation.args.iter().map(String::as_str))
                .map(shell_quote)
                .collect::<Vec<_>>()
                .join(" ")
        );
        let mut command = Command::new(&path);
        command
            .args(&invocation.args)
            .envs(invocation.env.iter().map(|(key, value)| (key, value)))
            .current_dir(&dir)
            .stdin(if matches!(invocation.brief, BriefDelivery::Stdin) {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(fs::File::create(dir.join("stdout"))?)
            .stderr(fs::File::create(dir.join("stderr"))?);
        let mut child = command
            .spawn()
            .with_context(|| format!("launch failed; evidence: {dir}"))?;
        if matches!(invocation.brief, BriefDelivery::Stdin) {
            child
                .stdin
                .take()
                .context("missing piped stdin")?
                .write_all(prompt.as_bytes())?;
        }
        let start = Instant::now();
        // std Child has try_wait and blocking wait, but no timed wait:
        // https://doc.rust-lang.org/1.89.0/std/process/struct.Child.html#method.try_wait
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if start.elapsed() >= COMPAT_TURN_TIMEOUT {
                child.kill()?;
                child.wait()?;
                break None;
            }
            thread::sleep(COMPAT_POLL_INTERVAL);
        };
        let failure = match status {
            None => Some(format!("timeout after {}s", COMPAT_TURN_TIMEOUT.as_secs())),
            Some(status) if !status.success() => {
                let stderr = fs::read_to_string(dir.join("stderr"))?;
                let last = stderr.lines().rfind(|line| !line.trim().is_empty());
                let detail = match last {
                    Some(line) => format!(": {line}"),
                    None => String::new(),
                };
                let code = match status.code() {
                    Some(code) => code.to_string(),
                    None => "signal".to_owned(),
                };
                Some(format!("exit {code}{detail}"))
            }
            Some(_) => {
                if read_optional_string(&dir.join("status.log"))?.as_deref()
                    != Some(format!("working: {nonce}\n").as_str())
                {
                    Some("nonce not written (tool use blocked or skipped)".to_owned())
                } else {
                    let after = version_line(&path, "--version");
                    (after != version)
                        .then(|| format!("version changed during turn: {version} -> {after}"))
                }
            }
        };
        match failure {
            Some(detail) => {
                passed = false;
                println!("{}\t{version}\tfail\t{detail}; evidence: {dir}", profile.id);
            }
            None => {
                fs::remove_dir_all(&dir)?;
                println!("{}\t{version}\tpass\t", profile.id);
            }
        }
    }
    Ok(if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

use crate::workspace_manifest::setup::SetupSelection;
use std::{
    fs,
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::{
    agent_window::shell_quote,
    agents::{self, AgentSpec, InvocationDefaults},
    config::spec::load_project_config_from,
    doctor::version_line,
    util::{current_dir_utf8, find_on_path, read_optional_string, utf8_path},
    workspace_manifest,
};

const COMPAT_TURN_TIMEOUT: Duration = Duration::from_secs(300);
const COMPAT_POLL_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) fn compat(families: &[String], selection: Option<&SetupSelection>) -> Result<ExitCode> {
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
    let root = current_dir_utf8()?;
    let models = load_project_config_from(&root)?.models;
    let manifest = workspace_manifest::load(&root, selection)?
        .context("compat requires a workspace manifest")?;
    let specs = manifest
        .ordered_bindings()
        .flat_map(|(_, binding)| binding.agents())
        .map(|agent| AgentSpec::parse(agent, &models))
        .collect::<Result<Vec<_>>>()?;
    let mut passed = true;
    println!("family\tversion\tresult\tdetail");
    for profile in profiles {
        let Some(spec) = specs.iter().find(|spec| spec.family() == profile.id) else {
            println!("{}\t-\tskip\tnot used by manifest", profile.id);
            continue;
        };
        let Some(path) = find_on_path(profile.binary) else {
            println!("{}\tmissing\tfail\tnot found on PATH", profile.id);
            passed = false;
            continue;
        };
        let version = version_line(&path, "--version");
        let mut invocation =
            agents::invocation(&spec.canonical(), InvocationDefaults::Headless, &models)?;
        let dir = utf8_path(
            std::env::temp_dir().join(format!("niles-compat-{}", Uuid::new_v4())),
            "compat directory",
        )?;
        fs::create_dir(&dir)?;
        fs::write(dir.join("status.log"), "")?;
        invocation
            .env
            .push(("NILES_AGENT_DIR".to_owned(), dir.to_string()));
        let sample_label = Uuid::new_v4().to_string();
        invocation.pass_primary(&format!(
            "The sample label for this Niles compatibility check is {sample_label}"
        ))?;
        let prompt = "For the Niles compatibility check, run `niles agent status working <sample-label>` using this run's sample label, then reply with only the word done.".to_owned();
        invocation.args.push(prompt);
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
            .stdin(Stdio::null())
            .stdout(fs::File::create(dir.join("stdout"))?)
            .stderr(fs::File::create(dir.join("stderr"))?);
        let mut child = command
            .spawn()
            .with_context(|| format!("launch failed; evidence: {dir}"))?;
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
                    != Some(format!("working: {sample_label}\n").as_str())
                {
                    Some("sample label not written (tool use blocked or skipped)".to_owned())
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

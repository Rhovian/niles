use std::{cmp::Ordering, env, fmt};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    agents::{self, AgentProfile, InvocationDefaults},
    analyze::{ProbeResult, ProbeStatus, run_probe},
    config::spec::AgentConfig,
};

pub(crate) const ALLOW_CLI_MISMATCH_ENV: &str = "NILES_ALLOW_CLI_MISMATCH";
pub(crate) const VERSION_UNAVAILABLE_PLACEHOLDER: &str = "version unavailable";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SemVer {
    major: u64,
    minor: u64,
    patch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VersionGateStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VersionGateReport {
    pub agent: String,
    pub binary: String,
    pub status: VersionGateStatus,
    pub min_version: String,
    pub tested_version: String,
    pub detected_version: Option<String>,
    pub message: String,
}

pub(crate) fn preflight_agent(
    agent: &str,
    config: Option<&AgentConfig>,
    defaults: InvocationDefaults,
    allow_cli_mismatch: bool,
) -> Result<Option<VersionGateReport>> {
    let spec = agents::parse_spec(agent)?;
    let Some(profile) = agents::profile_for(spec.family()) else {
        return Ok(None);
    };

    let invocation = agents::invocation(agent, config, defaults)?;
    let probe = run_probe(&invocation.binary, "--version");
    let report = evaluate_probe(spec.family(), &invocation.binary, profile, &probe);
    handle_preflight_report(&report, allow_cli_mismatch)?;
    Ok(Some(report))
}

pub(crate) fn evaluate_agent_probe(
    agent: &str,
    binary: &str,
    probe: &ProbeResult,
) -> Result<Option<VersionGateReport>> {
    let spec = agents::parse_spec(agent)?;
    let family = spec.family().to_owned();
    let Some(profile) = agents::profile_for(&family) else {
        return Ok(None);
    };
    Ok(Some(evaluate_probe(&family, binary, profile, probe)))
}

impl VersionGateReport {
    pub(crate) fn status_line(&self) -> String {
        let version = self
            .detected_version
            .as_deref()
            .map_or(VERSION_UNAVAILABLE_PLACEHOLDER, |version| version);
        format!(
            "version_gate: {} {} {} (min {}, tested {})",
            self.agent,
            self.status.as_str(),
            version,
            self.min_version,
            self.tested_version
        )
    }
}

impl VersionGateStatus {
    fn as_str(self) -> &'static str {
        match self {
            VersionGateStatus::Pass => "pass",
            VersionGateStatus::Warn => "warn",
            VersionGateStatus::Fail => "fail",
        }
    }
}

fn evaluate_probe(
    agent: &str,
    binary: &str,
    profile: AgentProfile,
    probe: &ProbeResult,
) -> VersionGateReport {
    let min_version = parse_profile_version(profile.min_version, agent, "min_version");
    let tested_version = parse_profile_version(profile.tested_version, agent, "tested_version");
    let (status, detected_version, message) = match probe.status {
        ProbeStatus::Success => {
            evaluate_successful_probe(agent, binary, profile, min_version, tested_version, probe)
        }
        ProbeStatus::Failed => (
            VersionGateStatus::Fail,
            None,
            format!(
                "{agent} CLI version gate failed: `{binary} --version` exited unsuccessfully. {}",
                probe_output_summary(probe)
            ),
        ),
        ProbeStatus::NotFound => (
            VersionGateStatus::Fail,
            None,
            format!("{agent} CLI version gate failed: binary `{binary}` was not found."),
        ),
    };

    VersionGateReport {
        agent: agent.to_owned(),
        binary: binary.to_owned(),
        status,
        min_version: profile.min_version.to_owned(),
        tested_version: profile.tested_version.to_owned(),
        detected_version: detected_version.map(|version| version.to_string()),
        message,
    }
}

fn evaluate_successful_probe(
    agent: &str,
    binary: &str,
    profile: AgentProfile,
    min_version: SemVer,
    tested_version: SemVer,
    probe: &ProbeResult,
) -> (VersionGateStatus, Option<SemVer>, String) {
    let output = probe_output_text(probe);
    let Some(version) = extract_semver(&output) else {
        return (
            VersionGateStatus::Fail,
            None,
            format!(
                "{agent} CLI version gate failed: could not parse a semantic version from `{binary} --version`. {}",
                probe_output_summary(probe)
            ),
        );
    };

    match version.cmp(&min_version) {
        Ordering::Less => (
            VersionGateStatus::Fail,
            Some(version),
            format!(
                "{agent} CLI {version} is below the supported minimum {}.",
                profile.min_version
            ),
        ),
        Ordering::Equal | Ordering::Greater => {
            if version > tested_version {
                (
                    VersionGateStatus::Warn,
                    Some(version),
                    format!(
                        "{agent} CLI {version} is newer than the tested version {}.",
                        profile.tested_version
                    ),
                )
            } else {
                (
                    VersionGateStatus::Pass,
                    Some(version),
                    format!(
                        "{agent} CLI {version} satisfies minimum {} and tested version {}.",
                        profile.min_version, profile.tested_version
                    ),
                )
            }
        }
    }
}

fn handle_preflight_report(report: &VersionGateReport, allow_cli_mismatch: bool) -> Result<()> {
    match report.status {
        VersionGateStatus::Pass => Ok(()),
        VersionGateStatus::Warn => {
            eprintln!(
                "warning: {} Proceeding because this is above the tested version.",
                report.message
            );
            Ok(())
        }
        VersionGateStatus::Fail => {
            if allow_cli_mismatch || env_allows_cli_mismatch()? {
                eprintln!(
                    "warning: {} Proceeding because CLI mismatch override is enabled.",
                    actionable_message(report)
                );
                Ok(())
            } else {
                bail!("{}", actionable_message(report))
            }
        }
    }
}

fn actionable_message(report: &VersionGateReport) -> String {
    format!(
        "{} Install or select `{}` version >= {} and <= tested {}, or rerun with --allow-cli-mismatch / set {ALLOW_CLI_MISMATCH_ENV}=1 to bypass.",
        report.message, report.binary, report.min_version, report.tested_version
    )
}

fn env_allows_cli_mismatch() -> Result<bool> {
    match env::var(ALLOW_CLI_MISMATCH_ENV) {
        Ok(value) => Ok(matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )),
        Err(env::VarError::NotPresent) => Ok(false),
        Err(env::VarError::NotUnicode(_)) => {
            bail!("{ALLOW_CLI_MISMATCH_ENV} must be valid Unicode")
        }
    }
}

fn parse_profile_version(version: &str, agent: &str, field: &str) -> SemVer {
    SemVer::parse_exact(version)
        .unwrap_or_else(|| panic!("{agent} {field} must be major.minor.patch, got {version:?}"))
}

fn extract_semver(text: &str) -> Option<SemVer> {
    for token in text.split_whitespace() {
        let bytes = token.as_bytes();
        for start in 0..bytes.len() {
            if bytes[start].is_ascii_digit()
                && let Some((version, _end)) = SemVer::parse_from(bytes, start)
            {
                return Some(version);
            }
        }
    }
    None
}

fn probe_output_text(probe: &ProbeResult) -> String {
    match (probe.stdout.is_empty(), probe.stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => probe.stdout.clone(),
        (true, false) => probe.stderr.clone(),
        (false, false) => format!("{}\n{}", probe.stdout, probe.stderr),
    }
}

fn probe_output_summary(probe: &ProbeResult) -> String {
    let stdout = if probe.stdout.is_empty() {
        "<empty>"
    } else {
        probe.stdout.as_str()
    };
    let stderr = if probe.stderr.is_empty() {
        "<empty>"
    } else {
        probe.stderr.as_str()
    };
    format!("stdout: {stdout:?}; stderr: {stderr:?}")
}

impl SemVer {
    fn parse_exact(value: &str) -> Option<Self> {
        let bytes = value.as_bytes();
        let (version, end) = Self::parse_from(bytes, 0)?;
        if end == bytes.len() {
            Some(version)
        } else {
            None
        }
    }

    fn parse_from(bytes: &[u8], start: usize) -> Option<(Self, usize)> {
        let mut index = start;
        let major = parse_number(bytes, &mut index)?;
        expect_byte(bytes, &mut index, b'.')?;
        let minor = parse_number(bytes, &mut index)?;
        expect_byte(bytes, &mut index, b'.')?;
        let patch = parse_number(bytes, &mut index)?;
        Some((
            Self {
                major,
                minor,
                patch,
            },
            index,
        ))
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_number(bytes: &[u8], index: &mut usize) -> Option<u64> {
    let start = *index;
    let mut value = 0_u64;
    while *index < bytes.len() && bytes[*index].is_ascii_digit() {
        let digit = u64::from(bytes[*index] - b'0');
        value = value.checked_mul(10)?.checked_add(digit)?;
        *index += 1;
    }
    if *index == start {
        return None;
    }
    Some(value)
}

fn expect_byte(bytes: &[u8], index: &mut usize, expected: u8) -> Option<()> {
    if bytes.get(*index).copied() == Some(expected) {
        *index += 1;
        Some(())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_orders_major_minor_patch() {
        assert!(SemVer::parse_exact("2.0.0") > SemVer::parse_exact("1.99.99"));
        assert!(SemVer::parse_exact("1.3.0") > SemVer::parse_exact("1.2.99"));
        assert!(SemVer::parse_exact("1.2.4") > SemVer::parse_exact("1.2.3"));
        assert_eq!(SemVer::parse_exact("1.2.3"), SemVer::parse_exact("1.2.3"));
    }

    #[test]
    fn extract_semver_scans_tokens_and_ignores_suffixes() {
        assert_eq!(
            extract_semver("codex-cli 0.142.4").map(|version| version.to_string()),
            Some("0.142.4".to_owned())
        );
        assert_eq!(
            extract_semver("2.1.197 (Claude Code)").map(|version| version.to_string()),
            Some("2.1.197".to_owned())
        );
        assert_eq!(
            extract_semver("tool 3.4.5-beta+local").map(|version| version.to_string()),
            Some("3.4.5".to_owned())
        );
    }

    #[test]
    fn gate_fails_below_minimum() {
        let probe = successful_probe("codex-cli 0.1.0");
        let report = evaluate_agent_probe("codex", "codex", &probe)
            .unwrap()
            .unwrap();

        assert_eq!(report.status, VersionGateStatus::Fail);
        assert_eq!(report.detected_version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn gate_passes_between_minimum_and_tested() {
        let probe = successful_probe("codex-cli 0.144.1");
        let report = evaluate_agent_probe("codex", "codex", &probe)
            .unwrap()
            .unwrap();

        assert_eq!(report.status, VersionGateStatus::Pass);
        assert_eq!(report.detected_version.as_deref(), Some("0.144.1"));
    }

    #[test]
    fn gate_warns_above_tested() {
        let probe = successful_probe("codex-cli 99.0.0");
        let report = evaluate_agent_probe("codex", "codex", &probe)
            .unwrap()
            .unwrap();

        assert_eq!(report.status, VersionGateStatus::Warn);
        assert_eq!(report.detected_version.as_deref(), Some("99.0.0"));
    }

    fn successful_probe(stdout: &str) -> ProbeResult {
        ProbeResult {
            status: ProbeStatus::Success,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        }
    }
}

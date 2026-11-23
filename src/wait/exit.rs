use anyhow::{Context, Result};
use std::{io::Write, process::ExitCode, time::Duration};

pub(crate) enum Outcome {
    Wake { wakes: Vec<String>, ack: String },
    Superseded,
    Timeout { timeout: Duration },
}

impl Outcome {
    pub(crate) fn emit(self, out: &mut dyn Write) -> Result<ExitCode> {
        Ok(ExitCode::from(match self {
            Self::Wake { wakes, ack } => {
                for wake in wakes {
                    writeln!(out, "{wake}").context("failed to print workspace wake")?;
                }
                writeln!(out, "ack: {ack}").context("failed to print workspace acknowledgement")?;
                out.flush()
                    .context("failed to flush workspace wake batch")?;
                super::EXIT_WAKE
            }
            Self::Superseded => {
                eprintln!("wait: superseded by a newer workspace wait");
                super::EXIT_SUPERSEDED
            }
            Self::Timeout { timeout } => {
                eprintln!(
                    "wait: timeout target=workspace timeout={}",
                    format_duration(timeout)
                );
                super::EXIT_TIMEOUT
            }
        }))
    }
}

pub(crate) fn format_duration(duration: Duration) -> String {
    let nanos = duration.as_nanos();
    if nanos == 0 {
        return "0s".to_owned();
    }
    if nanos.is_multiple_of(1_000_000_000) {
        return format!("{}s", duration.as_secs());
    }
    if nanos.is_multiple_of(1_000_000) {
        return format!("{}ms", nanos / 1_000_000);
    }
    format!("{:.3}s", duration.as_secs_f64())
}

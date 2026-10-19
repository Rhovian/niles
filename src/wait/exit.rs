use std::{process::ExitCode, time::Duration};

pub(super) enum Outcome {
    Wake,
    Superseded,
    Timeout { timeout: Duration },
}

pub(crate) struct WaitExit {
    code: u8,
    stderr: Option<String>,
}

impl WaitExit {
    pub(super) fn from_outcome(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Wake => Self {
                code: super::EXIT_WAKE,
                stderr: None,
            },
            Outcome::Superseded => Self {
                code: super::EXIT_SUPERSEDED,
                stderr: Some("wait: superseded by a newer workspace wait".to_owned()),
            },
            Outcome::Timeout { timeout } => Self {
                code: super::EXIT_TIMEOUT,
                stderr: Some(format!(
                    "wait: timeout target=workspace timeout={}",
                    format_duration(timeout)
                )),
            },
        }
    }

    pub(crate) fn emit(self) -> ExitCode {
        if let Some(stderr) = self.stderr {
            eprintln!("{stderr}");
        }
        ExitCode::from(self.code)
    }
}

fn format_duration(duration: Duration) -> String {
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

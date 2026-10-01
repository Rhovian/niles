use camino::Utf8PathBuf;
use chrono::{SecondsFormat, Utc};
use std::{fs, io::Write};

/// The watcher's own record of what it did, appended one line at a time.
#[derive(Clone)]
pub(super) struct WatchLog {
    pub(super) path: Utf8PathBuf,
}

impl WatchLog {
    pub(super) fn note(&self, line: &str) {
        let stamp = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        // A log that cannot be appended is not worth stopping the watcher over: the nudge is the
        // feature, and there is nowhere else this failure could be reported to.
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| writeln!(file, "{stamp} {line}"));
    }
}

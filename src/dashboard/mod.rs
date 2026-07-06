mod render;
mod snapshot;
#[cfg(test)]
mod snapshot_tests;
mod status;

use std::{
    io::{self, Write},
    thread,
    time::Duration,
};

use anyhow::Result;
use camino::Utf8Path;
use chrono::Utc;

const CLEAR_AND_HOME: &str = "\x1B[2J\x1B[H";
const TICK: Duration = Duration::from_secs(1);

pub(crate) fn run(workspace: &Utf8Path) -> Result<()> {
    loop {
        let snapshot = snapshot::collect(workspace, Utc::now());
        let mut stdout = io::stdout();
        write!(stdout, "{CLEAR_AND_HOME}{}", render::render(&snapshot))?;
        stdout.flush()?;
        thread::sleep(TICK);
    }
}

use super::Catalogue;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{self, Path};
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::{runtime, time};

const RESPONSE_ID: &str = "catalogue";

pub(super) fn models(catalogue: Catalogue, binary: &Path) -> Result<Value> {
    // Resolve PATH before changing cwd; relative PATH entries belong to the caller's directory.
    let binary = path::absolute(binary)?;
    // A dedicated runtime also works when called from the CLI's current-thread runtime.
    thread::spawn(move || {
        // TMPDIR can point inside the caller's repository; discovery must never load repo code.
        let cwd = tempfile::tempdir_in("/tmp")?;
        runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(read(catalogue, &binary, cwd.path()))
    })
    .join()
    .map_err(|_| anyhow::anyhow!("catalogue thread panicked"))?
}

async fn read(catalogue: Catalogue, binary: &Path, cwd: &Path) -> Result<Value> {
    let mut child = Command::new(binary)
        .args(catalogue.args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    // Tokio 1.54.1 collects both pipes while waiting; dropping its future kills the direct child.
    // https://docs.rs/tokio/1.54.1/tokio/process/struct.Child.html#method.wait_with_output
    // https://docs.rs/tokio/1.54.1/tokio/process/struct.Command.html#method.kill_on_drop
    time::timeout(Duration::from_secs(30), async {
        let mut stdin = child.stdin.take().context("catalogue stdin missing")?;
        if let Some(request) = catalogue.request {
            stdin
                .write_all(format!("{}\n", request.body).as_bytes())
                .await?;
        }
        drop(stdin);
        let output = child.wait_with_output().await?;
        anyhow::ensure!(
            output.status.success(),
            "catalogue exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let Some(pointer) = catalogue
            .request
            .and_then(|request| request.response_id_pointer)
        else {
            return Ok(serde_json::from_slice(&output.stdout)?);
        };
        for line in String::from_utf8(output.stdout)?.lines() {
            let value: Value = serde_json::from_str(line)?;
            if value.pointer(pointer).and_then(Value::as_str) == Some(RESPONSE_ID) {
                return Ok(value);
            }
        }
        anyhow::bail!("catalogue exited without a response")
    })
    .await
    .context("timed out querying models")?
}

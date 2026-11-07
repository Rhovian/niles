use std::{
    collections::BTreeMap, fs, io::ErrorKind, os::unix::process::CommandExt, process::Command,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub fn run() -> Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let bin = exe.parent().context("niles executable has no parent")?;
    if bin.ancestors().any(|path| path.ends_with("Cellar/niles")) {
        println!("Run: brew upgrade niles");
        return Ok(());
    }

    // dist 0.33.0 ships this alongside the app, not as a PATH dependency:
    // https://github.com/axodotdev/cargo-dist/blob/v0.33.0/book/src/reference/config.md#install-updater
    let updater = bin.join("niles-update");
    if updater.try_exists()? {
        return Err(Command::new(&updater).exec()).context("could not run niles-update");
    }

    // Cargo maintains this v1 tracking file even for current installs, covering legacy roots too:
    // https://github.com/rust-lang/cargo/blob/0.90.0/src/cargo/ops/common_for_install_and_uninstall.rs
    #[derive(Deserialize)]
    struct CargoInstalls {
        v1: BTreeMap<String, serde::de::IgnoredAny>,
    }
    let root = bin
        .parent()
        .context("niles install directory has no parent")?;
    let metadata = root.join(".crates.toml");
    match fs::read_to_string(&metadata) {
        Ok(text) => {
            let crates: CargoInstalls = toml::from_str(&text)
                .with_context(|| format!("invalid Cargo metadata: {}", metadata.display()))?;
            if crates.v1.keys().any(|id| id.starts_with("niles ")) {
                let mut command = String::from("cargo install niles --locked");
                let home = crate::util::home_dir()?.as_std_path().canonicalize()?;
                if root != home.join(".cargo") {
                    let root = root.to_str().context("Cargo install root is not UTF-8")?;
                    command.push_str(&format!(
                        " --root {}",
                        crate::agent_window::shell_quote(root)
                    ));
                }
                println!("Run: {command}");
                return Ok(());
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("could not read Cargo install metadata"),
    }
    bail!(
        "no updater found beside niles; for an older shell installation, rerun the shell installer from https://github.com/Rhovian/niles#quickstart"
    );
}

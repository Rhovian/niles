use crate::{agent_window, util};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::{env, fs};

const SHELL_INSTALL: &str =
    "rerun the shell installer from https://github.com/Rhovian/niles#quickstart";
const REMOTE_INSTALL: &str = "run cargo install niles --locked --features remote or scripts/install-remote.sh from a checkout";

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Channel {
    Stable,
    Nightly,
}

pub fn run(channel: Channel) -> Result<()> {
    let remote = cfg!(feature = "remote");
    let install = if remote {
        REMOTE_INSTALL
    } else {
        SHELL_INSTALL
    };
    let exe = env::current_exe()?.canonicalize()?;
    let bin = exe.parent().context("niles executable has no parent")?;
    let homebrew = bin.ancestors().any(|path| path.ends_with("Cellar/niles"));
    // dist 0.33.0 ships this alongside the app, not as a PATH dependency:
    // https://github.com/axodotdev/cargo-dist/blob/v0.33.0/book/src/reference/config.md#install-updater
    let updater = bin.join("niles-update");
    let has_shell_updater = !homebrew && updater.try_exists()?;
    if channel == Channel::Nightly && !has_shell_updater {
        bail!("nightly updates require a shell installation with niles-update; {install}");
    }
    if homebrew {
        println!("Run: brew upgrade niles");
        return Ok(());
    }
    if has_shell_updater {
        if remote {
            bail!("release binaries omit remote; {install}");
        }
        let mut command = Command::new(&updater);
        // The bundled axoupdater already selects releases; keep networking there:
        // https://github.com/axodotdev/axoupdater/blob/v0.10.2/axoupdater-cli/src/bin/axoupdater/main.rs
        if channel == Channel::Nightly {
            command.arg("--prerelease");
        }
        return Err(command.exec()).context("could not run niles-update");
    }

    // Cargo maintains this v1 tracking file even for current installs, covering legacy roots too:
    // https://github.com/rust-lang/cargo/blob/0.90.0/src/cargo/ops/common_for_install_and_uninstall.rs
    #[derive(Deserialize)]
    struct CargoInstalls {
        v1: BTreeMap<String, IgnoredAny>,
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
                if remote {
                    command.push_str(" --features remote");
                }
                let home = util::home_dir()?.as_std_path().canonicalize()?;
                if root != home.join(".cargo") {
                    let root = root.to_str().context("Cargo install root is not UTF-8")?;
                    command.push_str(&format!(" --root {}", agent_window::shell_quote(root)));
                }
                println!("Run: {command}");
                return Ok(());
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("could not read Cargo install metadata"),
    }
    let recovery = if remote {
        ""
    } else {
        "for an older shell installation, "
    };
    bail!("no updater found beside niles; {recovery}{install}");
}

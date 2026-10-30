mod defaults;
#[cfg(test)]
mod tests;
use super::{register, registry};
use crate::{skills, workspace_manifest as manifest};
use anyhow::{Context, Result, ensure};
use camino::Utf8Path;
use defaults::{RELEASE, REPOSITORY};
use manifest::WorkspaceManifest;
use std::{
    fs::{self, OpenOptions},
    io::{self, IsTerminal, Write},
    process::Command,
};
const DEFAULT_EDITOR: &str = "vi";
pub(crate) fn setup(cwd: &Utf8Path) -> Result<Option<WorkspaceManifest>> {
    let installed: Vec<_> = crate::agents::profiles()
        .filter(|p| crate::util::find_on_path(p.binary).is_some())
        .map(|p| p.id)
        .collect();
    ensure!(
        !installed.is_empty(),
        "no CLI found; install claude, codex or pi"
    );
    let found = installed.join(", ");
    let global = skills::global_dir()?;

    let notice = format!(
        "Niles isn't set up in {cwd}. Found: {found}.\nMissing skills will be pinned from {REPOSITORY}@{RELEASE} into {global}/skills. Existing skills are kept.\nWorkers run with approval prompts bypassed and no sandbox (docs/security.md).\n[Enter] default · [c] custom (edit the manifest yourself) · [n] skip"
    );
    let answer = ask(&notice, &["", "c", "n"])?;
    if answer == "n" {
        return Ok(None);
    }
    let models = crate::config::spec::load_project_config_from(cwd)?.models;
    let generated = defaults::manifest(&installed)?;
    let body = serde_saphyr::to_string(&generated)?;
    let (body, manifest) = if answer == "c" {
        let Some(edited) = custom(cwd, &body, &models)? else {
            return Ok(None);
        };
        edited
    } else {
        manifest::check(&generated, &models)?;
        (body, generated)
    };
    let names: std::collections::BTreeSet<_> = manifest
        .bindings
        .iter()
        .flat_map(|(name, binding)| std::iter::once(name).chain(binding.attached_skills()))
        .collect();
    for name in names {
        if skills::find(name, cwd, &global)?.is_none() {
            println!("pinning {name}");
            skills::import(defaults::reference(name)?, skills::Origin::Global)?;
        }
    }
    let path = manifest::manifest_path(cwd, None)?;
    fs::create_dir_all(path.parent().context("manifest has no parent")?)?;
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => file.write_all(body.as_bytes())?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    manifest::load(cwd, None)
}
fn ask(text: &str, choices: &[&str]) -> Result<String> {
    // std 1.89 read_line frames input; it does not implement choices or EOF policy.
    loop {
        print!("{text} ");
        io::stdout().flush()?;
        let mut line = String::new();
        ensure!(io::stdin().read_line(&mut line)? != 0, "setup input ended");
        let answer = line.trim().to_owned();
        if choices.is_empty() || choices.contains(&answer.as_str()) {
            return Ok(answer);
        }
        println!("Unrecognised answer.");
    }
}
fn custom(
    cwd: &Utf8Path,
    generated: &str,
    models: &crate::agents::ModelRoster,
) -> Result<Option<(String, WorkspaceManifest)>> {
    let dir = crate::store::paths::project_dir(cwd)?;
    fs::create_dir_all(&dir)?;
    // tempfile 3.27.0 file/imp/unix.rs: exclusive creation, mode 0600; Drop removes the draft.
    let file = tempfile::NamedTempFile::new_in(dir)?;
    writeln!(
        file.as_file(),
        "# Configuration: https://github.com/Rhovian/niles/blob/main/docs/configuration.md\n{generated}"
    )?;
    let path = Utf8Path::from_path(file.path()).context("draft path is not UTF-8")?;
    loop {
        edit(path)?;
        let body = fs::read_to_string(path)?;
        let parsed = crate::store::parse_yaml(&body).and_then(|manifest| {
            manifest::check(&manifest, models)?;
            Ok(manifest)
        });
        let error = match parsed {
            Ok(manifest) => return Ok(Some((body, manifest))),
            Err(error) => error,
        };
        let retry = format!("{error:#}\n[Enter] edit again · [q] abort");
        if ask(&retry, &["", "q"])? == "q" {
            return Ok(None);
        }
    }
}
pub(super) fn first_run(cwd: &Utf8Path) -> Result<Option<registry::Entry>> {
    if !io::stdin().is_terminal() || !registry::entries()?.is_empty() {
        return Ok(None);
    }
    let (path, mut name) = match register::directory("", cwd)? {
        register::Directory::Registered(entry) => return Ok(Some(entry)),
        register::Directory::Unregistered { path, default } => (path, default),
    };
    let notice = format!(
        "No projects yet. Register {path} as {name:?} and set it up now?\nRegistering is optional: press r in the home view to register any folder later.\n[Enter] register and set up · [n] not now"
    );
    if ask(&notice, &["", "n"])? == "n" {
        return Ok(None);
    }
    loop {
        match register::register(&name, &path) {
            Ok(entry) => return Ok(Some(entry)),
            Err(error) if error.downcast_ref::<io::Error>().is_some() => return Err(error),
            Err(error) => name = ask(&format!("{error:#}\nname:"), &[])?,
        }
    }
}
fn edit(path: &Utf8Path) -> Result<()> {
    let editor = std::env::var_os("VISUAL")
        .or_else(|| std::env::var_os("EDITOR"))
        .unwrap_or_else(|| DEFAULT_EDITOR.into());
    let editor = editor.to_str().context("editor command is not UTF-8")?;
    let status = Command::new("sh")
        .args(["-c", &format!("{editor} \"$1\""), "sh", path.as_str()])
        .status()
        .context("failed to run manifest editor")?;
    ensure!(status.success(), "manifest editor exited with {status}");
    Ok(())
}

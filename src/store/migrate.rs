use std::{fs, io::ErrorKind, os::unix::fs::symlink};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::util::read_dir_utf8_paths;

pub(super) fn legacy(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("failed to inspect {source}")),
    };
    if !metadata.is_dir() {
        bail!("cannot migrate {source}: expected a directory, not a file or symlink");
    }
    let parent = destination
        .parent()
        .context("state directory has no parent")?;
    fs::create_dir_all(parent)?;
    let key = destination
        .file_name()
        .context("state directory has no name")?;
    let lock = crate::wait::cursor::open_lock_file(&parent.join(format!(".{key}.lock")))?;
    lock.lock().context("failed to lock state migration")?;
    // Another Niles process may have completed the move while we waited.
    match fs::symlink_metadata(source) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("failed to inspect {source}")),
        Ok(metadata) if !metadata.is_dir() => {
            bail!("cannot migrate {source}: expected a directory")
        }
        Ok(_) => {}
    }
    match fs::symlink_metadata(destination) {
        Ok(_) => bail!(
            "cannot migrate {source}: {destination} already exists; reconcile the directories first"
        ),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {destination}"));
        }
    }
    move_dir(source, destination)
}

pub(crate) fn move_dir(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
            let name = destination
                .file_name()
                .context("move destination has no name")?;
            let staging = destination.with_file_name(format!(".{name}.moving"));
            // std::fs::copy copies one file, not a directory (Rust 1.89 std/fs.rs).
            // Publish only a complete copy; a failed copy leaves the source and staging intact.
            copy_dir(source, &staging)?;
            fs::rename(&staging, destination)?;
            fs::remove_dir_all(source).with_context(|| format!("failed to remove {source}"))
        }
        Err(error) => {
            Err(error).with_context(|| format!("failed to move {source} to {destination}"))
        }
    }
}

fn copy_dir(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    fs::create_dir(destination).with_context(|| format!("failed to create {destination}"))?;
    for path in read_dir_utf8_paths(source)? {
        let target = destination.join(path.file_name().context("state entry has no name")?);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            copy_dir(&path, &target)?;
        } else if metadata.is_symlink() {
            symlink(fs::read_link(&path)?, &target)?;
        } else if metadata.is_file() {
            fs::copy(&path, &target)
                .with_context(|| format!("failed to copy {path} to {target}"))?;
        } else {
            bail!("cannot migrate special file {path}");
        }
    }
    fs::set_permissions(destination, fs::metadata(source)?.permissions())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_path;

    #[test]
    fn staged_copy_preserves_files_and_symlinks_without_following_them() {
        let root = temp_test_path("migration-copy");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/report"), b"bytes\0").unwrap();
        symlink("missing", source.join("link")).unwrap();
        copy_dir(&source, &target).unwrap();
        assert_eq!(fs::read(target.join("nested/report")).unwrap(), b"bytes\0");
        assert_eq!(
            fs::read_link(target.join("link")).unwrap(),
            std::path::Path::new("missing")
        );
        assert!(copy_dir(&source, &target).is_err());
        assert!(source.join("nested/report").is_file());
        fs::remove_dir_all(root).unwrap();
    }
}

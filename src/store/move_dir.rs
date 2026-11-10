#[cfg(test)]
use crate::store::atomic_write;
use crate::util::read_dir_utf8_paths;
use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use std::os::unix::fs::symlink;
use std::{fs, io};

pub(crate) fn move_dir(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    move_dir_with_rename(source, destination, |source, destination| {
        fs::rename(source, destination)
    })
}

fn move_dir_with_rename(
    source: &Utf8Path,
    destination: &Utf8Path,
    rename: impl FnOnce(&Utf8Path, &Utf8Path) -> io::Result<()>,
) -> Result<()> {
    match rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
            copy_then_remove(source, destination)
        }
        Err(error) => {
            Err(error).with_context(|| format!("failed to move {source} to {destination}"))
        }
    }
}

fn copy_then_remove(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    // tempfile 3.27 removes staging on failure; std::fs::copy only copies individual files:
    // https://docs.rs/tempfile/3.27.0/tempfile/struct.TempDir.html
    let temporary = tempfile::Builder::new().prefix(".moving-").tempdir_in(
        destination
            .parent()
            .context("move destination has no parent")?,
    )?;
    let staging = camino::Utf8PathBuf::from_path_buf(temporary.path().join("contents"))
        .map_err(|path| anyhow::anyhow!("non-UTF-8 staging path: {}", path.display()))?;
    copy_dir(source, &staging).with_context(|| format!("failed to copy {source} to {staging}"))?;
    fs::rename(&staging, destination)?;
    fs::remove_dir_all(source).with_context(|| format!("failed to remove {source}"))
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
            bail!("cannot copy special file {path}");
        }
    }
    fs::set_permissions(destination, fs::metadata(source)?.permissions())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_path;
    use std::{ffi::CString, io::Error, path::Path};

    fn force_exdev(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
        move_dir_with_rename(source, destination, |_, _| {
            Err(Error::from_raw_os_error(libc::EXDEV))
        })
    }

    #[test]
    fn copy_and_cross_filesystem_move_preserve_files_and_symlinks() {
        for (transfer, moved) in [
            (copy_dir as fn(&Utf8Path, &Utf8Path) -> Result<()>, false),
            (force_exdev, true),
        ] {
            let root = temp_test_path("move-copy");
            let source = root.join("source");
            let target = root.join("target");
            fs::create_dir_all(source.join("nested")).unwrap();
            atomic_write(source.join("nested/report"), b"report\0bytes").unwrap();
            symlink("missing", source.join("link")).unwrap();
            transfer(&source, &target).unwrap();
            assert_eq!(source.exists(), !moved);
            assert_eq!(
                fs::read(target.join("nested/report")).unwrap(),
                b"report\0bytes"
            );
            assert_eq!(
                fs::read_link(target.join("link")).unwrap(),
                Path::new("missing")
            );
            if moved {
                assert_eq!(read_dir_utf8_paths(&root).unwrap(), vec![target]);
            } else {
                assert!(copy_dir(&source, &target).is_err());
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn failed_copies_remove_staging_and_preserve_the_source_and_original_error() {
        let root = temp_test_path("move-retry");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        atomic_write(source.join("report"), "retained").unwrap();
        let fifo = source.join("fifo");
        let fifo_c = CString::new(fifo.as_str()).unwrap();
        unsafe {
            assert_eq!(libc::mkfifo(fifo_c.as_ptr(), 0o600), 0);
        }
        for _ in 0..3 {
            let error = force_exdev(&source, &target).unwrap_err();
            assert!(format!("{error:#}").contains(&format!("cannot copy special file {fifo}")));
            assert!(!target.exists());
            assert_eq!(read_dir_utf8_paths(&root).unwrap(), vec![source.clone()]);
            assert_eq!(
                fs::read_to_string(source.join("report")).unwrap(),
                "retained"
            );
        }
        fs::remove_file(fifo).unwrap();
        force_exdev(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(
            fs::read_to_string(target.join("report")).unwrap(),
            "retained"
        );
        fs::remove_dir_all(root).unwrap();
    }
}

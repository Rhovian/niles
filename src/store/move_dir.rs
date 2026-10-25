use std::{fs, os::unix::fs::symlink};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::util::read_dir_utf8_paths;

pub(crate) fn move_dir(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    move_dir_with_rename(source, destination, |source, destination| {
        fs::rename(source, destination)
    })
}

fn move_dir_with_rename(
    source: &Utf8Path,
    destination: &Utf8Path,
    rename: impl FnOnce(&Utf8Path, &Utf8Path) -> std::io::Result<()>,
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
    let name = destination
        .file_name()
        .context("move destination has no name")?;
    let staging = destination.with_file_name(format!(".{name}.moving-{}", uuid::Uuid::new_v4()));
    // std::fs::copy copies one file, not a directory (Rust 1.89 std/fs.rs).
    if let Err(error) = copy_dir(source, &staging) {
        // The source is intact. Cleanup must not replace the original copy failure.
        let _ = fs::remove_dir_all(&staging);
        return Err(error).with_context(|| format!("failed to copy {source} to {staging}"));
    }
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

    #[test]
    fn staged_copy_preserves_files_and_symlinks_without_following_them() {
        let root = temp_test_path("move-copy");
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
    fn force_exdev(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
        move_dir_with_rename(source, destination, |_, _| {
            Err(std::io::Error::from_raw_os_error(libc::EXDEV))
        })
    }

    #[test]
    fn cross_filesystem_move_publishes_the_copy_and_removes_the_source() {
        let root = temp_test_path("move-exdev");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/report"), b"report\0bytes").unwrap();
        symlink("missing", source.join("link")).unwrap();
        force_exdev(&source, &target).unwrap();
        assert!(!source.exists(), "publishing must remove the source tree");
        assert_eq!(
            fs::read(target.join("nested/report")).unwrap(),
            b"report\0bytes"
        );
        assert_eq!(
            fs::read_link(target.join("link")).unwrap(),
            std::path::Path::new("missing")
        );
        assert_eq!(read_dir_utf8_paths(&root).unwrap(), vec![target]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_copies_remove_staging_and_preserve_the_source_and_original_error() {
        let root = temp_test_path("move-retry");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("report"), "retained").unwrap();
        let fifo = source.join("fifo");
        let fifo_c = std::ffi::CString::new(fifo.as_str()).unwrap();
        unsafe {
            assert_eq!(libc::mkfifo(fifo_c.as_ptr(), 0o600), 0);
        }
        for _ in 0..3 {
            let error = force_exdev(&source, &target).unwrap_err();
            assert!(format!("{error:#}").contains(&format!("cannot copy special file {fifo}")));
            assert!(!target.exists());
            assert!(
                read_dir_utf8_paths(&root)
                    .unwrap()
                    .iter()
                    .all(|path| !path.file_name().unwrap().starts_with(".target.moving-"))
            );
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

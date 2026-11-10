use super::*;
use crate::store::atomic_write;
use crate::test_support::{skill, temp_dir};
use std::os::unix::fs::symlink;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;
use std::{fs, io, thread};

fn write(root: &Utf8Path, name: &str, text: &str) -> Utf8PathBuf {
    let path = root.join("skills").join(name).join("SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    atomic_write(&path, text).unwrap();
    path
}
fn document(name: &str, description: &str, body: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\ncompatibility: other-tools\n---\n{body}"
    )
}

#[test]
fn roots_override_without_reading_shadowed_content() {
    let tmp = temp_dir("skills-resolve");
    let root: &Utf8Path = &tmp;
    let workspace = root.join("workspace");
    let name = "worker".parse().unwrap();
    let error = resolve(&name, &workspace, root).unwrap_err();
    assert!(error.to_string().contains("has no SKILL.md"));
    let path = write(
        root,
        "worker",
        &document("worker", "global", "global body\n"),
    );
    let global = resolve(&name, &workspace, root).unwrap();
    assert_eq!(global.body, "global body\n");
    assert_eq!(global.to_string(), format!("worker (global {path})"));
    atomic_write(path, "broken lower-priority content").unwrap();
    let body = "\nexact \" {workspace}\r\n";
    let path = write(
        &workspace.join(".niles"),
        "worker",
        &document("worker", "workspace", body),
    );
    let winner = resolve(&name, &workspace, root).unwrap();
    assert_eq!(winner.body, body);
    assert_eq!(winner.to_string(), format!("worker (workspace {path})"));
    let blocked = root.join("blocked-global-root");
    atomic_write(&blocked, "not a directory").unwrap();
    assert_eq!(resolve(&name, &workspace, &blocked).unwrap().body, body);
    fs::remove_file(&path).unwrap();
    let pointer = path.with_file_name("SKILL.ref");
    atomic_write(&pointer, "junk").unwrap();
    assert!(find(&name, &workspace, root).is_err());
    assert_eq!(fs::read_to_string(pointer).unwrap(), "junk");
}

#[test]
fn frontmatter_checks_are_content_safe_and_preserve_the_body() {
    let name = skill("example");
    let secret = "SECRET_FRONTMATTER";
    for text in [
        "body only".into(),
        "---\nname: example\n".into(),
        format!("---\nname: [\ndescription: {secret}\n---\nbody"),
        document(secret, "valid", "body"),
        document("example", "''", "body"),
        document("example", "'  '", "body"),
        document("example", &"é".repeat(1025), "body"),
        "---\nname: example\n---\nbody".into(),
    ] {
        let error = parse(&name, &text).unwrap_err().to_string();
        assert!(!error.contains(secret), "{error}");
    }
    let text = document("example", &"é".repeat(1024), "body\n").replace('\n', "\r\n");
    let (description, body) = parse(&name, &text).unwrap();
    assert_eq!(description.chars().count(), 1024);
    assert_eq!(body, "body\r\n");
}

#[test]
fn workspace_components_reject_symlinks_but_global_links_resolve() {
    let tmp = temp_dir("skills-resolve");
    let root: &Utf8Path = &tmp;
    let target = write(root, "example", &document("example", "valid", "body"));
    for (workspace_origin, directory_link) in
        [(true, true), (true, false), (false, true), (false, false)]
    {
        let workspace = root.join(format!("case-{workspace_origin}-{directory_link}"));
        let global = workspace.join("global");
        let base = if workspace_origin {
            workspace.join(".niles")
        } else {
            global.clone()
        };
        let dir = base.join("skills/example");
        fs::create_dir_all(dir.parent().unwrap()).unwrap();
        if directory_link {
            symlink(target.parent().unwrap(), &dir).unwrap();
        } else {
            fs::create_dir(&dir).unwrap();
            symlink(&target, dir.join("SKILL.md")).unwrap();
        }
        let result = resolve(&skill("example"), &workspace, &global);
        if workspace_origin {
            assert!(format!("{:#}", result.unwrap_err()).contains(dir.as_str()));
        } else {
            assert_eq!(result.unwrap().body, "body");
        }
    }
}

#[test]
fn oversized_and_special_files_are_rejected_without_blocking() {
    let tmp = temp_dir("skills-invalid-files");
    let root: &Utf8Path = &tmp;
    let path = root.join("skills/example/SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    for (create, remove, expected) in [
        (
            (|path: &Utf8Path| fs::create_dir(path)) as fn(&Utf8Path) -> io::Result<()>,
            (|path: &Utf8Path| fs::remove_dir(path)) as fn(&Utf8Path) -> io::Result<()>,
            &["regular file"][..],
        ),
        (
            |path| {
                assert!(Command::new("mkfifo").arg(path).status()?.success());
                Ok(())
            },
            |path| fs::remove_file(path),
            &["regular file"][..],
        ),
        (
            |path| {
                atomic_write(path, document("example", "valid", "body")).unwrap();
                fs::OpenOptions::new()
                    .write(true)
                    .open(path)?
                    .set_len(8 * 1024 * 1024 * 1024)
            },
            |path| fs::remove_file(path),
            &["maximum is 131072", "8589934592", "read 131073"][..],
        ),
    ] {
        create(&path).unwrap();
        let (tx, rx) = mpsc::channel();
        let root = root.to_owned();
        thread::spawn(move || tx.send(resolve(&skill("example"), &root, &root)).unwrap());
        let error = format!(
            "{:#}",
            rx.recv_timeout(Duration::from_secs(3))
                .expect("skill read blocked")
                .unwrap_err()
        );
        assert!(error.contains(path.as_str()), "{error}");
        for needle in expected {
            assert!(error.contains(needle), "{error}");
        }
        remove(&path).unwrap();
    }
}

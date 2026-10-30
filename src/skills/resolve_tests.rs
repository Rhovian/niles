use super::*;
use std::{fs, os::unix::fs::symlink};

fn write(root: &Utf8Path, name: &str, text: &str) -> Utf8PathBuf {
    let path = root.join("skills").join(name).join("SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    path
}
fn document(name: &str, description: &str, body: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\ncompatibility: other-tools\n---\n{body}"
    )
}

#[test]
fn roots_override_without_reading_shadowed_content() {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(tmp.path()).unwrap();
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
    fs::write(path, "broken lower-priority content").unwrap();
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
    fs::write(&blocked, "not a directory").unwrap();
    assert_eq!(resolve(&name, &workspace, &blocked).unwrap().body, body);
    fs::remove_file(&path).unwrap();
    let pointer = path.with_file_name("SKILL.ref");
    fs::write(&pointer, "junk").unwrap();
    assert!(find(&name, &workspace, root).is_err());
    assert_eq!(fs::read_to_string(pointer).unwrap(), "junk");
}

#[test]
fn frontmatter_checks_are_content_safe_and_preserve_the_body() {
    let name = "example".parse().unwrap();
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
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(tmp.path()).unwrap();
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
        let result = resolve(&"example".parse().unwrap(), &workspace, &global);
        if workspace_origin {
            assert!(format!("{:#}", result.unwrap_err()).contains(dir.as_str()));
        } else {
            assert_eq!(result.unwrap().body, "body");
        }
    }
}

#[test]
fn oversized_documents_are_rejected_before_parsing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(tmp.path()).unwrap();
    let path = write(root, "example", &document("example", "valid", "body"));
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(8 * 1024 * 1024 * 1024)
        .unwrap();
    let error = format!(
        "{:#}",
        resolve(&"example".parse().unwrap(), root, root)
            .map(|_| ())
            .expect_err("oversized document accepted")
    );
    assert!(
        error.contains(path.as_str())
            && error.contains("maximum is 131072")
            && error.contains("8589934592")
            && error.contains("read 131073"),
        "{error}"
    );
}

#[test]
fn global_special_files_are_rejected_without_blocking() {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(tmp.path()).unwrap();
    let path = root.join("skills/example/SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    for fifo in [false, true] {
        if fifo {
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
        } else {
            fs::create_dir(&path).unwrap();
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let root = root.to_owned();
        std::thread::spawn(move || {
            tx.send(resolve(&"example".parse().unwrap(), &root, &root))
                .unwrap()
        });
        let error = format!(
            "{:#}",
            rx.recv_timeout(std::time::Duration::from_secs(3))
                .expect("skill read blocked")
                .unwrap_err()
        );
        assert!(
            error.contains(path.as_str()) && error.contains("regular file"),
            "{error}"
        );
        if fifo {
            fs::remove_file(&path).unwrap();
        } else {
            fs::remove_dir(&path).unwrap();
        }
    }
}

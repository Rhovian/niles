use std::{fs, path::Path};

pub fn copy_skills(root: &Path) {
    for entry in fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("skills")).unwrap() {
        let entry = entry.unwrap();
        if !entry.file_type().unwrap().is_dir() {
            continue;
        }
        let source = entry.path();
        if !source.join("SKILL.md").try_exists().unwrap() {
            continue;
        }
        let dest = root.join("skills").join(source.file_name().unwrap());
        fs::create_dir_all(&dest).unwrap();
        let document = dest.join("SKILL.md");
        if !document.try_exists().unwrap() && !dest.join("SKILL.ref").try_exists().unwrap() {
            fs::copy(source.join("SKILL.md"), document).unwrap();
        }
    }
}

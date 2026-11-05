use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sovra-skills-test-{}-{}",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn package(&self, name: &str, content: &str) -> PathBuf {
        let directory = self.0.join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
        directory
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const SOURCE: &str = "---\nname: weekly-review\ndescription: Use when reviewing the week's work.\nallowed-tools: read_attachment\nmetadata:\n  author: Test\n---\n\nRead the supplied files.\n\n---\nKeep this Markdown rule inside the body.\n";

#[test]
fn malformed_boolean_metadata_cannot_silently_enable_a_skill() {
    for key in ["sovra-enabled", "sovra-requires-input", "sovra-auto-apply"] {
        let invalid = SOURCE.replace("author: Test", &format!("{key}: definitely"));
        assert!(format::decode(&invalid, "weekly-review").is_err());
    }
}

#[test]
fn automatic_use_defaults_on_and_respects_both_opt_out_spellings() {
    assert!(format::decode(SOURCE, "weekly-review").unwrap().auto_apply);
    let manual = SOURCE.replace(
        "allowed-tools:",
        "disable-model-invocation: true\nallowed-tools:",
    );
    let mut skill = format::decode(&manual, "weekly-review").unwrap();
    assert!(!skill.auto_apply);
    let overridden = SOURCE.replace("author: Test", "sovra-auto-apply: false");
    assert!(
        !format::decode(&overridden, "weekly-review")
            .unwrap()
            .auto_apply
    );
    assert!(!skill.metadata.contains_key("sovra-auto-apply"));
    skill.auto_apply = true;
    let encoded = format::encode(&skill).unwrap();
    assert!(!encoded.contains("disable-model-invocation"));
    assert!(
        format::decode(&encoded, "weekly-review")
            .unwrap()
            .auto_apply
    );
    skill.auto_apply = false;
    let encoded = format::encode(&skill).unwrap();
    assert!(
        !format::decode(&encoded, "weekly-review")
            .unwrap()
            .auto_apply
    );
}

#[test]
fn imports_edits_reopens_and_removes_without_losing_resources() {
    let fixture = Fixture::new();
    let source = fixture.package("weekly-review", SOURCE);
    fs::create_dir(source.join("references")).unwrap();
    fs::write(source.join("references/guide.md"), "Reference text.").unwrap();
    let root = fixture.0.join("installed");
    let store = LocalSkillStore::new(root.clone());
    let mut skill = store.import(&source).unwrap();
    assert!(skill.instructions.contains("---\nKeep"));
    assert_eq!(skill.tools, ["read_attachment"]);
    assert_eq!(
        skill.metadata.get("author").map(String::as_str),
        Some("Test")
    );
    skill.license = Some("MIT".into());
    skill
        .metadata
        .insert("sovra-future-option".into(), "retain this".into());
    skill.compatibility = Some("Sovra with file reading enabled".into());
    assert!(store.import(&source).is_err());
    skill.instructions = "Updated procedure.\nNever change source files.".into();
    skill.enabled = false;
    skill.aliases = vec!["weekly".into()];
    store.save(&skill).unwrap();
    let reopened = LocalSkillStore::new(root);
    let listed = reopened.list().unwrap();
    assert!(listed.issues.is_empty(), "{:?}", listed.issues);
    assert_eq!(listed.skills, [skill]);
    assert_eq!(
        reopened
            .resource("weekly-review", "references/guide.md")
            .unwrap(),
        "Reference text."
    );
    for path in [
        "../private.txt",
        "/etc/passwd",
        "references/../../private.txt",
        "references\\guide.md",
        ".env",
        "",
    ] {
        assert!(reopened.resource("weekly-review", path).is_err(), "{path}");
    }
    reopened.remove("weekly-review").unwrap();
    assert_eq!(reopened.list().unwrap().skills, [] as [Skill; 0]);
    assert!(
        source.join("SKILL.md").exists(),
        "uninstall never removes the import source"
    );
}

#[test]
fn malformed_packages_are_reported_without_hiding_valid_skills() {
    let fixture = Fixture::new();
    fixture.package("weekly-review", SOURCE);
    fixture.package("broken", "name: broken");
    let store = LocalSkillStore::new(fixture.0.clone());
    let listing = store.list().unwrap();
    assert_eq!(listing.skills.len(), 1);
    assert_eq!(listing.issues.len(), 1);
    assert!(listing.issues[0].starts_with("broken:"));
    assert!(store.remove("files").is_err());
    assert!(store.remove("../escape").is_err());
}

#[test]
fn yaml_limits_reject_aliases_duplicate_fields_tags_and_name_mismatches() {
    assert!(format::decode(SOURCE, "wrong-name").is_err());
    for header in [
        "name: weekly-review\nname: other\ndescription: test",
        "name: &n weekly-review\ndescription: *n",
        "name: weekly-review\ndescription: !include /etc/passwd",
    ] {
        assert!(format::decode(&format!("---\n{header}\n---\nBody"), "weekly-review").is_err());
    }
    let oversized = format!("{SOURCE}{}", "x".repeat(MAX_SKILL_BYTES));
    assert!(format::decode(&oversized, "weekly-review").is_err());
    let crlf = SOURCE.replace('\n', "\r\n");
    assert!(format::decode(&crlf, "weekly-review").is_ok());
}

#[cfg(unix)]
#[test]
fn symlinked_resources_and_packages_cannot_escape_and_storage_is_private() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let fixture = Fixture::new();
    let source = fixture.package("weekly-review", SOURCE);
    let root = fixture.0.join("installed");
    let store = LocalSkillStore::new(root.clone());
    store.import(&source).unwrap();
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(root.join("weekly-review/SKILL.md"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    symlink(&source, root.join("weekly-review/outside")).unwrap();
    assert!(store.resource("weekly-review", "outside/SKILL.md").is_err());
    symlink(&source, root.join("linked")).unwrap();
    assert!(store.resource("linked", "SKILL.md").is_err());
    assert!(store.remove("linked").is_err());
    symlink(source.join("SKILL.md"), source.join("outside.md")).unwrap();
    let other = LocalSkillStore::new(fixture.0.join("second-install"));
    assert!(other.import(&source).is_err());
    assert_eq!(other.list().unwrap().skills, [] as [Skill; 0]);
}

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

const PLUGIN: &[u8] =
    include_bytes!("../../../examples/plugins/packages/weekly-planning-1.0.0.json");

#[test]
fn plugins_reopen_offline_and_keep_index_and_content_separate() {
    use sovra_core::plugins::{InstalledPlugin, PluginEntry, PluginLibrary, PluginStore};
    let fixture = Fixture::new();
    let directory = fixture.0.join("plugins");
    let store = LocalPluginStore::new(directory.clone());
    assert_eq!(store.load().unwrap().installed, [] as [InstalledPlugin; 0]);
    let entry =
        PluginEntry::for_package("https://example.com/package.json".into(), PLUGIN).unwrap();
    store.cache(&entry, PLUGIN).unwrap();
    let library = PluginLibrary {
        catalog_url: Some("https://example.com/catalog.json".into()),
        installed: vec![InstalledPlugin {
            entry: entry.clone(),
            enabled: false,
            catalog_url: "https://example.com/catalog.json".into(),
            github: None,
        }],
    };
    store.save(&library).unwrap();
    let reopened = LocalPluginStore::new(directory.clone());
    assert_eq!(reopened.load().unwrap(), library);
    assert_eq!(reopened.package(&entry).unwrap().id, entry.id);
    assert!(
        !fs::read_to_string(directory.join("plugins.json"))
            .unwrap()
            .contains("instructions")
    );
    let mut changed = library;
    changed.installed[0].enabled = true;
    reopened.save(&changed).unwrap();
    assert!(reopened.load().unwrap().installed[0].enabled);
    assert!(reopened.cache(&entry, b"changed").is_err());
    assert_eq!(reopened.package(&entry).unwrap().id, entry.id);
    fs::write(directory.join(format!("{}.json", entry.sha256)), b"corrupt").unwrap();
    assert!(reopened.package(&entry).is_err());
    assert!(
        reopened.load().is_ok(),
        "a damaged package must not hide the controls needed to disable it"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(directory.join("plugins.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn publisher_pages_a_large_catalog_and_never_overwrites_an_existing_publication() {
    use sovra_core::plugins::{PluginCatalog, PluginPackage};
    let fixture = Fixture::new();
    let source = fixture.0.join("source");
    fs::create_dir(&source).unwrap();
    for index in 0..100 {
        let mut package: PluginPackage = serde_json::from_slice(PLUGIN).unwrap();
        package.id = format!("io.example.plugin-{index}");
        package.title = format!("Helper {index:03}");
        fs::write(
            source.join(format!("{index}.json")),
            serde_json::to_vec(&package).unwrap(),
        )
        .unwrap();
    }
    let output = fixture.0.join("public");
    check_catalog(&source, "https://example.github.io/plugins").unwrap();
    assert!(!output.exists());
    assert_eq!(fs::read_dir(&source).unwrap().count(), 100);
    publish_catalog(&source, &output, "https://example.github.io/plugins").unwrap();
    let first = PluginCatalog::parse(&fs::read(output.join("catalog.json")).unwrap()).unwrap();
    assert_eq!(first.plugins.len(), 48);
    assert_eq!(
        first.next.as_deref(),
        Some("https://example.github.io/plugins/catalog-2.json")
    );
    let last = PluginCatalog::parse(&fs::read(output.join("catalog-3.json")).unwrap()).unwrap();
    assert_eq!(last.plugins.len(), 4);
    assert!(last.next.is_none());
    assert_eq!(
        last.previous.as_deref(),
        Some("https://example.github.io/plugins/catalog-2.json")
    );
    for entry in first.plugins.iter().chain(&last.plugins) {
        let bytes = fs::read(
            output
                .join("packages")
                .join(format!("{}.json", entry.sha256)),
        )
        .unwrap();
        entry.verify_package(&bytes).unwrap();
    }
    assert!(publish_catalog(&source, &output, "https://example.github.io/plugins").is_err());
    assert!(output.join("catalog.json").exists());
}

#[test]
fn catalog_check_names_invalid_sources_and_leaves_them_untouched() {
    let fixture = Fixture::new();
    let source = fixture.0.join("source");
    fs::create_dir(&source).unwrap();
    let bad_path = source.join("broken.json");
    fs::write(&bad_path, b"{}").unwrap();
    let url = "https://example.github.io/plugins";
    let error = check_catalog(&source, url).unwrap_err();
    assert!(error.0.contains("broken.json"));
    assert_eq!(fs::read(&bad_path).unwrap(), b"{}");
    let output = fixture.0.join("public");
    assert_eq!(publish_catalog(&source, &output, url).unwrap_err(), error);
    assert!(!output.exists());

    fs::write(&bad_path, PLUGIN).unwrap();
    fs::write(source.join("duplicate.json"), PLUGIN).unwrap();
    let duplicate = check_catalog(&source, url).unwrap_err();
    assert!(duplicate.0.contains("io.github.sovra.weekly-planning"));
    assert!(duplicate.0.contains("is repeated"));
    assert_eq!(fs::read_dir(&source).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn plugin_storage_refuses_symlinked_index_and_packages() {
    use sovra_core::plugins::{PluginEntry, PluginStore};
    let fixture = Fixture::new();
    let outside = fixture.0.join("outside.json");
    fs::write(&outside, PLUGIN).unwrap();
    let directory = fixture.0.join("plugins");
    fs::create_dir(&directory).unwrap();
    let store = LocalPluginStore::new(directory.clone());
    let entry =
        PluginEntry::for_package("https://example.com/package.json".into(), PLUGIN).unwrap();
    std::os::unix::fs::symlink(&outside, directory.join(format!("{}.json", entry.sha256))).unwrap();
    assert!(store.package(&entry).is_err());
    assert!(store.cache(&entry, PLUGIN).is_err());
    std::os::unix::fs::symlink(&outside, directory.join("plugins.json")).unwrap();
    assert!(store.load().is_err());
    assert!(
        store
            .save(&sovra_core::plugins::PluginLibrary::default())
            .is_err()
    );
    assert_eq!(fs::read(outside).unwrap(), PLUGIN);
}

#[test]
fn personal_skills_cannot_claim_plugin_ownership_and_hide_their_edit_controls() {
    let fixture = Fixture::new();
    let store = LocalSkillStore::new(fixture.0.join("skills"));
    let forged = SOURCE.replace("author: Test", "sovra-plugin: io.example.forged");
    assert!(format::decode(&forged, "weekly-review").is_err());
    let mut skill = format::decode(SOURCE, "weekly-review").unwrap();
    skill.metadata.insert(
        sovra_core::plugins::PLUGIN_METADATA_KEY.into(),
        "io.example.forged".into(),
    );
    assert!(store.save(&skill).is_err());
    assert_eq!(store.list().unwrap().skills, [] as [Skill; 0]);
}

#[test]
fn repository_skill_files_decode_without_their_tool_grants() {
    use sovra_core::plugins::PluginStore as _;
    let store = LocalPluginStore::new(std::env::temp_dir().join("unused-plugin-store"));
    let content = "---\nname: pdf\ndescription: >\n  Read and fill\n  PDF forms.\nallowed-tools: Bash\n---\n\nUse the form.\n";
    let skill = store.read_skill_file(Some("pdf"), content).unwrap();
    assert_eq!(skill.name, "pdf");
    assert_eq!(skill.description, "Read and fill PDF forms.");
    assert_eq!(skill.instructions, "Use the form.");
    // A root SKILL.md has no folder, so its own name is used.
    assert_eq!(store.read_skill_file(None, content).unwrap().name, "pdf");
    assert!(store.read_skill_file(Some("other"), content).is_err());
    assert!(store.read_skill_file(None, "no frontmatter").is_err());
}

#[test]
fn unused_plugin_packages_are_deleted_and_everything_else_is_left_alone() {
    use sovra_core::plugins::{
        InstalledPlugin, PluginEntry, PluginLibrary, PluginPackage, PluginStore,
    };
    let fixture = Fixture::new();
    let directory = fixture.0.join("plugins");
    let store = LocalPluginStore::new(directory.clone());
    let kept = PluginEntry::for_package("https://example.com/kept.json".into(), PLUGIN).unwrap();
    let mut other: PluginPackage = serde_json::from_slice(PLUGIN).unwrap();
    other.version = "1.0.1".into();
    let other_bytes = serde_json::to_vec(&other).unwrap();
    let replaced =
        PluginEntry::for_package("https://example.com/old.json".into(), &other_bytes).unwrap();
    store.cache(&kept, PLUGIN).unwrap();
    store.cache(&replaced, &other_bytes).unwrap();
    let library = PluginLibrary {
        catalog_url: None,
        installed: vec![InstalledPlugin {
            entry: kept.clone(),
            catalog_url: "https://example.com/catalog.json".into(),
            enabled: true,
            github: None,
        }],
    };
    store.save(&library).unwrap();
    fs::write(directory.join("notes.txt"), "mine").unwrap();
    let lookalike = directory.join(format!("{}.json", "a".repeat(64)));
    fs::create_dir(&lookalike).unwrap();

    assert_eq!(store.remove_unused(&library).unwrap(), 1);
    assert!(store.package(&kept).is_ok());
    assert!(store.package(&replaced).is_err());
    assert_eq!(store.load().unwrap(), library);
    assert!(directory.join("notes.txt").exists());
    assert!(lookalike.is_dir(), "only regular package files are deleted");
    assert_eq!(store.remove_unused(&library).unwrap(), 0);
    assert_eq!(store.remove_unused(&PluginLibrary::default()).unwrap(), 1);
    assert!(directory.join("plugins.json").exists());
}

#[test]
fn the_plugin_template_is_added_with_every_skill_and_file() {
    let template = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugin-template");
    let check = check_repository(&template, "Sovra-AI/sovra-plugin-template").unwrap();
    assert_eq!(check.left_out, [] as [String; 0]);
    let package = &check.package;
    assert_eq!(package.title, "Sovra-AI/sovra-plugin-template");
    assert_eq!(package.id, "io.github.sovra-ai.sovra-plugin-template");
    assert!(
        package
            .description
            .starts_with("Helpers for writing kind notes.")
    );
    assert_eq!(package.services, []);
    let [skill] = package.skills.as_slice() else {
        panic!("one example skill: {:?}", package.skills);
    };
    assert_eq!(
        (skill.name.as_str(), skill.title.as_str()),
        ("thank-you-note", "Write a thank-you note")
    );
    assert_eq!(
        skill.resources.keys().collect::<Vec<_>>(),
        ["tone-guide.md"],
        "the README, license and workflow are not part of any skill"
    );
    let listed = package.skills(true).remove(0);
    assert!(listed.instructions.contains("tone-guide.md"));
    assert_eq!(
        listed.friendly_card_copy().example_display,
        "Example: “Help me thank my neighbour for watering my plants”",
        "the Skills page shows the author's example, not the command name"
    );
}

/// Copies `from` into `to` without what GitHub never serves for this repository:
/// `.git` and the build and worktree folders `.gitignore` excludes.
fn copy_as_pushed(from: &Path, to: &Path, top_level: bool) {
    fs::create_dir_all(to).unwrap();
    for item in fs::read_dir(from).unwrap() {
        let item = item.unwrap();
        let name = item.file_name();
        let kind = item.file_type().unwrap();
        if top_level && (name == ".git" || name == "target") {
            continue;
        }
        if kind.is_dir() {
            if name == "worktrees" && from.ends_with(".claude") {
                continue;
            }
            copy_as_pushed(&item.path(), &to.join(&name), false);
        } else if kind.is_file() {
            fs::copy(item.path(), to.join(&name)).unwrap();
        }
    }
}

#[test]
fn this_repository_is_added_as_a_code_plugin_with_its_date_tools() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture = Fixture::new();
    let checkout = fixture.0.join("SovraHarness");
    copy_as_pushed(&root, &checkout, true);
    let check = check_repository(&checkout, "Sovra-AI/SovraHarness").unwrap();
    assert_eq!(check.left_out, [] as [String; 0]);
    let package = &check.package;
    assert_eq!(package.api_version, sovra_core::plugins::CODE_API_VERSION);
    assert!(
        package.description.ends_with("In Sovra it also adds exact date tools."),
        "plugin.json describes the skills and the tools together"
    );
    let code = package.code.as_ref().expect("the repository brings code");
    let tools: Vec<_> = code.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(tools, ["days_between", "weekday", "add_days"]);
    assert_eq!(
        code.network,
        [] as [String; 0],
        "the date helper needs no internet"
    );
    assert_eq!(
        code.module_bytes().unwrap(),
        fs::read(root.join("plugin.wasm")).unwrap()
    );
}

#[test]
fn this_repository_is_added_as_a_plugin_with_every_skill() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture = Fixture::new();
    let checkout = fixture.0.join("SovraHarness");
    copy_as_pushed(&root, &checkout, true);
    let check = check_repository(&checkout, "Sovra-AI/SovraHarness").unwrap();
    assert_eq!(check.left_out, [] as [String; 0]);
    let package = &check.package;
    assert_eq!(package.id, "io.github.sovra-ai.sovraharness");
    assert!(
        package
            .description
            .starts_with("Helpers for working on Sovra itself")
    );
    assert_eq!(package.services, []);
    let names: Vec<_> = package.skills.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "thank-you-note",
            "add-a-sovra-feature",
            "change-sovra-screens",
            "finish-a-sovra-change",
            "fix-a-sovra-bug",
            "make-a-sovra-plugin",
        ],
        "the template's example skill comes along; skills/ holds this repository's own"
    );
    let plugin_maker = &package.skills[5];
    assert_eq!(
        plugin_maker.resources.keys().collect::<Vec<_>>(),
        ["plugin-rules.md"]
    );
    for skill in package.skills(true) {
        assert!(
            skill
                .friendly_card_copy()
                .example_display
                .starts_with("Example: “"),
            "{} shows its author's example on the Skills page",
            skill.title
        );
    }
}

#[test]
fn a_repository_check_names_what_sovra_would_leave_out() {
    let fixture = Fixture::new();
    let repository = fixture.0.join("repository");
    let good = repository.join("skills/notes");
    let wrong = repository.join("skills/letters");
    fs::create_dir_all(&good).unwrap();
    fs::create_dir_all(&wrong).unwrap();
    fs::write(
        good.join("SKILL.md"),
        "---\nname: notes\ndescription: Take notes.\n---\nWrite tidy notes.\n",
    )
    .unwrap();
    fs::write(good.join("photo.png"), [0x89, b'P', b'N', b'G', 0xff, 0xfe]).unwrap();
    fs::write(
        wrong.join("SKILL.md"),
        "---\nname: letter-writer\ndescription: Write letters.\n---\nWrite.\n",
    )
    .unwrap();
    fs::create_dir_all(repository.join(".git")).unwrap();
    fs::write(repository.join(".git/SKILL.md"), "not a skill").unwrap();
    fs::write(
        repository.join(".mcp.json"),
        r#"{"mcpServers":{"docs":{"url":"https://docs.example/mcp"},"local":{"command":"npx"}}}"#,
    )
    .unwrap();

    let check = check_repository(&repository, "owner/repo").unwrap();
    assert_eq!(check.package.skills.len(), 1);
    assert_eq!(
        check.package.services.len(),
        1,
        "only the web connection is suggested"
    );
    assert_eq!(check.left_out.len(), 2, "{:?}", check.left_out);
    assert!(
        check
            .left_out
            .iter()
            .any(|reason| reason.starts_with("letters ("))
    );
    assert!(
        check
            .left_out
            .iter()
            .any(|reason| reason.starts_with("notes: 1 file"))
    );

    fs::remove_dir_all(&good).unwrap();
    fs::remove_file(repository.join(".mcp.json")).unwrap();
    assert!(
        check_repository(&repository, "owner/repo").is_err(),
        "a repository with nothing usable is refused, as in the app"
    );
    assert!(check_repository(&repository, "not a repository").is_err());
}

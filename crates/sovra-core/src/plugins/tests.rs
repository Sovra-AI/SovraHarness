use super::*;

mod schema;

const CATALOG: &[u8] = include_bytes!("../../../../examples/plugins/catalog.json");
const PACKAGE: &[u8] =
    include_bytes!("../../../../examples/plugins/packages/weekly-planning-1.0.0.json");

#[test]
fn published_example_verifies_and_projects_usable_skills_without_permissions() {
    let catalog = PluginCatalog::parse(CATALOG).unwrap();
    let entry = &catalog.plugins[0];
    let package = entry.verify_package(PACKAGE).unwrap();
    let skill = package.skills(true).remove(0);
    assert!(skill.name.starts_with("plugin-"));
    let prompt = format!("/{} my chores", skill.name);
    assert_eq!(skill.tools, [] as [String; 0]);
    assert_eq!(skill.aliases, [] as [String; 0]);
    assert!(!skill.builtin);
    let catalog = SkillCatalog::new(vec![skill]).unwrap();
    assert!(catalog.resolve(&prompt).unwrap().is_some());
    let disabled = SkillCatalog::new(package.skills(false)).unwrap();
    assert!(disabled.resolve(&prompt).is_err());
    assert_eq!(disabled.search(""), [] as [&Skill; 0]);
}

#[test]
fn transport_integrity_identity_and_schema_are_checked_before_admission() {
    let catalog = PluginCatalog::parse(CATALOG).unwrap();
    let mut entry = catalog.plugins[0].clone();
    assert!(entry.verify_package(b"wrong download").is_err());
    assert!(
        entry
            .verify_package(&vec![b' '; MAX_PLUGIN_BYTES + 1])
            .is_err()
    );
    for replacement in [
        ("weekly-planning", "different-plugin"),
        ("1.0.0", "2.0.0"),
        ("\"schema_version\": 1", "\"schema_version\": 99"),
        ("\"services\": []", "\"command\": \"run-me\""),
        ("\"name\": \"plan\"", "\"name\": \"../escape\""),
    ] {
        let changed = String::from_utf8(PACKAGE.to_vec())
            .unwrap()
            .replace(replacement.0, replacement.1);
        entry.sha256 = sha256_hex(changed.as_bytes());
        assert!(
            entry.verify_package(changed.as_bytes()).is_err(),
            "{replacement:?}"
        );
    }
}

#[test]
fn catalogs_reject_duplicate_ids_unknown_fields_and_unbounded_or_unsafe_data() {
    let mut catalog = PluginCatalog::parse(CATALOG).unwrap();
    catalog.plugins.push(catalog.plugins[0].clone());
    assert!(PluginCatalog::parse(&serde_json::to_vec(&catalog).unwrap()).is_err());
    assert!(PluginCatalog::parse(&vec![b' '; MAX_CATALOG_BYTES + 1]).is_err());
    assert!(PluginCatalog::parse(br#"{"schema_version":1,"plugins":[],"command":"x"}"#).is_err());
    for url in [
        "http://example.com/catalog.json",
        "file:///tmp/catalog.json",
        "https://user:password@example.com/p",
        "https://example.com/p?token=secret",
        "https://example.com/p#fragment",
    ] {
        assert!(check_download_url(url).is_err(), "{url}");
    }
    assert!(check_download_url("https://example.github.io/plugins/catalog.json").is_ok());
    assert!(check_download_url("http://127.0.0.1:8080/catalog.json").is_ok());
}

#[test]
fn library_validates_saved_data_and_state_keeps_installed_plugins_on_failure() {
    let entry = PluginCatalog::parse(CATALOG).unwrap().plugins.remove(0);
    let installed = InstalledPlugin {
        entry,
        catalog_url: "https://example.github.io/plugins/catalog.json".into(),
        enabled: true,
        github: None,
    };
    let mut library = PluginLibrary {
        catalog_url: None,
        installed: vec![installed.clone()],
    };
    library.validate().unwrap();
    let mut state = PluginState::default();
    state.apply(&PluginEvent::Loaded(library.clone()));
    state.apply(&PluginEvent::Busy);
    assert!(state.busy);
    state.apply(&PluginEvent::Error("Download failed".into()));
    assert!(!state.busy);
    assert_eq!(state.library, library);
    let catalog = PluginCatalog::parse(CATALOG).unwrap();
    state.apply(&PluginEvent::Catalog {
        url: "https://example.com/catalog.json".into(),
        catalog: catalog.clone(),
    });
    assert_eq!(state.catalog, Some(catalog));
    assert_eq!(state.catalog_url, "https://example.com/catalog.json");
    library.installed.push(installed);
    assert!(library.validate().is_err());
}

#[test]
fn updates_preserve_publisher_identity_and_immutable_versions() {
    let entry = PluginCatalog::parse(CATALOG).unwrap().plugins.remove(0);
    let installed = InstalledPlugin {
        entry: entry.clone(),
        catalog_url: "https://example.com/catalog.json".into(),
        enabled: true,
        github: None,
    };
    assert!(check_update(&installed, &entry, &installed.catalog_url).is_ok());
    assert!(check_update(&installed, &entry, "https://other.example/catalog.json").is_err());
    let mut candidate = entry.clone();
    candidate.version = "0.9.0".into();
    assert!(check_update(&installed, &candidate, &installed.catalog_url).is_err());
    candidate.version = "1.0.0".into();
    candidate.sha256 = "0".repeat(64);
    assert!(check_update(&installed, &candidate, &installed.catalog_url).is_err());
    candidate.version = "1.0.1".into();
    assert!(check_update(&installed, &candidate, &installed.catalog_url).is_ok());
    candidate.version = "999999999999999.0.0".into();
    assert!(candidate.validate().is_err());
    let built = PluginEntry::for_package(entry.package_url.clone(), PACKAGE).unwrap();
    assert_eq!(built, entry);
    let mut incompatible: PluginPackage = serde_json::from_slice(PACKAGE).unwrap();
    incompatible.api_version = 2;
    assert!(
        PluginEntry::for_package(
            entry.package_url,
            &serde_json::to_vec(&incompatible).unwrap()
        )
        .is_err()
    );
}

#[test]
fn only_package_files_no_installed_plugin_uses_are_collectable() {
    let entry = PluginCatalog::parse(CATALOG).unwrap().plugins.remove(0);
    let library = PluginLibrary {
        catalog_url: None,
        installed: vec![InstalledPlugin {
            entry: entry.clone(),
            catalog_url: "https://example.com/catalog.json".into(),
            enabled: true,
            github: None,
        }],
    };
    let kept = format!("{}.json", entry.sha256);
    assert_eq!(library.package_files(), BTreeSet::from([kept.clone()]));
    assert!(PluginLibrary::is_package_file(&kept));
    for not_a_package in [
        "plugins.json".to_owned(),
        ".write-1-2".into(),
        format!("{}.JSON", entry.sha256),
        format!("{}.json", entry.sha256.to_uppercase()),
        format!("{}.json", &entry.sha256[1..]),
        format!("../{}.json", &entry.sha256[3..]),
    ] {
        assert!(
            !PluginLibrary::is_package_file(&not_a_package),
            "{not_a_package}"
        );
    }
}

#[test]
fn removing_a_plugin_closes_its_open_panels_and_says_connections_stay() {
    let package: PluginPackage = serde_json::from_slice(PACKAGE).unwrap();
    let mut state = PluginState {
        details: Some(("io.example.gone".into(), package.clone())),
        update: Some(("io.example.gone".into(), PluginChanges::default())),
        busy: true,
        ..PluginState::default()
    };
    let mut other = state.clone();
    other.details = Some(("io.example.kept".into(), package));
    let removed = PluginEvent::Removed {
        id: "io.example.gone".into(),
        title: "Weekly planning".into(),
    };
    state.apply(&removed);
    assert_eq!(
        (state.details, state.update, state.busy),
        (None, None, false)
    );
    assert!(state.notice.contains("Removed Weekly planning"));
    assert!(state.notice.contains("Connections"));
    other.apply(&removed);
    assert!(
        other.details.is_some(),
        "another plugin's details stay open"
    );
}

#[test]
fn plugin_skill_cards_show_an_everyday_example_never_the_command_name() {
    let catalog = PluginCatalog::parse(CATALOG).unwrap();
    let mut package = catalog.plugins[0].verify_package(PACKAGE).unwrap();
    let plain = package.skills(true).remove(0).friendly_card_copy();
    assert!(
        !plain.example_display.contains("plugin-"),
        "{}",
        plain.example_display
    );
    assert!(plain.example_display.contains(&plain.friendly_title));
    assert!(plain.sample_draft.starts_with("/plugin-"));

    let long = format!("Plan   my\nweek {}", "x".repeat(300));
    package.skills[0] = package.skills[0].clone().with_example(&long);
    assert!(package.skills[0].example.starts_with("Plan my week x"));
    assert!(package.skills[0].example.len() <= 200);
    package.skills[0] = package.skills[0]
        .clone()
        .with_example("Plan my week around the school run");
    package.validate(&catalog.plugins[0]).unwrap();
    let skill = package.skills(true).remove(0);
    let copy = skill.friendly_card_copy();
    assert_eq!(
        copy.example_display,
        "Example: “Plan my week around the school run”"
    );
    assert_eq!(
        copy.sample_draft,
        format!("/{} Plan my week around the school run", skill.name)
    );

    package.skills[0].example = "Two\nlines".into();
    assert!(package.validate(&catalog.plugins[0]).is_err());
}

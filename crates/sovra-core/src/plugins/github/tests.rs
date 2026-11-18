use super::*;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const NEWER: &str = "89abcdef0123456789abcdef0123456789abcdef";

fn repository() -> GitHubRepository {
    GitHubRepository::parse("owner/skills").unwrap()
}

fn skill(name: &str, instructions: &str) -> PluginSkill {
    repository_skill(
        name.into(),
        &name.replace('-', " "),
        &format!("Helps with {name}."),
        instructions.into(),
    )
    .unwrap()
}

fn snapshot(commit: &str, skills: Vec<PluginSkill>) -> RepositorySnapshot {
    RepositorySnapshot {
        repository: repository(),
        commit: commit.into(),
        description: Some("Handy  skills\nfor writing.".into()),
        skills,
        manifests: Vec::new(),
        left_out: Vec::new(),
    }
}

fn found(plan: &RepositoryPlan) -> Vec<(Option<String>, String)> {
    plan.skills
        .iter()
        .map(|skill| (skill.folder.clone(), skill.path.clone()))
        .collect()
}

fn installed(snapshot: &RepositorySnapshot) -> InstalledPlugin {
    let (entry, _, pin) = snapshot.package().unwrap();
    InstalledPlugin {
        entry,
        catalog_url: snapshot.repository.web_url(),
        enabled: true,
        github: Some(pin),
    }
}

#[test]
fn every_common_way_of_writing_a_repository_address_is_understood() {
    for address in [
        "https://github.com/Owner-1/my.skills",
        "  github.com/Owner-1/my.skills/ ",
        "http://www.github.com/Owner-1/my.skills.git",
        "Owner-1/my.skills",
        "https://github.com/Owner-1/my.skills?tab=readme-ov-file#install",
    ] {
        let parsed = GitHubRepository::parse(address).unwrap();
        assert_eq!(parsed.owner, "Owner-1", "{address}");
        assert_eq!(parsed.name, "my.skills", "{address}");
    }
    assert_eq!(
        GitHubRepository::parse("Owner-1/my.skills")
            .unwrap()
            .plugin_id(),
        "io.github.owner-1.my-skills"
    );
    for address in [
        "",
        "owner",
        "https://gitlab.com/owner/repo",
        "https://github.com/owner/repo/tree/main/skills",
        "-owner/repo",
        "owner/..",
        "owner/re po",
    ] {
        assert!(GitHubRepository::parse(address).is_err(), "{address}");
    }
}

#[test]
fn plugin_ids_stay_valid_for_long_and_punctuated_names() {
    let long = GitHubRepository {
        owner: "a".repeat(39),
        name: format!("{}_.-x", "B".repeat(99)),
    };
    let id = long.plugin_id();
    assert!(id.len() <= 128);
    assert!(super::super::valid_plugin_id(&id), "{id}");
    let odd = GitHubRepository {
        owner: "x".into(),
        name: "___".into(),
    };
    assert_eq!(odd.plugin_id(), "io.github.x.repo");
}

#[test]
fn the_file_list_keeps_skills_and_manifests_and_skips_dependencies() {
    let tree = serde_json::json!({
        "sha": "tree",
        "truncated": false,
        "tree": [
            {"path": "README.md", "type": "blob", "size": 10},
            {"path": "skills", "type": "tree"},
            {"path": "skills/pdf/SKILL.md", "type": "blob"},
            {"path": "skills/pdf/scripts/run.py", "type": "blob"},
            {"path": ".claude/skills/pdf/SKILL.md", "type": "blob"},
            {"path": ".claude/skills/notes/SKILL.md", "type": "blob"},
            {"path": ".github/skills/ci/SKILL.md", "type": "blob"},
            {"path": "node_modules/pkg/skill/SKILL.md", "type": "blob"},
            {"path": "SKILL.md", "type": "blob"},
            {"path": ".mcp.json", "type": "blob"},
            {"path": ".claude-plugin/plugin.json", "type": "blob"},
            {"path": "docs/.mcp.json", "type": "blob"}
        ]
    });
    let plan = plan_repository(&serde_json::to_vec(&tree).unwrap()).unwrap();
    assert_eq!(
        found(&plan),
        vec![
            (Some("notes".into()), ".claude/skills/notes/SKILL.md".into()),
            (Some("pdf".into()), ".claude/skills/pdf/SKILL.md".into()),
            (None, "SKILL.md".into()),
        ]
    );
    assert_eq!(plan.manifests, [".claude-plugin/plugin.json", ".mcp.json"]);
}

#[test]
fn too_many_skills_truncated_lists_and_empty_repositories_are_explained() {
    let many: Vec<_> = (0..=MAX_GITHUB_SKILLS)
        .map(|n| serde_json::json!({"path": format!("s{n}/SKILL.md"), "type": "blob"}))
        .collect();
    let error = plan_repository(&serde_json::to_vec(&serde_json::json!({"tree": many})).unwrap())
        .unwrap_err();
    assert!(error.0.contains("33 skills"), "{error}");
    let truncated = serde_json::json!({"tree": [], "truncated": true});
    assert!(plan_repository(&serde_json::to_vec(&truncated).unwrap()).is_err());
    let empty = serde_json::json!({"tree": [{"path": "README.md", "type": "blob"}]});
    assert!(plan_repository(&serde_json::to_vec(&empty).unwrap()).is_err());
    assert!(plan_repository(b"not json").is_err());
}

#[test]
fn a_snapshot_becomes_a_verifiable_package_pinned_to_its_commit() {
    let mut snapshot = snapshot(COMMIT, vec![skill("pdf", "Read the PDF carefully.")]);
    snapshot.manifests = vec![
        (
            ".mcp.json".into(),
            serde_json::to_vec(&serde_json::json!({"mcpServers": {
                "docs": {"type": "http", "url": "https://docs.example/mcp"},
                "local": {"command": "npx", "args": ["server"]},
                "plain": {"url": "http://insecure.example/mcp"}
            }}))
            .unwrap(),
        ),
        (
            ".claude-plugin/plugin.json".into(),
            serde_json::to_vec(&serde_json::json!({
                "name": "pdf-tools",
                "description": "Work with PDF files.",
                "mcpServers": {"docs": {"url": "https://other.example/mcp"}}
            }))
            .unwrap(),
        ),
    ];
    let (entry, bytes, pin) = snapshot.package().unwrap();
    let package = entry.verify_package(&bytes).unwrap();
    assert_eq!(entry.id, "io.github.owner.skills");
    assert_eq!(entry.title, "owner/skills");
    assert_eq!(entry.description, "Work with PDF files.");
    assert_eq!(
        entry.package_url,
        format!("https://github.com/owner/skills/tree/{COMMIT}")
    );
    assert_eq!(pin.short_commit(), "0123456");
    // Program-starting and plain-HTTP servers are never suggested. Names stay unique,
    // and `.mcp.json` is read in the order it was passed, ahead of `plugin.json`.
    assert_eq!(
        package.services,
        [PluginService {
            name: "docs".into(),
            address: "https://docs.example/mcp".into(),
        }]
    );
    assert_eq!(package.skills(true)[0].tools, [] as [String; 0]);

    let plain = self::snapshot(COMMIT, vec![skill("pdf", "x")]);
    assert_eq!(
        plain.package().unwrap().0.description,
        "Handy skills for writing."
    );
    let mut bad = plain;
    bad.commit = "main".into();
    assert!(bad.package().is_err());
    // Each file is within its limit, but together they pass the 4 MB package limit.
    let oversized = self::snapshot(
        COMMIT,
        (0..2)
            .map(|n| {
                let mut big = skill(&format!("s{n}"), "x");
                for file in 0..20 {
                    big.resources.insert(
                        format!("part-{file}.md"),
                        "x".repeat(MAX_GITHUB_RESOURCE_BYTES),
                    );
                }
                big
            })
            .collect(),
    );
    assert!(oversized.package().unwrap_err().0.contains("4 MB"));
}

#[test]
fn skill_names_must_fit_plugin_rules() {
    assert!(repository_skill("ok-name".into(), "t", "d", "i".into()).is_ok());
    assert!(repository_skill("x".repeat(33), "t", "d", "i".into()).is_err());
    assert!(repository_skill("Bad Name".into(), "t", "d", "i".into()).is_err());
}

#[test]
fn saved_github_plugins_are_validated_and_replaced_only_by_newer_commits() {
    let first = installed(&snapshot(COMMIT, vec![skill("pdf", "one")]));
    let newer = installed(&snapshot(NEWER, vec![skill("pdf", "two")]));
    let library = super::super::PluginLibrary {
        catalog_url: None,
        installed: vec![first.clone()],
    };
    library.validate().unwrap();
    let serialized = serde_json::to_vec(&library).unwrap();
    assert_eq!(
        serde_json::from_slice::<super::super::PluginLibrary>(&serialized).unwrap(),
        library
    );

    check_replacement(&first, &newer).unwrap();
    assert!(
        check_replacement(&first, &first)
            .unwrap_err()
            .0
            .contains("up to date")
    );

    let mut elsewhere = newer.clone();
    elsewhere.github.as_mut().unwrap().repository.owner = "someone".into();
    assert!(check_replacement(&first, &elsewhere).is_err());
    let catalog = InstalledPlugin {
        github: None,
        catalog_url: "https://publisher.example/catalog.json".into(),
        ..first.clone()
    };
    assert!(check_replacement(&catalog, &newer).is_err());
    assert!(check_replacement(&first, &catalog).is_err());

    let mut tampered = first;
    tampered.catalog_url = "https://github.com/someone/skills".into();
    assert!(
        super::super::PluginLibrary {
            catalog_url: None,
            installed: vec![tampered],
        }
        .validate()
        .is_err()
    );
}

#[test]
fn changes_list_added_removed_and_edited_skills_and_connections() {
    let old = snapshot(
        COMMIT,
        vec![
            skill("keep", "same"),
            skill("edit", "old"),
            skill("gone", "x"),
        ],
    );
    let mut new = snapshot(
        NEWER,
        vec![
            skill("keep", "same"),
            skill("edit", "new"),
            skill("fresh", "y"),
        ],
    );
    new.manifests = vec![(
        ".mcp.json".into(),
        br#"{"mcpServers":{"docs":{"url":"https://docs.example/mcp"}}}"#.to_vec(),
    )];
    let package = |snapshot: &RepositorySnapshot| {
        let (entry, bytes, _) = snapshot.package().unwrap();
        entry.verify_package(&bytes).unwrap()
    };
    let changes = PluginChanges::between(&package(&old), &package(&new));
    assert_eq!(changes.added, ["fresh"]);
    assert_eq!(changes.removed, ["gone"]);
    assert_eq!(changes.changed, ["edit"]);
    assert_eq!(
        changes.connections_added,
        ["docs: https://docs.example/mcp"]
    );
    assert_eq!(changes.connections_removed, [] as [String; 0]);
    assert!(PluginChanges::between(&package(&old), &package(&old)).is_empty());
}

#[test]
fn update_events_drive_the_screen_state() {
    let first = installed(&snapshot(COMMIT, vec![skill("pdf", "one")]));
    let mut state = super::super::PluginState::default();
    state.apply(&super::super::PluginEvent::Loaded(
        super::super::PluginLibrary {
            catalog_url: None,
            installed: vec![first.clone()],
        },
    ));
    let id = first.entry.id;
    state.apply(&super::super::PluginEvent::UpdateReady {
        id: id.clone(),
        changes: PluginChanges::default(),
    });
    assert_eq!(state.update.as_ref().map(|(id, _)| id), Some(&id));
    state.apply(&super::super::PluginEvent::UpToDate { id: id.clone() });
    assert!(state.update.is_none());
    assert_eq!(state.notice, "owner/skills is already up to date.");
    state.apply(&super::super::PluginEvent::UpdateReady {
        id,
        changes: PluginChanges::default(),
    });
    state.apply(&super::super::PluginEvent::Loaded(
        super::super::PluginLibrary::default(),
    ));
    assert!(state.update.is_none());
}

#[test]
fn oversized_skills_are_left_out_by_name_instead_of_failing_the_repository() {
    let tree = serde_json::json!({"tree": [
        {"path": "skills/huge/SKILL.md", "type": "blob", "size": MAX_GITHUB_FILE_BYTES + 1},
        {"path": "skills/small/SKILL.md", "type": "blob", "size": 10},
        {"path": "SKILL.md", "type": "blob", "size": MAX_GITHUB_FILE_BYTES + 1}
    ]});
    let plan = plan_repository(&serde_json::to_vec(&tree).unwrap()).unwrap();
    assert_eq!(
        found(&plan),
        [(Some("small".into()), "skills/small/SKILL.md".into())]
    );
    assert_eq!(
        plan.left_out,
        [
            "the main skill (its instructions are larger than 64 KB)",
            "huge (its instructions are larger than 64 KB)",
        ]
    );
    let only_huge = serde_json::json!({"tree": [
        {"path": "skills/huge/SKILL.md", "type": "blob", "size": MAX_GITHUB_FILE_BYTES + 1}
    ]});
    let error = plan_repository(&serde_json::to_vec(&only_huge).unwrap()).unwrap_err();
    assert!(
        error.0.contains("None of this repository's skills"),
        "{error}"
    );
    let mut empty = snapshot(COMMIT, Vec::new());
    empty.left_out = vec!["template (bad name)".into()];
    assert!(
        empty
            .package()
            .unwrap_err()
            .0
            .contains("template (bad name)")
    );
}

#[test]
fn github_plugins_may_hold_more_skills_than_catalog_packages() {
    let many = snapshot(
        COMMIT,
        (0..20).map(|n| skill(&format!("s{n}"), "x")).collect(),
    );
    let (entry, bytes, _) = many.package().unwrap();
    assert_eq!(entry.verify_package(&bytes).unwrap().skills.len(), 20);
    let mut catalog_entry = entry;
    catalog_entry.package_url = "https://publisher.example/package.json".into();
    assert!(
        catalog_entry.verify_package(&bytes).is_err(),
        "catalog packages keep the 16 limit"
    );
}

#[test]
fn skill_folders_keep_their_text_files_within_budgets() {
    let mut items = vec![
        serde_json::json!({"path": "skills/pdf/SKILL.md", "type": "blob", "size": 100}),
        serde_json::json!({"path": "skills/pdf/reference.md", "type": "blob", "size": 100}),
        serde_json::json!({"path": "skills/pdf/scripts/fill.py", "type": "blob", "size": 100}),
        serde_json::json!({"path": "skills/pdf/logo.PNG", "type": "blob", "size": 100}),
        serde_json::json!({"path": "skills/pdf/.env", "type": "blob", "size": 10}),
        serde_json::json!({"path": "skills/pdf/node_modules/x/index.js", "type": "blob", "size": 10}),
        serde_json::json!({"path": "skills/pdf/huge.md", "type": "blob", "size": MAX_GITHUB_RESOURCE_BYTES + 1}),
        // A nested skill owns its own folder, not the skill around it.
        serde_json::json!({"path": "skills/pdf/forms/SKILL.md", "type": "blob", "size": 100}),
        serde_json::json!({"path": "skills/pdf/forms/fields.md", "type": "blob", "size": 100}),
        serde_json::json!({"path": "README.md", "type": "blob", "size": 100}),
    ];
    for n in 0..70 {
        items.push(serde_json::json!({"path": "skills/many/SKILL.md".to_string(), "type": "blob", "size": 10}));
        items.push(serde_json::json!({"path": format!("skills/many/notes/{n:02}.md"), "type": "blob", "size": 10}));
    }
    let plan =
        plan_repository(&serde_json::to_vec(&serde_json::json!({"tree": items})).unwrap()).unwrap();
    let files = |name: &str| {
        let skill = plan
            .skills
            .iter()
            .find(|skill| skill.folder.as_deref() == Some(name))
            .unwrap();
        (
            skill
                .resources
                .iter()
                .map(|(_, relative)| relative.as_str())
                .collect::<Vec<_>>(),
            skill.skipped_files,
        )
    };
    assert_eq!(files("pdf"), (vec!["reference.md", "scripts/fill.py"], 2));
    assert_eq!(files("forms"), (vec!["fields.md"], 0));
    let (many, skipped) = files("many");
    assert_eq!((many.len(), skipped), (64, 6));
    let pdf = plan
        .skills
        .iter()
        .find(|skill| skill.folder.as_deref() == Some("pdf"))
        .unwrap();
    assert_eq!(pdf.resources[0].0, "skills/pdf/reference.md");
}

#[test]
fn skill_files_are_carried_listed_and_read_back_only_for_github_plugins() {
    let mut with_files = skill("pdf", "Read reference.md first.");
    with_files
        .resources
        .insert("reference.md".into(), "Page sizes: A4, Letter.".into());
    with_files
        .resources
        .insert("scripts/fill.py".into(), "print('never run')".into());
    let snapshot = snapshot(COMMIT, vec![with_files]);
    let (entry, bytes, _) = snapshot.package().unwrap();
    let package = entry.verify_package(&bytes).unwrap();
    let projected = package.skills(true).remove(0);
    assert!(
        projected
            .instructions
            .ends_with("resource): reference.md, scripts/fill.py"),
        "{}",
        projected.instructions
    );
    assert_eq!(projected.tools, [] as [String; 0]);
    assert_eq!(
        package.resource(&projected.name, "./reference.md"),
        Some("Page sizes: A4, Letter.")
    );
    assert_eq!(package.resource(&projected.name, "missing.md"), None);
    assert_eq!(package.resource("plugin-other-pdf", "reference.md"), None);

    // The catalog format stays as published: a catalog package with files is refused.
    let mut catalog_entry = entry;
    catalog_entry.package_url = "https://publisher.example/package.json".into();
    assert!(catalog_entry.verify_package(&bytes).is_err());

    for unsafe_path in ["../escape.md", "/etc/passwd", ".hidden", "a//b", "a\\b"] {
        let mut bad = skill("pdf", "x");
        bad.resources.insert(unsafe_path.into(), "x".into());
        assert!(
            self::snapshot(COMMIT, vec![bad]).package().is_err(),
            "{unsafe_path}"
        );
    }
    assert!(valid_resource_path("scripts/fill.py"));
}

#[test]
fn large_files_in_one_skill_cannot_use_up_every_later_skills_budget() {
    let mut items = vec![
        serde_json::json!({"path": "skills/a-schemas/SKILL.md", "type": "blob", "size": 10}),
        serde_json::json!({"path": "skills/z-sheets/SKILL.md", "type": "blob", "size": 10}),
        serde_json::json!({"path": "skills/z-sheets/guide.md", "type": "blob", "size": 500}),
    ];
    for n in 0..13 {
        items.push(serde_json::json!({
            "path": format!("skills/a-schemas/schema-{n:02}.xsd"),
            "type": "blob",
            "size": MAX_GITHUB_RESOURCE_BYTES
        }));
    }
    let plan =
        plan_repository(&serde_json::to_vec(&serde_json::json!({"tree": items})).unwrap()).unwrap();
    let sheets = &plan.skills[1];
    assert_eq!(sheets.folder.as_deref(), Some("z-sheets"));
    assert_eq!(sheets.resources.len(), 1, "the short guide is kept");
    let schemas = &plan.skills[0];
    assert_eq!(schemas.resources.len() + schemas.skipped_files, 13);
    assert!(
        schemas.skipped_files > 0,
        "the 1.5 MB repository budget applies"
    );
}

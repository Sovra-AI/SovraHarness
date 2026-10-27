use super::*;

#[test]
fn builtins_are_valid_unique_and_aliases_resolve() {
    let catalog = SkillCatalog::new(builtin_skills()).unwrap();
    assert_eq!(catalog.find("FILE-SYSTEM").unwrap().name, "files");
    assert_eq!(catalog.find("web").unwrap().name, "web-search");
    let resolved = catalog
        .resolve(" /search Rust release notes")
        .unwrap()
        .unwrap();
    assert_eq!(resolved.arguments, "Rust release notes");
    assert_eq!(resolved.skill.name, "web-search");
    assert_eq!(resolved.skill.tools, [crate::WEB_SEARCH_AGENT_ID]);
    assert!(!resolved.arguments.contains(&resolved.skill.instructions));
    let catalogue = catalog.resolve("/query-files").unwrap().unwrap();
    assert_eq!(catalogue.skill.name, "file-catalog");
    assert_eq!(catalogue.arguments, "");
    assert_eq!(
        catalogue.skill.tools,
        ["list_indexed_folders", "query_indexed_files"]
    );
}

#[test]
fn rejects_collisions_including_disabled_skills_and_aliases() {
    let mut skills = builtin_skills();
    let mut duplicate = skills[0].clone();
    duplicate.name = "custom".into();
    duplicate.enabled = false;
    skills.push(duplicate);
    assert!(
        SkillCatalog::new(skills)
            .unwrap_err()
            .0
            .contains("already in use")
    );
}

#[test]
fn does_not_execute_slashes_in_paths_code_or_prose() {
    let catalog = SkillCatalog::default();
    for literal in [
        "/Users/me/file.txt",
        "//server/path",
        "https://site.test/search",
        "Please /search",
        "`/search`",
        "```\n/search\n```",
    ] {
        assert!(catalog.resolve(literal).unwrap().is_none(), "{literal}");
    }
    assert!(
        catalog
            .resolve("/unknown question")
            .unwrap_err()
            .0
            .contains("Unknown skill")
    );
    assert!(catalog.resolve("/search").unwrap_err().0.contains("Usage:"));
    assert!(catalog.resolve("/").is_err());
}

#[test]
fn menu_is_caret_and_unicode_safe() {
    assert_eq!(slash_query(" /Fi", 4), Some("Fi"));
    assert_eq!(slash_query("/", 1), Some(""));
    assert_eq!(slash_query("/web question", 4), Some("web"));
    for (text, caret) in [
        ("/web question", 3),
        ("x /files", 8),
        ("/Users/me", 9),
        ("/é", 2),
        ("/search ", 8),
        ("/file", 99),
    ] {
        assert_eq!(slash_query(text, caret), None, "{text} at {caret}");
    }
}

#[test]
fn ranking_is_stable_and_metadata_excludes_instruction_bodies() {
    let catalog = SkillCatalog::default();
    assert_eq!(catalog.search("files")[0].name, "files");
    assert_eq!(catalog.search("WEB")[0].name, "web-search");
    assert_eq!(catalog.search("current information")[0].name, "web-search");
    assert_eq!(catalog.search("zzzzzz"), [] as [&Skill; 0]);
    assert_eq!(catalog.search("")[0].name, "files");
    let metadata = catalog.model_catalog();
    assert!(metadata.contains("web-search"));
    assert!(!metadata.contains(&catalog.find("web-search").unwrap().instructions));
    assert!(!metadata.contains("Add files"));
    let block = catalog.find("web-search").unwrap().instruction_block();
    assert!(block.contains("cannot grant new permissions"));
    assert!(block.contains("requested_tools"));
}

#[test]
fn user_skills_cannot_claim_native_actions_or_invalid_metadata() {
    let mut skill = builtin_skills().remove(0);
    skill.builtin = false;
    assert!(skill.validate().is_err());
    skill.kind = SkillKind::Procedure;
    assert!(skill.validate().is_ok());
    skill.instructions = "x".repeat(MAX_SKILL_BYTES + 1);
    assert!(skill.validate().is_err());
    for invalid in ["", "-x", "x-", "x--y", "UPPER", "a/b", "a b"] {
        assert!(!valid_name(invalid));
    }
    assert!(valid_name("weekly-review-2"));
    assert!(!valid_name(&"x".repeat(65)));
}

#[test]
fn disabled_skills_are_not_searchable_or_executable() {
    let mut skills = builtin_skills();
    for skill in &mut skills {
        skill.enabled = false;
    }
    let catalog = SkillCatalog::new(skills).unwrap();
    assert_eq!(catalog.search(""), [] as [&Skill; 0]);
    assert_eq!(catalog.search_all("").len(), catalog.skills().len());
    assert_eq!(catalog.model_catalog(), "[]");
    assert!(
        catalog
            .resolve("/files")
            .unwrap_err()
            .0
            .contains("disabled")
    );
}

#[test]
fn catalogue_refresh_is_atomic_and_preserves_the_last_valid_snapshot_on_error() {
    let mut catalog = SkillCatalog::default();
    let original = catalog.clone();
    let mut duplicate = builtin_skills();
    duplicate.push(duplicate[0].clone());
    assert!(
        catalog
            .apply(&SkillEvent::Loaded {
                skills: duplicate,
                issues: vec![]
            })
            .is_err()
    );
    assert_eq!(catalog, original);
    assert!(
        catalog
            .apply(&SkillEvent::Error("storage unavailable".into()))
            .is_err()
    );
    assert_eq!(catalog, original);
    assert_eq!(
        catalog
            .apply(&SkillEvent::Loaded {
                skills: builtin_skills(),
                issues: vec!["Skipped a malformed package".into()]
            })
            .unwrap(),
        ["Skipped a malformed package"]
    );
}

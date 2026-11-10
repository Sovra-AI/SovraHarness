use super::*;

fn source() -> ArtifactSource {
    ArtifactSource {
        session_id: "s".into(),
        message_id: "m".into(),
        author: "Sovra".into(),
        shared_from: String::new(),
    }
}

#[test]
fn every_kind_has_one_row_in_order() {
    for (index, info) in KINDS.iter().enumerate() {
        assert_eq!(info.kind.info(), &KINDS[index]);
        assert_ne!(info.explanation, "");
    }
    assert_eq!(ArtifactKind::for_language("Mermaid"), ArtifactKind::Diagram);
    assert_eq!(ArtifactKind::for_language("html"), ArtifactKind::Webpage);
    assert_eq!(ArtifactKind::for_language("svg"), ArtifactKind::Picture);
    assert_eq!(ArtifactKind::for_language("python"), ArtifactKind::Code);
    assert_eq!(ArtifactKind::for_language(""), ArtifactKind::Note);
    assert_eq!(file_extension(ArtifactKind::Code, "Python"), "py");
    assert_eq!(file_extension(ArtifactKind::Table, "python"), "csv");
}

#[test]
fn pieces_cover_code_tables_and_checklists() {
    let reply = "Here you go:\n\n```python\nprint('hi')\n```\n\n| Day | Meal |\n|---|---|\n| Mon | Soup |\n\n- [ ] Milk\n- [x] Eggs\n\nDone.";
    let pieces = find_pieces(reply);
    let kinds: Vec<ArtifactKind> = pieces.iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        [
            ArtifactKind::Code,
            ArtifactKind::Table,
            ArtifactKind::Checklist
        ]
    );
    assert_eq!(pieces[0].content, "print('hi')");
    assert_eq!(pieces[0].language, "python");
    assert_eq!(
        &reply[pieces[1].span.clone()],
        "| Day | Meal |\n|---|---|\n| Mon | Soup |\n"
    );
    assert_eq!(pieces[2].content, "- [ ] Milk\n- [x] Eggs");
}

#[test]
fn saving_a_reply_keeps_the_main_block_or_the_whole_answer() {
    let code = "Shopping script:\n```python\nitems = ['milk', 'eggs', 'bread', 'tea', 'jam']\nfor item in items:\n    print(item)\n```";
    let draft = draft_from_reply(code, source(), "Reply");
    assert_eq!(draft.kind, ArtifactKind::Code);
    assert_eq!(draft.title, "Shopping script");
    assert!(draft.content.starts_with("items"));

    let letter = "# Letter to the council\n\nDear Sir or Madam,\n\nI am writing about the bins.\n\n```text\nRef 123\n```\nYours faithfully";
    let draft = draft_from_reply(letter, source(), "Reply");
    assert_eq!(draft.kind, ArtifactKind::Document);
    assert_eq!(draft.title, "Letter to the council");
    assert_eq!(draft.content, letter);

    let list = "Things for the trip:\n- [ ] Passport\n- [ ] Tickets\nHave fun and remember to rest well on the way there, the journey is long.";
    assert_eq!(
        draft_from_reply(list, source(), "Reply").kind,
        ArtifactKind::Checklist
    );
    let piece = draft_from_piece("mermaid", "graph TD; A-->B\n", source(), "Reply");
    assert_eq!(
        (piece.kind, piece.content.as_str()),
        (ArtifactKind::Diagram, "graph TD; A-->B")
    );
}

#[test]
fn edits_keep_versions_and_titles_stay_clean() {
    let draft = ArtifactDraft {
        title: "## **Plan**\nmore".into(),
        kind: ArtifactKind::Document,
        language: String::new(),
        content: "v1".into(),
        source: source(),
    };
    let mut artifact = Artifact::from_draft(artifact_id(Timestamp(5), 1), draft, Timestamp(5));
    assert_eq!(artifact.id, "art-5-1");
    assert_eq!(artifact.title, "Plan");
    artifact.edit("Plan B", "v2", Timestamp(9));
    artifact.edit("Plan C", "v2", Timestamp(10));
    assert_eq!(artifact.content, "v2");
    assert_eq!(artifact.title, "Plan C");
    assert_eq!(
        artifact.versions.len(),
        1,
        "title-only edits add no version"
    );
    assert_eq!(artifact.versions[0].content, "v1");
    assert_eq!(clean_title("  ", ArtifactKind::Table), "Untitled table");
    artifact.title = "Bills: 2026/27?".into();
    assert_eq!(artifact.file_name(), "Bills 2026 27.md");
}

#[test]
fn tables_export_as_csv_and_parse_from_either_form() {
    let artifact = Artifact::from_draft(
        "a".into(),
        ArtifactDraft {
            title: "Meals".into(),
            kind: ArtifactKind::Table,
            language: String::new(),
            content: "| Day | Meal |\n|---|---|\n| Mon | Soup, bread |".into(),
            source: source(),
        },
        Timestamp(1),
    );
    assert_eq!(artifact.export_text(), "Day,Meal\nMon,\"Soup, bread\"\n");
    let (headers, rows) = parse_markdown_table("a,b\n1,2").expect("csv");
    assert_eq!((headers.len(), rows.len()), (2, 1));
    assert!(parse_markdown_table("just words").is_none());
    assert_eq!(artifact.preview(), "Day | Meal");
}

#[test]
fn library_folds_events_filters_and_counts() {
    let make = |id: &str, kind, at| {
        Artifact::from_draft(
            id.into(),
            ArtifactDraft {
                title: id.into(),
                kind,
                language: String::new(),
                content: format!("content of {id}"),
                source: source(),
            },
            Timestamp(at),
        )
    };
    let mut library = ArtifactLibrary::default();
    library.apply(&ArtifactEvent::Loaded(vec![
        make("old", ArtifactKind::Code, 1),
        make("new", ArtifactKind::Document, 2),
    ]));
    assert_eq!(library.artifacts[0].id, "new");
    library.apply(&ArtifactEvent::Saved(make("newest", ArtifactKind::Code, 3)));
    assert_eq!(library.artifacts[0].id, "newest");
    assert_eq!(
        library.notice.as_deref(),
        Some("Saved “newest” to Artifacts")
    );
    let code = ArtifactFilter {
        kind: Some(ArtifactKind::Code),
        query: String::new(),
    };
    assert_eq!(library.filtered(&code).len(), 2);
    let search = ArtifactFilter {
        kind: None,
        query: "OF OLD".into(),
    };
    assert_eq!(library.filtered(&search).len(), 1);
    assert_eq!(
        library.kind_counts(),
        [(ArtifactKind::Document, 1), (ArtifactKind::Code, 2)]
    );
    library.apply(&ArtifactEvent::Deleted { id: "old".into() });
    assert!(library.get("old").is_none());
}

#[test]
fn browser_pages_escape_text_and_keep_html() {
    let mut artifact = Artifact::from_draft(
        "a".into(),
        ArtifactDraft {
            title: "<Flow>".into(),
            kind: ArtifactKind::Diagram,
            language: "mermaid".into(),
            content: "graph TD; A-->B".into(),
            source: source(),
        },
        Timestamp(1),
    );
    let page = browser_page(&artifact);
    assert!(page.contains("&lt;Flow&gt;"));
    assert!(page.contains("A--&gt;B"));
    assert!(page.contains("mermaid"));
    artifact.kind = ArtifactKind::Webpage;
    artifact.content = "<h1>Hi</h1>".into();
    assert_eq!(browser_page(&artifact), "<h1>Hi</h1>");
}

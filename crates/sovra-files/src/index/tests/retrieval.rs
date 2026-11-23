//! A granted folder is searched by the question, not by the file name.

use super::semantic::target;
use super::*;
use sovra_core::file_index::semantic::{EmbeddingVector, SemanticIndex, answer_from_index};

const LEVY: &str = "# Keeper note\n\nThe allotment levy ZX-ORCHID-991 is forty-two dinars.\n";
const SEAL: &str = "# Warehouse\n\nThe warehouse seal is QX-NEEDLE-17.\n";
const PARAPHRASE: &str = "What sum was reserved for the greenhouse plot?";

/// Maps the levy passage and its paraphrase onto one direction, and everything
/// else onto another. This stands in for a consented embedding model: the test
/// commits the vectors it returns through the shipped index.
fn embed(input: &[String]) -> Result<Vec<EmbeddingVector>, IndexError> {
    input.iter().map(|text| embed_text(text)).collect()
}

fn embed_text(text: &str) -> Result<EmbeddingVector, IndexError> {
    let lower = text.to_ascii_lowercase();
    let levy = lower.contains("allotment")
        || lower.contains("zx-orchid-991")
        || lower.contains("dinars")
        || lower.contains("keeper");
    let paraphrase =
        lower.contains("greenhouse") || lower.contains("reserved") || lower.contains("plot");
    EmbeddingVector::new(if levy || paraphrase {
        vec![1.0, 0.0]
    } else {
        vec![0.0, 1.0]
    })
}

fn query(text: &str) -> IndexQuery {
    IndexQuery {
        text: text.to_owned(),
        roots: vec![],
        path_prefix: None,
        limit: 8,
    }
}

fn joined(report: &sovra_core::file_index::semantic::SearchReport) -> String {
    report
        .passages
        .iter()
        .map(|passage| {
            format!(
                "{}:{}-{}:{}",
                passage.path, passage.start_line, passage.end_line, passage.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn granted_folder_answers_a_question_that_names_no_file() {
    let fixture = Fixture::new();
    fixture.write("aa-misc.txt", LEVY);
    fixture.write("bb-misc.txt", SEAL);
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();

    let asked = "Where is the allotment levy written?";
    assert!(!asked.contains("aa-misc"));
    assert!(!asked.contains("bb-misc"));
    let levy_terms = sovra_core::file_index::content_terms(LEVY);
    let paraphrase_terms = sovra_core::file_index::content_terms(PARAPHRASE);
    assert!(
        paraphrase_terms.iter().all(|term| !levy_terms
            .iter()
            .any(|body| body.eq_ignore_ascii_case(term))),
        "paraphrase must share no distinctive passage word"
    );

    let natural = answer_from_index(&store, &query(asked), &[]).unwrap();
    let natural_text = joined(&natural);
    assert!(natural_text.contains("forty-two dinars"), "{natural_text}");
    assert!(natural_text.contains("aa-misc.txt"));
    assert!(natural_text.contains("Keeper note"));
    assert!(natural.passages.iter().any(|passage| {
        passage.path == "aa-misc.txt"
            && passage.start_line >= 1
            && passage.end_line >= passage.start_line
    }));
    assert!(
        natural
            .notices
            .iter()
            .any(|notice| notice.contains("not fully vectorised"))
    );

    let lexical_paraphrase = answer_from_index(&store, &query(PARAPHRASE), &[]).unwrap();
    assert!(
        !joined(&lexical_paraphrase).contains("forty-two"),
        "lexical search must miss a paraphrase with no shared words"
    );

    let identifier = answer_from_index(&store, &query("ZX-ORCHID-991"), &[]).unwrap();
    assert!(joined(&identifier).contains("aa-misc.txt"));
    assert!(joined(&identifier).contains("ZX-ORCHID-991"));

    store.configure(&root.id, Some(&target())).unwrap();
    let setting = store.settings().unwrap().remove(0);
    let inputs = store.pending(&setting).unwrap();
    let vectors = embed(
        &inputs
            .iter()
            .map(|input| input.text.clone())
            .collect::<Vec<_>>(),
    )
    .expect("vectors");
    let orchid: Vec<_> = inputs
        .iter()
        .zip(vectors.iter())
        .filter(|(input, _)| input.text.contains("ZX-ORCHID-991"))
        .map(|(input, vector)| (input.clone(), vector.clone()))
        .collect();
    assert_ne!(orchid.len(), 0);
    let (partial_inputs, partial_vectors): (Vec<_>, Vec<_>) = orchid.into_iter().unzip();
    store
        .commit(&setting, &partial_inputs, &partial_vectors)
        .unwrap();
    let status = store.status(&setting).unwrap();
    assert!(status.embedded < status.total);

    let paraphrase_vector = embed(&[PARAPHRASE.to_owned()]).expect("vectors").remove(0);
    let hybrid = answer_from_index(
        &store,
        &query(PARAPHRASE),
        &[(&setting, &paraphrase_vector)],
    )
    .unwrap();
    let hybrid_text = joined(&hybrid);
    assert!(
        hybrid_text.contains("forty-two dinars"),
        "hybrid search must find the paraphrase: {hybrid_text}"
    );
    assert!(hybrid_text.contains("aa-misc.txt"));
    assert!(hybrid_text.contains("Keeper note"));
    assert!(
        hybrid
            .notices
            .iter()
            .any(|notice| notice.contains("not fully vectorised") && notice.contains("not proof"))
    );

    let partial_identifier = answer_from_index(
        &store,
        &query("QX-NEEDLE-17"),
        &[(&setting, &paraphrase_vector)],
    )
    .unwrap();
    assert!(
        joined(&partial_identifier).contains("QX-NEEDLE-17"),
        "a missing vector must not drop the word match"
    );
    assert!(
        partial_identifier
            .notices
            .iter()
            .any(|notice| notice.contains("not fully vectorised"))
    );

    let rest = store.pending(&setting).unwrap();
    let rest_vectors = embed(
        &rest
            .iter()
            .map(|input| input.text.clone())
            .collect::<Vec<_>>(),
    )
    .expect("vectors");
    store.commit(&setting, &rest, &rest_vectors).unwrap();
    assert_eq!(
        store.status(&setting).unwrap().embedded,
        store.status(&setting).unwrap().total
    );
    let complete = answer_from_index(
        &store,
        &query(PARAPHRASE),
        &[(&setting, &paraphrase_vector)],
    )
    .unwrap();
    assert!(joined(&complete).contains("forty-two dinars"));
    assert!(
        !complete
            .notices
            .iter()
            .any(|notice| notice.contains("not fully vectorised"))
    );

    store.remove(&root.id).unwrap();
    let removed = answer_from_index(&store, &query(asked), &[]).unwrap();
    let removed_text = joined(&removed);
    assert!(!removed_text.contains("forty-two"));
    assert!(!removed_text.contains("QX-NEEDLE-17"));
    assert_eq!(removed.passages.len(), 0);
}

#[test]
fn saved_answer_keeps_the_wording_that_was_read() {
    use sovra_core::file_index::{IndexEvent, IndexState};

    let fixture = Fixture::new();
    fixture.write(
        "keeper-note.txt",
        "# Keeper note\n\nThe allotment levy ZX-ORCHID-991 is forty-two dinars.\n",
    );
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();

    let found = store.search(&query("ZX-ORCHID-991")).unwrap();
    let passage = found
        .into_iter()
        .find(|passage| passage.path == "keeper-note.txt" && passage.text.contains("forty-two"))
        .expect("granted folder returns the passage that was read");
    let mut state = IndexState::default();
    state.apply(&IndexEvent::Roots(vec![root.clone()]));
    let request_id = state.begin_search();
    state.apply(&IndexEvent::Results {
        request_id,
        passages: vec![passage.clone()],
    });
    let saved = store
        .save_answer(&state.passages[0].reference)
        .expect("the shown passage saves");
    assert_eq!(
        store.save_answer(&state.passages[0].reference).unwrap(),
        saved
    );
    state.apply(&IndexEvent::AnswerSaved(saved.clone()));
    let reading = saved.reread();
    assert_eq!(reading.file, passage.path);
    assert_eq!(reading.wording, passage.text);
    assert_eq!(reading.read_on, passage.observed_at);
    assert_eq!(reading.start_line, passage.start_line);
    assert_eq!(reading.end_line, passage.end_line);
    assert_eq!(saved.reference(), &passage.reference);

    fixture.write(
        "keeper-note.txt",
        "# Keeper note\n\nThe allotment levy ZX-ORCHID-991 is ninety-nine dinars.\n",
    );
    store.scan(&root.id, &Observer::default()).unwrap();
    let updated = store
        .search(&query("ZX-ORCHID-991"))
        .unwrap()
        .into_iter()
        .find(|passage| passage.path == "keeper-note.txt")
        .expect("a new search sees the edited file");
    assert!(updated.text.contains("ninety-nine"), "{}", updated.text);
    assert!(
        !updated.text.contains("forty-two"),
        "current search must not keep the old wording: {}",
        updated.text
    );
    assert_ne!(updated.reference.version_id, passage.reference.version_id);

    let still = saved.reread();
    assert_eq!(still.file, reading.file);
    assert_eq!(still.wording, reading.wording);
    assert_eq!(still.read_on, reading.read_on);
    assert_eq!(
        (still.start_line, still.end_line),
        (reading.start_line, reading.end_line)
    );
    assert!(still.wording.contains("forty-two"));
    let request_id = state.begin_search();
    state.apply(&IndexEvent::Results {
        request_id,
        passages: vec![updated],
    });
    assert_eq!(state.saved_answers[0].reread().wording, reading.wording);
    drop(store);
    let reopened = fixture.store();
    let stored = reopened.saved_answers(None).unwrap();
    assert_eq!(stored.answers, vec![saved]);
    reopened.remove(&root.id).unwrap();
    assert_eq!(reopened.saved_answers(None).unwrap().answers, Vec::new());
}

#[test]
fn folder_words_expand_inflections_but_identifiers_keep_literal_tokens() {
    let fixture = Fixture::new();
    fixture.write(
        "grove.txt",
        "An orange contains a nutrient. The serial is QX-DYES-21.",
    );
    fixture.write("yard.txt", "Cranes are feeding. The serial is QX-DYE-21.");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    for (term, path) in [
        ("oranges nutrients", "grove.txt"),
        ("crane", "yard.txt"),
        ("QX-DYE-21", "yard.txt"),
    ] {
        let hits = search(&store, term);
        assert_eq!(hits.len(), 1, "{term}");
        assert_eq!(hits[0].path, path);
    }
    let saved = search(&store, "orange")[0].reference.clone();
    // Simulate an existing v11 installation containing real saved chunks.
    let db = store.connect().unwrap();
    db.execute_batch("DROP TRIGGER word_forms_insert; DROP TRIGGER word_forms_delete; DROP TABLE index_word_forms; PRAGMA user_version=11;").unwrap();
    drop(db);
    let reopened = fixture.store();
    assert_eq!(search(&reopened, "oranges")[0].reference, saved);
    reopened.remove(&root.id).unwrap();
    assert_eq!(search(&reopened, "oranges"), Vec::new());
}

#[test]
fn selected_language_postings_backfill_then_follow_new_files_and_grant_removal() {
    use sovra_core::knowledge_analysis::Language;
    let fixture = Fixture::new();
    let forms = [
        (Language::English, "orange", "oranges"),
        (Language::French, "maison", "maisons"),
        (Language::German, "Garten", "Gärten"),
        (Language::Spanish, "gato", "gatos"),
        (Language::Italian, "ragazzo", "ragazzi"),
        (Language::Portuguese, "livro", "livros"),
        (Language::Dutch, "appel", "appels"),
        (Language::Arabic, "كتاب", "الكتاب"),
    ];
    for (lang, first, second) in forms {
        assert_eq!(lang.stem_text(first), lang.stem_text(second), "{lang:?}");
        fixture.write(&format!("{}.txt", lang.name()), second);
    }
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    for (language, first, _) in forms {
        let query = IndexQuery {
            text: first.into(),
            roots: vec![root.id.clone()],
            path_prefix: None,
            limit: 4,
        };
        let found = store.search_language(&query, language).unwrap();
        assert!(
            found
                .iter()
                .any(|p| p.path == format!("{}.txt", language.name())),
            "{language:?}: {found:?}"
        );
    }
    fixture.write("additional.txt", "Les nouvelles maisons.");
    store.scan(&root.id, &Observer::default()).unwrap();
    let query = IndexQuery {
        text: "maison".into(),
        roots: vec![],
        path_prefix: None,
        limit: 4,
    };
    assert_eq!(
        fixture
            .store()
            .search_language(&query, Language::French)
            .unwrap()
            .len(),
        2
    );
    store.remove(&root.id).unwrap();
    assert_eq!(
        store.search_language(&query, Language::French).unwrap(),
        Vec::new()
    );
}
#[test]
fn actual_mask_pixels_reopen_and_follow_the_source_grant() {
    use sovra_core::knowledge_analysis::mask::Operation;
    let fixture = Fixture::new();
    let raster = image::GrayImage::from_fn(5, 5, |x, y| {
        image::Luma([if (1..4).contains(&x) && (1..4).contains(&y) {
            0
        } else {
            255
        }])
    });
    raster
        .save(fixture.source().join("silhouette.png"))
        .unwrap();
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let hit = search(&store, "silhouette").remove(0);
    let mask = fixture
        .store()
        .binary_mask(&hit.reference)
        .unwrap()
        .unwrap();
    assert_eq!(mask.foreground(), 9);
    assert_eq!(mask.apply(Operation::Erode, 1).unwrap().foreground(), 1);
    assert_eq!(mask.apply(Operation::Dilate, 1).unwrap().foreground(), 25);
    store.remove(&root.id).unwrap();
    assert!(store.binary_mask(&hit.reference).unwrap().is_none());
}

#[test]
fn a_matching_late_table_row_keeps_its_header_and_complete_source_order() {
    use std::fmt::Write as _;
    let fixture = Fixture::new();
    let mut table = String::from("timestamp,value,unit,instrument\n");
    for i in 0..401 {
        writeln!(table, "2026-11-23,{i},g,scale{i}").unwrap();
    }
    fixture.write("neutral.csv", &table);
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let hit = search(&store, "scale400").remove(0);
    assert!(hit.reference.ordinal > 0);
    assert_eq!(
        store.table_wording(&hit.reference).unwrap().as_deref(),
        Some(table.as_str())
    );
    store.remove(&root.id).unwrap();
    assert_eq!(store.table_wording(&hit.reference).unwrap(), None);
}

#[test]
fn saved_answer_pages_resume_after_newer_saves_without_duplicates() {
    let fixture = Fixture::new();
    for i in 0..23 {
        fixture.write(&format!("quote{i}.txt"), &format!("Reference citation{i}"));
    }
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    for i in 0..22 {
        store
            .save_answer(&search(&store, &format!("citation{i}"))[0].reference)
            .unwrap();
    }
    let first = store.saved_answers(None).unwrap();
    assert_eq!(first.answers.len(), 20);
    store
        .save_answer(&search(&store, "citation22")[0].reference)
        .unwrap();
    let next = fixture.store().saved_answers(first.next_before).unwrap();
    assert_eq!(
        next.answers
            .iter()
            .map(sovra_core::file_index::DatedAnswer::file)
            .collect::<Vec<_>>(),
        ["quote1.txt", "quote0.txt"]
    );
    assert!(next.next_before.is_none());
}

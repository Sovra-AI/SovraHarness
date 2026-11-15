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
    let saved = state.save_shown_answer(0).expect("the shown passage saves");
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
}

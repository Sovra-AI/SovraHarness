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

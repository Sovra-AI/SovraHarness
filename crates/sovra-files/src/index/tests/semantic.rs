use super::*;
use sovra_core::file_index::semantic::{EmbeddingTarget, EmbeddingVector, SemanticIndex};
use sovra_core::models::{ModelConnection, ModelInfo, ProviderKind};

pub(super) fn target() -> EmbeddingTarget {
    EmbeddingTarget::for_embeddings(
        &ModelConnection::new("local", ProviderKind::Ollama),
        &ModelInfo {
            id: "embed".into(),
            output_modalities: vec!["embeddings".into()],
            ..ModelInfo::default()
        },
    )
    .unwrap()
}

#[test]
fn semantic_vectors_keep_current_citations_and_revoked_work_cannot_reappear() {
    let fixture = Fixture::new();
    fixture.write("notes/travel.txt", "A passenger jet crosses the Atlantic.");
    fixture.write("notes/cooking.txt", "Simmer the soup in a saucepan.");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    assert_eq!(store.settings().unwrap(), vec![]);
    store.configure(&root.id, Some(&target())).unwrap();
    let setting = store.settings().unwrap().remove(0);
    let inputs = store.pending(&setting).unwrap();
    let vectors: Vec<_> = inputs
        .iter()
        .map(|i| {
            EmbeddingVector::new(if i.text.contains("jet") {
                vec![1., 0.]
            } else {
                vec![0., 1.]
            })
            .unwrap()
        })
        .collect();
    assert!(store.commit(&setting, &inputs, &vectors[..1]).is_err());
    assert_eq!(store.status(&setting).unwrap().embedded, 0);
    store.commit(&setting, &inputs, &vectors).unwrap();
    assert!(store.pending(&setting).unwrap().is_empty());
    assert_eq!(fixture.store().status(&setting).unwrap().embedded, 2);
    let query = IndexQuery {
        text: "aircraft flight".into(),
        roots: vec![root.id.clone()],
        path_prefix: Some("notes/".into()),
        limit: 1,
    };
    assert_eq!(store.search(&query).unwrap(), vec![]);
    let vector = EmbeddingVector::new(vec![4., 0.]).unwrap();
    let result = store.nearest(&setting, &query, &vector).unwrap();
    assert_eq!(result[0].path, "notes/travel.txt");
    assert_eq!(store.read(&result[0].reference).unwrap(), result[0]);
    assert!(
        store
            .nearest(&setting, &query, &EmbeddingVector::new(vec![1.]).unwrap())
            .is_err()
    );
    let mut other = query.clone();
    other.roots = vec!["unapproved".into()];
    assert_eq!(store.nearest(&setting, &other, &vector).unwrap(), vec![]);
    other.roots = vec![];
    other.path_prefix = Some("elsewhere/".into());
    assert_eq!(store.nearest(&setting, &other, &vector).unwrap(), vec![]);
    fixture.write("notes/travel.txt", "Train tickets cost ten dollars.");
    store.scan(&root.id, &Observer::default()).unwrap();
    store.commit(&setting, &inputs, &vectors).unwrap();
    assert!(store.read(&result[0].reference).is_err());
    assert_eq!(store.pending(&setting).unwrap().len(), 1);
    assert_eq!(store.status(&setting).unwrap().embedded, 1);
    store.configure(&root.id, None).unwrap();
    assert!(store.commit(&setting, &inputs, &vectors).is_err());
    assert!(store.nearest(&setting, &query, &vector).is_err());
    store.configure(&root.id, Some(&target())).unwrap();
    let next = store.settings().unwrap().remove(0);
    assert!(next.id > setting.id);
    assert_eq!(store.status(&next).unwrap().embedded, 0);
    store.remove(&root.id).unwrap();
    assert_eq!(store.settings().unwrap(), vec![]);
    assert!(store.pending(&next).is_err());
}

#[test]
fn changed_dimensions_roll_back_the_whole_embedding_batch() {
    let fixture = Fixture::new();
    fixture.write("a.txt", "first passage");
    fixture.write("b.txt", "second passage");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    store.configure(&root.id, Some(&target())).unwrap();
    let setting = store.settings().unwrap().remove(0);
    let inputs = store.pending(&setting).unwrap();
    assert!(
        store
            .commit(
                &setting,
                &inputs,
                &[
                    EmbeddingVector::new(vec![1., 0.]).unwrap(),
                    EmbeddingVector::new(vec![1.]).unwrap()
                ]
            )
            .is_err()
    );
    assert_eq!(store.status(&setting).unwrap().embedded, 0);
    store
        .commit(
            &setting,
            &inputs[..1],
            &[EmbeddingVector::new(vec![1., 0.]).unwrap()],
        )
        .unwrap();
    assert!(
        store
            .commit(
                &setting,
                &inputs[1..],
                &[EmbeddingVector::new(vec![1.]).unwrap()]
            )
            .is_err()
    );
    assert_eq!(store.status(&setting).unwrap().embedded, 1);
}

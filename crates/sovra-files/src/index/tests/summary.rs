use super::*;
use sovra_core::file_index::{EvidenceTier, summary::*};
use sovra_core::models::{ModelConnection, ModelDestination, ModelInfo, ModelWork, ProviderKind};

fn plan(roots: Vec<String>) -> SummaryPlan {
    let connection = ModelConnection::new("local", ProviderKind::OpenAiCompatible);
    let chat = ModelInfo {
        id: "text-model".into(),
        output_modalities: vec!["text".into()],
        ..ModelInfo::default()
    };
    let embedding = ModelInfo {
        id: "embedding-model".into(),
        output_modalities: vec!["embeddings".into()],
        ..ModelInfo::default()
    };
    SummaryPlan {
        name: "Garden map".into(),
        roots,
        summarizer: ModelDestination::for_work(&connection, &chat, ModelWork::Summaries).unwrap(),
        embedder: ModelDestination::for_embeddings(&connection, &embedding).unwrap(),
        max_calls: 80,
        automatic: false,
    }
}

fn setup() -> (
    Fixture,
    LocalFileIndex,
    IndexedRoot,
    SummaryConfiguration,
    SummaryInputRef,
) {
    let fixture = Fixture::new();
    fixture.write(
        "orchid.txt",
        "The orchid garden is proposed for next spring.",
    );
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let configuration = store.configure(&plan(vec![root.id.clone()])).unwrap();
    let reference = SummaryInputRef::Passage(search(&store, "orchid")[0].reference.clone());
    (fixture, store, root, configuration, reference)
}

fn output() -> SummaryText {
    SummaryText {
        title: "Garden proposal".into(),
        claims: vec![SummaryClaim {
            statement: "An orchid garden is proposed.".into(),
            supports: vec![0],
        }],
        caveats: vec!["The plan does not establish completion.".into()],
    }
}

fn prepare(
    store: &LocalFileIndex,
    configuration: u64,
    inputs: Vec<SummaryInputRef>,
) -> PreparedSummary {
    store
        .prepare(&SummaryRequest {
            configuration_id: configuration,
            kind: SummaryKind::PassageCluster,
            inputs,
        })
        .unwrap()
}

fn reserve(store: &LocalFileIndex, prepared: &PreparedSummary) -> SummaryAttempt {
    match store.reserve(prepared).unwrap() {
        SummaryReservation::Attempt(attempt) => attempt,
        SummaryReservation::Cached(_) => panic!("expected a new request"),
    }
}

fn save(store: &LocalFileIndex, configuration: u64, inputs: Vec<SummaryInputRef>) -> SummaryRecord {
    let prepared = prepare(store, configuration, inputs);
    let attempt = reserve(store, &prepared);
    store.commit(&attempt, &prepared, &output()).unwrap()
}

#[test]
fn summary_dag_preserves_exact_inputs_and_deduplicates_overlapping_sources() {
    let (fixture, store, _, configuration, reference) = setup();
    let leaf = save(&store, configuration.id, vec![reference.clone()]);
    let parent = save(
        &store,
        configuration.id,
        vec![SummaryInputRef::Summary(leaf.id), reference.clone()],
    );
    let top = save(
        &store,
        configuration.id,
        vec![
            SummaryInputRef::Summary(parent.id),
            SummaryInputRef::Summary(leaf.id),
        ],
    );
    assert_eq!(
        (top.level, top.source_files, top.source_passages),
        (3, 1, 1)
    );
    assert_eq!(top.basis.tier, EvidenceTier::ModelInference);
    assert!(top.basis.confidence.is_none());
    assert!(top.basis.observed_at.is_none());
    assert!(top.basis.valid_time.is_none());
    assert_eq!(top.model, configuration.plan.summarizer);
    assert_eq!(top.prompt_revision, SUMMARY_PROMPT_REVISION);
    assert_eq!(leaf.inputs[0].reference, reference);
    assert!(leaf.inputs[0].text.contains("next spring"));
    assert!(
        top.inputs[0]
            .extraction_note
            .as_ref()
            .unwrap()
            .contains("Model-generated")
    );
    assert_eq!(fixture.store().read_summary(top.id, false).unwrap(), top);
    let page = store
        .query_summaries(&SummaryQuery {
            limit: 2,
            text: Some("proposal".into()),
            ..SummaryQuery::default()
        })
        .unwrap();
    assert_eq!(
        page.summaries.iter().map(|s| s.id).collect::<Vec<_>>(),
        [top.id, parent.id]
    );
    let older = store
        .query_summaries(&SummaryQuery {
            before_id: page.next_before,
            ..SummaryQuery::default()
        })
        .unwrap();
    assert_eq!(older.summaries[0].id, leaf.id);
    assert!(older.next_before.is_none());
}

#[test]
fn summary_reservations_are_durable_bounded_and_cancelled_across_pause_resume() {
    let (fixture, store, _, configuration, reference) = setup();
    let mut budget = configuration.plan;
    budget.max_calls = 3;
    let configuration = store.configure(&budget).unwrap();
    let prepared = prepare(&store, configuration.id, vec![reference]);
    let failed = reserve(&store, &prepared);
    store.fail_summary(&failed, "transport failed").unwrap();
    assert!(store.commit(&failed, &prepared, &output()).is_err());
    let cancelled = reserve(&store, &prepared);
    store.set_enabled(configuration.id, false).unwrap();
    assert!(store.reserve(&prepared).is_err());
    store.set_enabled(configuration.id, true).unwrap();
    assert!(store.commit(&cancelled, &prepared, &output()).is_err());
    let success = reserve(&store, &prepared);
    let record = store.commit(&success, &prepared, &output()).unwrap();
    assert!(store.commit(&success, &prepared, &output()).is_err());
    assert!(
        matches!(store.reserve(&prepared).unwrap(),SummaryReservation::Cached(cached) if cached.id==record.id)
    );
    let higher = prepare(
        &store,
        configuration.id,
        vec![SummaryInputRef::Summary(record.id)],
    );
    assert!(store.reserve(&higher).unwrap_err().0.contains("budget"));
    let reopened = fixture.store().configurations().unwrap();
    assert_eq!(
        reopened
            .iter()
            .find(|c| c.id == configuration.id)
            .unwrap()
            .calls_used,
        3
    );
}

#[test]
fn source_changes_invalidate_all_ancestors_and_reject_in_flight_results() {
    let (fixture, store, root, configuration, reference) = setup();
    fixture.write("independent.txt", "Tulip beds are complete.");
    store.scan(&root.id, &Observer::default()).unwrap();
    let independent = save(
        &store,
        configuration.id,
        vec![SummaryInputRef::Passage(
            search(&store, "Tulip")[0].reference.clone(),
        )],
    );
    let leaf = save(&store, configuration.id, vec![reference.clone()]);
    let left = save(
        &store,
        configuration.id,
        vec![SummaryInputRef::Summary(leaf.id)],
    );
    let right = save(
        &store,
        configuration.id,
        vec![reference, SummaryInputRef::Summary(leaf.id)],
    );
    let top = save(
        &store,
        configuration.id,
        vec![
            SummaryInputRef::Summary(left.id),
            SummaryInputRef::Summary(right.id),
        ],
    );
    let pending = prepare(
        &store,
        configuration.id,
        vec![SummaryInputRef::Summary(top.id)],
    );
    let attempt = reserve(&store, &pending);
    fixture.write("orchid.txt", "The orchid garden has been cancelled.");
    store.scan(&root.id, &Observer::default()).unwrap();
    assert!(store.commit(&attempt, &pending, &output()).is_err());
    for id in [leaf.id, left.id, right.id, top.id] {
        assert!(store.read_summary(id, false).is_err());
        assert_eq!(
            store.read_summary(id, true).unwrap().invalidation,
            Some(SummaryInvalidation::SourceChanged)
        );
    }
    let current = store.query_summaries(&SummaryQuery::default()).unwrap();
    assert_eq!(
        current.summaries.iter().map(|s| s.id).collect::<Vec<_>>(),
        [independent.id]
    );
    let history = store
        .query_summaries(&SummaryQuery {
            include_stale: true,
            ..SummaryQuery::default()
        })
        .unwrap();
    assert_eq!(history.summaries.len(), 5);
    assert_eq!(history.summaries.iter().filter(|s| s.stale).count(), 4);
    assert!(
        store.read_summary(leaf.id, true).unwrap().inputs[0]
            .text
            .contains("proposed")
    );
}

#[test]
fn derived_corrections_retire_ancestors_and_allow_rebuilding_without_erasing_history() {
    let (_fixture, store, _, configuration, reference) = setup();
    let leaf = save(&store, configuration.id, vec![reference.clone()]);
    let parent = save(
        &store,
        configuration.id,
        vec![SummaryInputRef::Summary(leaf.id)],
    );
    assert_eq!(store.invalidate_summary(leaf.id).unwrap(), 2);
    assert_eq!(store.invalidate_summary(leaf.id).unwrap(), 0);
    assert_eq!(
        store.read_summary(parent.id, true).unwrap().invalidation,
        Some(SummaryInvalidation::DependencyInvalidated)
    );
    let replacement = save(&store, configuration.id, vec![reference]);
    assert_ne!(replacement.id, leaf.id);
    assert!(store.read_summary(replacement.id, false).is_ok());
    assert!(store.read_summary(parent.id, false).is_err());
}

#[test]
fn removing_any_approved_scope_permanently_revokes_the_generation() {
    let (fixture, store, root, configuration, reference) = setup();
    let record = save(&store, configuration.id, vec![reference]);
    store.remove(&root.id).unwrap();
    assert!(store.read_summary(record.id, true).is_err());
    assert_eq!(
        store.configurations().unwrap(),
        Vec::<SummaryConfiguration>::new()
    );
    assert_eq!(
        store
            .query_summaries(&SummaryQuery {
                include_stale: true,
                ..SummaryQuery::default()
            })
            .unwrap()
            .summaries,
        Vec::<SummaryHit>::new()
    );
    let regranted = store.add_root(&fixture.source()).unwrap();
    store.scan(&regranted.id, &Observer::default()).unwrap();
    assert!(store.set_enabled(configuration.id, true).is_err());
    assert!(store.read_summary(record.id, true).is_err());
    let new = store.configure(&plan(vec![regranted.id])).unwrap();
    let source = SummaryInputRef::Passage(search(&store, "orchid")[0].reference.clone());
    assert!(
        store
            .read_summary(save(&store, new.id, vec![source]).id, false)
            .is_ok()
    );
}

#[test]
fn summary_scope_filters_cover_every_source_and_generations_cannot_mix() {
    let (fixture, store, root, configuration, reference) = setup();
    let other = fixture.0.join("second");
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("lily.txt"), "Lily planting schedule.").unwrap();
    let second = store.add_root(&other).unwrap();
    store.scan(&second.id, &Observer::default()).unwrap();
    let lily = SummaryInputRef::Passage(search(&store, "Lily")[0].reference.clone());
    assert!(
        store
            .prepare(&SummaryRequest {
                configuration_id: configuration.id,
                kind: SummaryKind::Document,
                inputs: vec![lily.clone()]
            })
            .is_err()
    );
    let mixed = store
        .configure(&plan(vec![root.id.clone(), second.id.clone()]))
        .unwrap();
    let leaf = save(&store, configuration.id, vec![reference.clone()]);
    assert!(
        store
            .prepare(&SummaryRequest {
                configuration_id: mixed.id,
                kind: SummaryKind::Workspace,
                inputs: vec![SummaryInputRef::Summary(leaf.id)]
            })
            .is_err()
    );
    let mixed_record = save(&store, mixed.id, vec![reference, lily]);
    let restricted = store
        .query_summaries(&SummaryQuery {
            roots: vec![root.id.clone()],
            ..SummaryQuery::default()
        })
        .unwrap();
    assert_eq!(
        restricted
            .summaries
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        [leaf.id]
    );
    let both = store
        .query_summaries(&SummaryQuery {
            roots: vec![root.id, second.id],
            configuration_ids: vec![mixed.id],
            ..SummaryQuery::default()
        })
        .unwrap();
    assert_eq!(both.summaries[0].id, mixed_record.id);
    assert_eq!(both.summaries[0].source_files, 2);
}

#[test]
fn rename_and_disappearance_invalidate_summaries_even_without_new_text() {
    let (fixture, store, root, configuration, reference) = setup();
    let first = save(&store, configuration.id, vec![reference]);
    std::fs::rename(
        fixture.source().join("orchid.txt"),
        fixture.source().join("renamed.txt"),
    )
    .unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    assert!(store.read_summary(first.id, false).is_err());
    let renamed = save(
        &store,
        configuration.id,
        vec![SummaryInputRef::Passage(
            search(&store, "orchid")[0].reference.clone(),
        )],
    );
    assert!(renamed.inputs[0].label.contains("renamed.txt"));
    std::fs::remove_file(fixture.source().join("renamed.txt")).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    assert!(store.read_summary(renamed.id, false).is_err());
    assert!(store.read_summary(renamed.id, true).is_ok());
}

#[test]
fn corrupt_summaries_and_forged_prepared_inputs_fail_without_saving_partial_results() {
    let (_fixture, store, _, configuration, reference) = setup();
    let prepared = prepare(&store, configuration.id, vec![reference]);
    let attempt = reserve(&store, &prepared);
    let mut forged = prepared.clone();
    forged.inputs[0].text = "forged source".into();
    assert!(store.commit(&attempt, &forged, &output()).is_err());
    let mut invalid = output();
    invalid.claims[0].supports = vec![1];
    assert!(store.commit(&attempt, &prepared, &invalid).is_err());
    assert_eq!(
        store
            .query_summaries(&SummaryQuery::default())
            .unwrap()
            .summaries,
        Vec::<SummaryHit>::new()
    );
    let record = store.commit(&attempt, &prepared, &output()).unwrap();
    store
        .connect()
        .unwrap()
        .execute(
            "UPDATE index_summaries SET details='broken JSON' WHERE id=?1",
            [i64::try_from(record.id).unwrap()],
        )
        .unwrap();
    assert!(store.read_summary(record.id, false).is_err());
    assert!(store.query_summaries(&SummaryQuery::default()).is_err());
}

#[test]
fn summary_depth_is_bounded_without_dropping_the_source_lineage() {
    let (_fixture, store, _, configuration, reference) = setup();
    let mut record = save(&store, configuration.id, vec![reference]);
    for _ in 1..MAX_SUMMARY_LEVEL {
        record = save(
            &store,
            configuration.id,
            vec![SummaryInputRef::Summary(record.id)],
        );
    }
    assert_eq!(
        (record.level, record.source_passages),
        (MAX_SUMMARY_LEVEL, 1)
    );
    assert!(
        store
            .prepare(&SummaryRequest {
                configuration_id: configuration.id,
                kind: SummaryKind::Workspace,
                inputs: vec![SummaryInputRef::Summary(record.id)]
            })
            .unwrap_err()
            .0
            .contains("hierarchy budget")
    );
}

#[test]
fn summary_search_is_literal_and_matches_unicode_case() {
    let (_fixture, store, _, configuration, reference) = setup();
    let prepared = prepare(&store, configuration.id, vec![reference]);
    let attempt = reserve(&store, &prepared);
    let mut content = output();
    content.title = "CAFÉ 100%_proposal".into();
    store.commit(&attempt, &prepared, &content).unwrap();
    for text in ["café", "%_"] {
        assert_eq!(
            store
                .query_summaries(&SummaryQuery {
                    text: Some(text.into()),
                    ..SummaryQuery::default()
                })
                .unwrap()
                .summaries
                .len(),
            1
        );
    }
    assert_eq!(
        store
            .query_summaries(&SummaryQuery {
                text: Some("100__proposal".into()),
                ..SummaryQuery::default()
            })
            .unwrap()
            .summaries,
        Vec::<SummaryHit>::new()
    );
}

#[test]
fn removing_an_unused_scope_still_revokes_its_approved_map_generation() {
    let (fixture, store, root, _, reference) = setup();
    let second_path = fixture.0.join("second");
    std::fs::create_dir(&second_path).unwrap();
    let second = store.add_root(&second_path).unwrap();
    let configuration = store
        .configure(&plan(vec![root.id, second.id.clone()]))
        .unwrap();
    let record = save(&store, configuration.id, vec![reference]);
    store.remove(&second.id).unwrap();
    assert!(store.read_summary(record.id, true).is_err());
    assert_eq!(
        store
            .query_summaries(&SummaryQuery {
                include_stale: true,
                ..SummaryQuery::default()
            })
            .unwrap()
            .summaries,
        Vec::<SummaryHit>::new()
    );
}

#[test]
fn schema_seven_upgrade_preserves_existing_source_citations() {
    let (fixture, store, root, _, reference) = setup();
    // Reconstruct the prior schema in this private fixture, retaining its source tables.
    let db = store.connect().unwrap();
    db.execute_batch(
        "ALTER TABLE index_versions DROP COLUMN partial; DROP TABLE index_scan_notices; DROP TABLE index_scan_reports; DROP TRIGGER summary_changed_source; DROP TRIGGER summary_deleted_source;
        DROP TRIGGER summary_revoked_scope; DROP TABLE index_summary_sources;
        DROP TABLE index_summary_edges; DROP TABLE index_summary_attempts;
        DROP TABLE index_summaries; DROP TABLE index_summary_scopes;
        DROP TABLE index_summary_configurations; PRAGMA user_version=7;",
    )
    .unwrap();
    drop(db);
    let reopened = fixture.store();
    let SummaryInputRef::Passage(passage) = &reference else {
        panic!("source reference");
    };
    assert!(reopened.read(passage).unwrap().text.contains("orchid"));
    let configuration = reopened.configure(&plan(vec![root.id])).unwrap();
    let summary = save(&reopened, configuration.id, vec![reference]);
    assert_eq!(summary.source_passages, 1);
    let version: i64 = reopened
        .connect()
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

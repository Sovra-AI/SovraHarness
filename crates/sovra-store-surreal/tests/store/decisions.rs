use sovra_core::decisions::{DecisionDraft, DecisionEvidence, DecisionPerson, DecisionRecord};
use sovra_core::{MemoryStore, Timestamp};

fn record(id: &str) -> DecisionRecord {
    DecisionRecord::create(
        id.into(),
        DecisionDraft {
            question: "Renew the lease?".into(),
            decision: "Renew for a year.".into(),
            criteria: "Within the annual budget.".into(),
            review_on: "2099-01-01".into(),
            ends_when: "The building becomes unavailable.".into(),
            evidence: vec![DecisionEvidence {
                reference: "Lease, page 2".into(),
                passage: "Annual renewal is available.".into(),
            }],
            ..DecisionDraft::default()
        },
        person("writer"),
        Timestamp(1),
    )
    .expect("decision test operation succeeds")
}

fn person(id: &str) -> DecisionPerson {
    DecisionPerson {
        account_id: id.into(),
        name: id.into(),
    }
}

#[tokio::test]
async fn decision_history_and_acceptance_round_trip_and_stale_writes_fail_atomically() {
    let store = super::open_store().await;
    assert!(
        store
            .decision("one")
            .await
            .expect("decision test operation succeeds")
            .is_none()
    );
    assert_eq!(
        store
            .list_decisions(0)
            .await
            .expect("decision test operation succeeds"),
        Vec::new()
    );
    let original = record("one");
    store
        .save_decision(&original, 0)
        .await
        .expect("decision test operation succeeds");
    assert!(store.save_decision(&original, 0).await.is_err());
    let accepted = original
        .accept(1, person("reviewer"), Timestamp(2))
        .expect("decision test operation succeeds");
    store
        .save_decision(&accepted, 1)
        .await
        .expect("decision test operation succeeds");
    let stale = original
        .end(1, "No longer needed".into(), person("writer"), Timestamp(3))
        .expect("decision test operation succeeds");
    assert!(store.save_decision(&stale, 1).await.is_err());
    assert_eq!(
        store
            .decision("one")
            .await
            .expect("decision test operation succeeds"),
        Some(accepted.clone())
    );
    let mut next_draft = accepted
        .current()
        .expect("decision test operation succeeds")
        .draft
        .clone();
    next_draft.decision = "Renew for two years.".into();
    let next = accepted
        .edit(2, next_draft, person("writer"), Timestamp(4))
        .expect("decision test operation succeeds");
    store
        .save_decision(&next, 2)
        .await
        .expect("decision test operation succeeds");
    assert_eq!(
        store
            .decision("one")
            .await
            .expect("decision test operation succeeds"),
        Some(next.clone())
    );
    assert_eq!(
        store
            .list_decisions(0)
            .await
            .expect("decision test operation succeeds"),
        vec![next.summary().expect("decision test operation succeeds")]
    );
    assert_eq!(
        store
            .list_decisions(1)
            .await
            .expect("decision test operation succeeds"),
        Vec::new()
    );
    assert_eq!(next.versions[0], accepted.versions[0]);
}

#[tokio::test]
async fn decision_list_is_bounded_with_a_look_ahead_record() {
    let store = super::open_store().await;
    for index in 0..103 {
        store
            .save_decision(&record(&format!("decision-{index:03}")), 0)
            .await
            .expect("decision test operation succeeds");
    }
    let first = store
        .list_decisions(0)
        .await
        .expect("decision test operation succeeds");
    assert_eq!(first.len(), 101);
    assert_eq!(first[0].id, "decision-000");
    let last = store
        .list_decisions(100)
        .await
        .expect("decision test operation succeeds");
    assert_eq!(last.len(), 3);
    assert_eq!(last[0].id, "decision-100");
}

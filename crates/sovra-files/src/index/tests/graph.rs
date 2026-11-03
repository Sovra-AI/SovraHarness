use super::*;

#[test]
fn replacing_a_hierarchy_checks_the_final_batch_not_intermediate_order() {
    let (_fixture, store, _, mut initial) = setup();
    initial.entities = ["a", "b", "c"]
        .into_iter()
        .map(|id| EntityDraft {
            local_id: id.into(),
            kind: EntityKind::Task,
            label: id.into(),
            evidence: vec![0],
        })
        .collect();
    let base = initial.relations[0].clone();
    initial.relations = vec![
        RelationDraft {
            subject: DraftEndpoint::New("a".into()),
            relation: RelationKind::PartOf,
            object: DraftEndpoint::New("b".into()),
            ..base.clone()
        },
        RelationDraft {
            subject: DraftEndpoint::New("b".into()),
            ..base.clone()
        },
        RelationDraft {
            subject: DraftEndpoint::New("c".into()),
            ..base.clone()
        },
    ];
    let p = store.propose(&initial, &origin()).unwrap();
    store.decide(p.id, true).unwrap();
    let page = store.query(&GraphQuery::default()).unwrap();
    let node = |label: &str| {
        DraftEndpoint::Entity(
            page.entities
                .iter()
                .find(|e| e.label == label)
                .unwrap()
                .id
                .clone(),
        )
    };
    let old = page
        .relations
        .iter()
        .find(|r| r.relation == RelationKind::PartOf)
        .unwrap()
        .id
        .clone();
    let replacement = GraphDraft {
        title: "Reparent the task hierarchy".into(),
        evidence: initial.evidence,
        entities: vec![],
        relations: vec![
            RelationDraft {
                subject: node("b"),
                relation: RelationKind::PartOf,
                object: node("a"),
                ..base.clone()
            },
            RelationDraft {
                subject: node("a"),
                relation: RelationKind::PartOf,
                object: node("c"),
                replaces: Some(old),
                ..base
            },
        ],
    };
    let p = store.propose(&replacement, &origin()).unwrap();
    store.decide(p.id, true).unwrap();
    assert_eq!(
        store
            .query(&GraphQuery::default())
            .unwrap()
            .relations
            .iter()
            .filter(|r| r.relation == RelationKind::PartOf)
            .count(),
        2
    );
}

use sovra_core::file_index::{Confidence, EvidenceTier, WorldValidity, graph::*};

fn origin() -> GraphOrigin {
    GraphOrigin {
        session_id: "session".into(),
        invocation_id: "call".into(),
        agent: "file helper".into(),
    }
}
fn draft(reference: PassageRef) -> GraphDraft {
    GraphDraft {
        title: "Map the project brief".into(),
        evidence: vec![reference],
        entities: vec![EntityDraft {
            local_id: "brief".into(),
            kind: EntityKind::Document,
            label: "Orchid brief".into(),
            evidence: vec![0],
        }],
        relations: vec![RelationDraft {
            subject: DraftEndpoint::New("brief".into()),
            relation: RelationKind::GroundedIn,
            object: DraftEndpoint::Source(0),
            statement: "This source represents the Orchid brief.".into(),
            evidence: vec![0],
            confidence: Some(Confidence::new(0.7).unwrap()),
            valid_time: None,
            replaces: None,
        }],
    }
}
fn setup() -> (Fixture, LocalFileIndex, IndexedRoot, GraphDraft) {
    let fixture = Fixture::new();
    fixture.write(
        "brief.txt",
        "Orchid project brief: a proposal for the garden.",
    );
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let draft = draft(search(&store, "Orchid")[0].reference.clone());
    (fixture, store, root, draft)
}

#[test]
fn paging_continues_after_a_full_batch_of_temporally_filtered_relationships() {
    let (_fixture, store, _, mut draft) = setup();
    draft.relations[0].valid_time = Some(WorldValidity {
        from: Some(1),
        until: Some(10),
    });
    let old = store.propose(&draft, &origin()).unwrap();
    store.decide(old.id, true).unwrap();
    store
        .connect()
        .unwrap()
        .execute("UPDATE index_knowledge_relations SET created=0", [])
        .unwrap();
    let base = RelationDraft {
        valid_time: None,
        ..draft.relations[0].clone()
    };
    for batch in 0..42 {
        draft.relations = (0..24)
            .map(|n| RelationDraft {
                statement: format!("Undated relationship {batch}/{n}"),
                ..base.clone()
            })
            .collect();
        let mut origin = origin();
        origin.invocation_id = format!("batch-{batch}");
        let proposal = store.propose(&draft, &origin).unwrap();
        store.decide(proposal.id, true).unwrap();
    }
    let mut query = GraphQuery {
        valid_at: Some(5),
        ..Default::default()
    };
    let first = store.query(&query).unwrap();
    assert_eq!(first.relations.len(), 0);
    assert!(first.truncated);
    query.before = first.next;
    assert!(query.before.is_some());
    let second = store.query(&query).unwrap();
    assert_eq!(second.relations.len(), 1);
    assert_eq!(second.relations[0].proposal_id, old.id);
    assert!(second.next.is_none());
    assert!(!second.truncated);
}

#[test]
fn saved_relationship_pages_cover_equal_timestamps_and_do_not_repeat_after_insertion() {
    let (_fixture, store, root, mut draft) = setup();
    for n in 0..7 {
        draft.relations[0].statement = format!("Orchid relationship {n}");
        let mut origin = origin();
        origin.invocation_id = format!("paging-{n}");
        let proposal = store.propose(&draft, &origin).unwrap();
        store.decide(proposal.id, true).unwrap();
    }
    // Equal timestamps exercise the ID tie-breaker without relying on test speed.
    store
        .connect()
        .unwrap()
        .execute("UPDATE index_knowledge_relations SET created=1", [])
        .unwrap();
    let mut query = GraphQuery {
        limit: 2,
        ..Default::default()
    };
    let first = store.query(&query).unwrap();
    assert_eq!(first.relations.len(), 2);
    query.before = first.next;
    assert!(query.before.is_some());
    let mut ids: Vec<_> = first.relations.into_iter().map(|r| r.id).collect();
    draft.relations[0].statement = "A newly inserted relationship".into();
    let mut origin = origin();
    origin.invocation_id = "insert-after-first-page".into();
    let new = store.propose(&draft, &origin).unwrap();
    store.decide(new.id, true).unwrap();
    for _ in 0..5 {
        let page = store.query(&query).unwrap();
        ids.extend(page.relations.into_iter().map(|r| r.id));
        query.before = page.next;
        if query.before.is_none() {
            break;
        }
    }
    assert!(query.before.is_none());
    assert_eq!(ids.len(), 7);
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 7);
    store.remove(&root.id).unwrap();
    assert_eq!(
        store.query(&GraphQuery::default()).unwrap().relations.len(),
        0
    );
}

#[test]
fn review_is_required_idempotent_and_does_not_upgrade_an_inference() {
    let (fixture, store, _, draft) = setup();
    let proposed = store.propose(&draft, &origin()).unwrap();
    assert_eq!(store.propose(&draft, &origin()).unwrap().id, proposed.id);
    assert_eq!(store.pending().unwrap(), vec![proposed.clone()]);
    assert_eq!(
        store.query(&GraphQuery::default()).unwrap().relations,
        Vec::<GraphRelation>::new()
    );
    assert_eq!(
        store.decide(proposed.id, true).unwrap().state,
        ProposalState::Accepted
    );
    assert_eq!(store.pending().unwrap(), Vec::<KnowledgeProposal>::new());
    assert!(store.decide(proposed.id, true).is_err());
    let page = fixture.store().query(&GraphQuery::default()).unwrap();
    assert_eq!(page.relations.len(), 1);
    assert_eq!(page.entities.len(), 1);
    assert_eq!(page.relations[0].basis.tier, EvidenceTier::ModelInference);
    assert_eq!(page.relations[0].origin, Some(origin()));
    assert!(
        store
            .propose(&draft, &origin())
            .unwrap()
            .reviewed_at
            .is_some()
    );
    assert_eq!(
        page.relations[0].basis.confidence,
        Confidence::new(0.7).ok()
    );
    assert!(page.relations[0].basis.observed_at.is_none());
    assert_eq!(
        store.evidence(page.sources[0].id).unwrap().passage,
        proposed.sources[0]
    );
    let mut rejected = draft;
    rejected.title = "Alternative rejected mapping".into();
    let proposal = store.propose(&rejected, &origin()).unwrap();
    store.decide(proposal.id, false).unwrap();
    assert_eq!(
        store.query(&GraphQuery::default()).unwrap().relations.len(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(fixture.source().join("brief.txt")).unwrap(),
        "Orchid project brief: a proposal for the garden."
    );
}

#[test]
fn changed_sources_block_approval_and_require_explicit_historical_queries() {
    let (fixture, store, root, draft) = setup();
    let accepted = store.propose(&draft, &origin()).unwrap();
    store.decide(accepted.id, true).unwrap();
    let mut later = draft.clone();
    later.title = "Another pending claim".into();
    let pending = store.propose(&later, &origin()).unwrap();
    fixture.write(
        "brief.txt",
        "Orchid project changed; the garden is now cancelled.",
    );
    store.scan(&root.id, &Observer::default()).unwrap();
    assert!(
        store
            .decide(pending.id, true)
            .unwrap_err()
            .0
            .contains("source changed")
    );
    assert_eq!(
        store.query(&GraphQuery::default()).unwrap().relations,
        Vec::<GraphRelation>::new()
    );
    let historical = store
        .query(&GraphQuery {
            include_stale: true,
            ..GraphQuery::default()
        })
        .unwrap();
    assert_eq!(historical.relations.len(), 1);
    assert!(!historical.relations[0].sources_current);
    let evidence = store.evidence(historical.sources[0].id).unwrap();
    assert!(!evidence.source.current);
    assert!(evidence.passage.text.contains("proposal for the garden"));
    assert!(
        store
            .propose(
                &GraphDraft {
                    title: "New stale citation".into(),
                    ..draft
                },
                &origin()
            )
            .is_err()
    );
    store.decide(pending.id, false).unwrap();
}

#[test]
fn revoked_grants_cannot_be_resurrected_and_release_pending_quota() {
    let (fixture, store, root, draft) = setup();
    let proposal = store.propose(&draft, &origin()).unwrap();
    store.decide(proposal.id, true).unwrap();
    let evidence = store.query(&GraphQuery::default()).unwrap().sources[0].id;
    for i in 0..8 {
        store
            .propose(
                &GraphDraft {
                    title: format!("Pending {i}"),
                    ..draft.clone()
                },
                &origin(),
            )
            .unwrap();
    }
    store.remove(&root.id).unwrap();
    assert_eq!(store.pending().unwrap(), Vec::<KnowledgeProposal>::new());
    assert!(store.evidence(evidence).is_err());
    assert_eq!(
        store
            .query(&GraphQuery {
                include_stale: true,
                ..GraphQuery::default()
            })
            .unwrap()
            .relations,
        Vec::<GraphRelation>::new()
    );
    let regranted = store.add_root(&fixture.source()).unwrap();
    store.scan(&regranted.id, &Observer::default()).unwrap();
    assert!(store.evidence(evidence).is_err());
    assert_eq!(
        store
            .query(&GraphQuery {
                include_stale: true,
                ..GraphQuery::default()
            })
            .unwrap()
            .relations,
        Vec::<GraphRelation>::new()
    );
    let fresh = self::draft(search(&store, "Orchid")[0].reference.clone());
    assert!(
        store
            .propose(
                &fresh,
                &GraphOrigin {
                    invocation_id: "new-call".into(),
                    ..origin()
                }
            )
            .is_ok()
    );
}

#[test]
fn logical_identity_and_traversal_respect_all_endpoint_grants() {
    let (fixture, store, root, draft) = setup();
    let p = store.propose(&draft, &origin()).unwrap();
    store.decide(p.id, true).unwrap();
    let entity = store.query(&GraphQuery::default()).unwrap().entities[0]
        .id
        .clone();
    let other = fixture.0.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("export.md"),
        "Orchid project brief: the same proposal in another format.",
    )
    .unwrap();
    let root2 = store.add_root(&other).unwrap();
    store.scan(&root2.id, &Observer::default()).unwrap();
    let second = search(&store, "Orchid")
        .into_iter()
        .find(|p| p.reference.root_id == root2.id)
        .unwrap();
    let mut linked = self::draft(second.reference);
    linked.entities.clear();
    linked.relations[0].subject = DraftEndpoint::Entity(entity.clone());
    linked.title = "Ground the second representation".into();
    let p = store.propose(&linked, &origin()).unwrap();
    assert_eq!(p.required_roots.len(), 2);
    store.decide(p.id, true).unwrap();
    assert_eq!(
        store
            .query(&GraphQuery {
                entity_id: Some(entity.clone()),
                hops: 3,
                ..GraphQuery::default()
            })
            .unwrap()
            .relations
            .len(),
        2
    );
    assert_eq!(
        store.query(&GraphQuery::default()).unwrap().entities.len(),
        1
    );
    assert_eq!(
        store
            .query(&GraphQuery {
                roots: vec![root2.id],
                ..GraphQuery::default()
            })
            .unwrap()
            .relations,
        Vec::<GraphRelation>::new()
    );
    store.remove(&root.id).unwrap();
    assert_eq!(
        store.query(&GraphQuery::default()).unwrap().relations,
        Vec::<GraphRelation>::new()
    );
    assert!(
        store
            .query(&GraphQuery {
                entity_id: Some(entity),
                ..GraphQuery::default()
            })
            .is_err()
    );
}

#[test]
fn invalid_domains_and_part_cycles_leave_no_partial_knowledge() {
    let (_fixture, store, _, mut draft) = setup();
    draft.entities[0].kind = EntityKind::Section;
    draft.entities.push(EntityDraft {
        local_id: "parent".into(),
        kind: EntityKind::Section,
        label: "Parent".into(),
        evidence: vec![0],
    });
    draft.relations[0].relation = RelationKind::PartOf;
    draft.relations[0].object = DraftEndpoint::New("parent".into());
    draft.relations.push(RelationDraft {
        subject: DraftEndpoint::New("parent".into()),
        object: DraftEndpoint::New("brief".into()),
        ..draft.relations[0].clone()
    });
    let p = store.propose(&draft, &origin()).unwrap();
    assert!(store.decide(p.id, true).unwrap_err().0.contains("cycle"));
    assert_eq!(
        store.query(&GraphQuery::default()).unwrap().relations,
        Vec::<GraphRelation>::new()
    );
    assert_eq!(store.pending().unwrap()[0].state, ProposalState::Pending);
    draft.entities[0].kind = EntityKind::Document;
    assert!(
        store
            .propose(&draft, &origin())
            .unwrap_err()
            .0
            .contains("incompatible")
    );
}

#[test]
fn supersession_keeps_history_and_distinguishes_recorded_from_world_time() {
    let (_fixture, store, _, draft) = setup();
    let p = store.propose(&draft, &origin()).unwrap();
    store.decide(p.id, true).unwrap();
    let original = store.query(&GraphQuery::default()).unwrap().relations[0].clone();
    let before = now() - 10;
    store.connect().unwrap().execute("UPDATE index_knowledge_relations SET created=?1, details=json_set(details,'$.basis.recorded_at',?1) WHERE id=?2", params![i64::try_from(before).unwrap(), original.id]).unwrap();
    let mut revised = draft;
    revised.title = "Correct the interpretation".into();
    revised.entities.clear();
    revised.relations[0].subject = match original.subject {
        GraphEndpoint::Entity(id) => DraftEndpoint::Entity(id),
        GraphEndpoint::Source(_) => panic!("entity subject"),
    };
    revised.relations[0].statement = "This is a draft, not the final brief.".into();
    revised.relations[0].replaces = Some(original.id.clone());
    revised.relations[0].valid_time = Some(WorldValidity {
        from: Some(20),
        until: Some(40),
    });
    let p = store.propose(&revised, &origin()).unwrap();
    assert_eq!(p.replaced_relations[0].id, original.id);
    store.decide(p.id, true).unwrap();
    let current = store.query(&GraphQuery::default()).unwrap();
    assert_eq!(current.relations.len(), 1);
    assert_eq!(current.relations[0].replaces, Some(original.id.clone()));
    let historical = store
        .query(&GraphQuery {
            recorded_at: Some(before + 1),
            ..GraphQuery::default()
        })
        .unwrap();
    assert_eq!(historical.relations[0].id, original.id);
    assert!(historical.relations[0].expired_at.is_some());
    assert_eq!(
        store
            .query(&GraphQuery {
                include_superseded: true,
                ..GraphQuery::default()
            })
            .unwrap()
            .relations
            .len(),
        2
    );
    assert_eq!(
        store
            .query(&GraphQuery {
                valid_at: Some(30),
                include_superseded: true,
                ..GraphQuery::default()
            })
            .unwrap()
            .relations
            .len(),
        1
    );
    assert_eq!(
        store
            .query(&GraphQuery {
                valid_at: Some(50),
                ..GraphQuery::default()
            })
            .unwrap()
            .relations,
        Vec::<GraphRelation>::new()
    );
    assert!(
        store
            .query(&GraphQuery {
                limit: 1,
                include_superseded: true,
                ..GraphQuery::default()
            })
            .unwrap()
            .truncated
    );
}

#[test]
fn malformed_stored_records_surface_errors_instead_of_silent_absence() {
    let (_fixture, store, _, draft) = setup();
    let p = store.propose(&draft, &origin()).unwrap();
    store
        .connect()
        .unwrap()
        .execute(
            "UPDATE index_knowledge_proposals SET details='{}' WHERE id=?1",
            [i64::try_from(p.id).unwrap()],
        )
        .unwrap();
    assert!(store.pending().is_err());
}

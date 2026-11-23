use super::*;
use sovra_core::file_index::EvidenceTier;
use sovra_core::{
    file_index::{
        automatic::DiscoveryOverview,
        graph::{GraphQuery, RelationKind},
        ontology::LifeArea,
    },
    knowledge_analysis::FindingKind,
};

fn finish(store: &LocalFileIndex) -> DiscoveryOverview {
    let auto = store.automatic().unwrap();
    for _ in 0..100 {
        if !auto.advance().unwrap() {
            return auto.overview(None).unwrap().unwrap();
        }
    }
    panic!("automatic indexing failed to reach an idle state");
}

#[test]
fn ordinary_scans_classify_current_sources_and_keep_evidence_without_review() {
    let fixture = Fixture::new();
    fixture.write(
        "Work/tasks.txt",
        "Painting depends on Priming.\nPainting is a task for Garden repairs.\n",
    );
    fixture.write(
        "Work/observations.csv",
        "date,x,y,value,unit\n2025-05-01,0,0,12,kg\n2025-05-02,3,4,20,kg\n",
    );
    fixture.write("Work/plants.csv","subject,predicate,object,subject_type,object_type\nBasil,grown_by,Juniper Co,farm.crop,organization\n");
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    let report = finish(&store);
    assert!(!report.working);
    assert_eq!(report.kinds.iter().map(|v| v.1).sum::<u64>(), 3);
    assert_eq!(report.areas, [(LifeArea::Work, 3)]);
    for kind in [
        FindingKind::Profile,
        FindingKind::Measurement,
        FindingKind::Chronology,
        FindingKind::Topology,
        FindingKind::Relation,
    ] {
        assert!(
            report.examples.iter().any(|e| e.kind == kind),
            "{kind:?} {report:?}"
        );
    }
    let graph = store.knowledge().unwrap();
    let page = graph.query(&GraphQuery::default()).unwrap();
    assert_eq!(page.relations.len(), 3, "{page:?} {report:?}");
    assert_eq!(
        graph.pending().unwrap().len(),
        0,
        "automatic discoveries require no review"
    );
    let prose = page
        .relations
        .iter()
        .find(|r| r.relation == RelationKind::DependsOn)
        .unwrap();
    assert_eq!(prose.basis.tier, EvidenceTier::HeuristicExtraction);
    assert_eq!(
        prose.subject,
        page.relations
            .iter()
            .find(|r| r.relation == RelationKind::ForProject)
            .unwrap()
            .subject
    );
    let table = page
        .relations
        .iter()
        .find(|r| matches!(r.relation, RelationKind::Property(_)))
        .unwrap();
    assert_eq!(table.basis.tier, EvidenceTier::DeterministicExtraction);
    let id = prose.evidence[0];
    assert!(graph.evidence(id).unwrap().passage.text.contains("Priming"));
    assert!(
        store
            .automatic()
            .unwrap()
            .overview(Some(report.revision))
            .unwrap()
            .is_none()
    );
    let reopened = fixture.store();
    assert_eq!(finish(&reopened), report);
    fixture.write(
        "Work/tasks.txt",
        "Painting depends on Sanding and drying the primer.\n",
    );
    reopened.scan(&root.id, &Observer::default()).unwrap();
    let refreshed = finish(&reopened);
    assert!(refreshed.revision > report.revision);
    assert_eq!(refreshed.predicates.iter().map(|p| p.1).sum::<u64>(), 2);
    assert!(
        refreshed
            .examples
            .iter()
            .all(|e| !e.detail.contains("Priming"))
    );
    reopened.remove(&root.id).unwrap();
    let empty = finish(&reopened);
    assert!(empty.kinds.is_empty() && empty.examples.is_empty() && empty.predicates.is_empty());
    assert!(graph.evidence(id).is_err());
}

#[test]
fn existing_catalogue_backfills_every_supported_relation_across_windows() {
    use std::fmt::Write as _;
    let fixture = Fixture::new();
    let mut rows = String::from("subject,predicate,object\n");
    for i in 0..155 {
        writeln!(rows, "Task {i},depends_on,Foundation").unwrap();
    }
    fixture.write("tasks.csv", &rows);
    let store = fixture.store();
    let root = store.add_root(&fixture.source()).unwrap();
    store.scan(&root.id, &Observer::default()).unwrap();
    {
        let db = store.connect().unwrap();
        db.execute_batch("DROP TRIGGER auto_file_added; DROP TRIGGER auto_file_changed; DROP TABLE index_auto_terms; DROP TABLE index_auto_findings; DROP TABLE index_auto_files; DROP TABLE index_auto_counts; DROP TABLE index_auto_state; PRAGMA user_version=15;").unwrap();
    }
    let reopened = fixture.store();
    let report = finish(&reopened);
    assert_eq!(report.predicates, [(RelationKind::DependsOn, 155)]);
    assert_eq!(
        report
            .counts
            .iter()
            .find(|(k, _)| *k == FindingKind::Relation)
            .unwrap()
            .1,
        155
    );
    assert!(
        report
            .examples
            .iter()
            .any(|e| e.line == 156 && e.detail.contains("Task 154"))
    );
    let auto = reopened.automatic().unwrap();
    let mut cursor = None;
    let mut lines = std::collections::BTreeSet::new();
    loop {
        let page = auto.findings(FindingKind::Relation, cursor).unwrap();
        assert!(page.items.len() <= 20);
        for item in page.items {
            assert!(lines.insert(item.line));
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(lines.len(), 155);
}

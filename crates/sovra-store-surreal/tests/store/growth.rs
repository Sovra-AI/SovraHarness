//! Opt-in disk growth measurements through the public storage port, without model calls.

use super::*;
use sovra_core::MemoryListQuery;
use std::time::Instant;

#[tokio::test]
#[ignore = "writes 5000 synthetic disk facts; set SOVRA_MEMORY_GROWTH_OUTPUT to a fresh directory"]
#[expect(
    clippy::disallowed_methods,
    reason = "opt-in test reads its explicit report destination once"
)]
async fn disk_growth_measurements() {
    let output = std::path::PathBuf::from(
        std::env::var("SOVRA_MEMORY_GROWTH_OUTPUT")
            .expect("set a new report directory outside the checkout"),
    );
    assert!(!output.exists());
    std::fs::create_dir_all(&output).unwrap();
    let store = SurrealStore::open(StoreConfig::on_disk(output.join("memory")))
        .await
        .unwrap();
    let target = store
        .save_fact(fact(
            "Mina prefers jasmine tea at breakfast",
            &["Mina"],
            None,
        ))
        .await
        .unwrap();
    let mut report = serde_json::json!({"suite":"disk-memory-growth-v1","engine":"SurrealKV",
        "samples_per_size":20,"model_calls":0,"status":"running","stages":[]});
    let mut inserted = 1;
    for size in [100, 1_000, 5_000] {
        let writes = Instant::now();
        while inserted < size {
            let name = format!("Colleague{inserted}");
            let drink = if inserted % 10 == 0 {
                "jasmine"
            } else {
                "rooibos"
            };
            store
                .save_fact(fact(
                    &format!("{name} prefers {drink} tea at breakfast"),
                    &[&name],
                    None,
                ))
                .await
                .unwrap();
            inserted += 1;
        }
        let write_ms = writes.elapsed().as_millis();
        let mut lexical = Vec::new();
        let mut entity = Vec::new();
        let mut pages = Vec::new();
        for _ in 0..20 {
            let started = Instant::now();
            let hits = store
                .recall(RecallQuery {
                    text: Some("Mina jasmine".into()),
                    limit: 24,
                    ..RecallQuery::recent(24)
                })
                .await
                .unwrap();
            lexical.push(started.elapsed().as_micros());
            assert!(hits.len() <= 24);
            assert!(hits.iter().any(|hit| hit.fact.id == target));
            let started = Instant::now();
            let hits = store
                .recall(RecallQuery {
                    text: Some("breakfast".into()),
                    entity: Some("Mina".into()),
                    ..RecallQuery::recent(24)
                })
                .await
                .unwrap();
            entity.push(started.elapsed().as_micros());
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].fact.id, target);
            let started = Instant::now();
            let page = store
                .list_memories(MemoryListQuery {
                    search: "breakfast".into(),
                    limit: 20,
                    ..Default::default()
                })
                .await
                .unwrap();
            pages.push(started.elapsed().as_micros());
            assert_eq!(page.facts.len(), 20);
            assert!(page.next.is_some());
        }
        report["stages"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
            "facts":size,"incremental_write_ms":write_ms,"lexical_us":lexical,
            "entity_us":entity,"first_page_us":pages}));
        std::fs::write(
            output.join("report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
    report["status"] = serde_json::json!("complete");
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

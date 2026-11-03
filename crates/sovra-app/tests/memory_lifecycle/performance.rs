//! Opt-in runtime measurements with synthetic evidence and a zero-cost loopback model.

use super::*;
use sovra_core::{MemoryInput, NewEpisode, Timestamp};
use std::sync::Mutex;

#[derive(Default)]
struct Measurements {
    sent: BTreeMap<String, Instant>,
    provider_us: BTreeMap<String, u128>,
    grounded: BTreeMap<String, bool>,
    extraction_calls: usize,
}

fn server(measurements: Arc<Mutex<Measurements>>) -> Server {
    Server::new(move |request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"memory-fixture"}]}));
        }
        let extraction = is_extraction(request);
        let mut measured = measurements.lock().unwrap();
        if extraction {
            measured.extraction_calls += 1;
        } else {
            let messages = request.body["messages"].as_array().unwrap();
            let prompt = messages
                .iter()
                .rev()
                .find_map(|message| {
                    (message["role"] == "user")
                        .then(|| message["content"].as_str())
                        .flatten()
                })
                .unwrap();
            let code = prompt.strip_prefix("Recall ").unwrap();
            let started = measured.sent[prompt];
            measured
                .provider_us
                .insert(prompt.into(), started.elapsed().as_micros());
            measured.grounded.insert(
                prompt.into(),
                messages.iter().any(|message| {
                    message["role"] == "system"
                        && message["content"]
                            .to_string()
                            .contains(&format!("My rehearsal code is {code}."))
                }),
            );
        }
        Reply::json(
            &json!({"id":"runtime-bench","object":"chat.completion","created":1,
            "model":"memory-fixture","choices":[{"index":0,"message":{"role":"assistant",
            "content":if extraction { "{\"memories\":[]}" } else { "Synthetic fixture reply." }},"finish_reason":"stop"}]}),
        )
    })
}

fn summary(values: &[u128]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    json!({"samples":sorted.len(),"p50_us":sorted[(sorted.len()*50).div_ceil(100)-1],
        "p95_us":sorted[(sorted.len()*95).div_ceil(100)-1],"values_us":values})
}

fn seed(fixture: &Fixture, size: usize, copied: bool) {
    let config = StoreConfig::on_disk(fixture.root.join("memory"));
    let client = sovra_runtime::start_with_config(
        "synthetic-runtime-seed",
        AgentConfig::default(),
        Some(Box::new(move || {
            Box::pin(async move {
                let store = SurrealStore::open(config).await?;
                if !copied {
                    for index in 0..size {
                        store
                            .enqueue_memory(MemoryInput {
                                key: "one".into(),
                                episode: NewEpisode {
                                    session_id: format!("synthetic-{index}"),
                                    kind: EpisodeKind::UserMessage,
                                    content: format!("My rehearsal code is fixturecode{index}."),
                                    occurred_at: Timestamp(1),
                                },
                            })
                            .await?;
                    }
                }
                // Isolate retrieval from a historical extraction storm. New runtime
                // inputs still exercise capture and all four normal extraction slots.
                store.set_memory_enabled(false).await?;
                store.set_memory_enabled(true).await?;
                Ok(Arc::new(store) as Arc<dyn MemoryStore>)
            })
        })),
    )
    .unwrap();
    let mut driver = Driver::new(client);
    driver.timeout = Duration::from_secs(7_200);
    driver.wait(|event| matches!(event, Event::Ready { .. }));
    driver.close();
}

fn measure(fixture: &Fixture, measurements: &Mutex<Measurements>, concurrency: usize) -> Value {
    let mut driver = fixture.start();
    let mut acknowledgements = Vec::new();
    let mut completions = Vec::new();
    let started = Instant::now();
    let mut prompts = Vec::new();
    for batch in 0..(64 / concurrency) {
        let mut sessions = Vec::new();
        for request in 0..concurrency {
            let index = concurrency * 100 + batch * concurrency + request;
            let prompt = format!("Recall fixturecode{}", index % 1_000);
            let session = format!("runtime-bench-{concurrency}-{index}");
            let sent = Instant::now();
            measurements
                .lock()
                .unwrap()
                .sent
                .insert(prompt.clone(), sent);
            driver.send(Command::RunPrompt {
                session_id: session.clone(),
                prompt: prompt.clone(),
                context: None,
            });
            sessions.push((session, sent));
            prompts.push(prompt);
        }
        let mut jobs = Vec::new();
        for (session, sent) in &sessions {
            let Event::Memory(MemoryEvent::Queued { episode_id, .. }) = driver.wait(|event| matches!(event,Event::Memory(MemoryEvent::Queued {session_id,..}) if session_id==session)) else { unreachable!() };
            acknowledgements.push(sent.elapsed().as_micros());
            jobs.push(episode_id);
        }
        for (session, sent) in &sessions {
            driver.wait(
                |event| matches!(event,Event::AgentCompleted {session_id} if session_id==session),
            );
            completions.push(sent.elapsed().as_micros());
        }
        // Drain each batch so the next one starts with the same queue condition.
        for episode in jobs {
            let Event::Memory(MemoryEvent::Processed(job)) = driver.wait(|event| matches!(event, Event::Memory(MemoryEvent::Processed(job)) if job.episode_id == episode && matches!(job.state, MemoryJobState::Complete | MemoryJobState::Failed | MemoryJobState::Cancelled))) else { unreachable!() };
            assert_eq!(job.state, MemoryJobState::Complete);
        }
    }
    let elapsed_us = started.elapsed().as_micros();
    let MemoryReviewResult::Page(page) =
        driver.review(MemoryAction::List(MemoryListQuery::default()))
    else {
        unreachable!()
    };
    let overview = page.overview.unwrap();
    let measured = measurements.lock().unwrap();
    let latency = prompts
        .iter()
        .map(|prompt| measured.provider_us[prompt])
        .collect::<Vec<_>>();
    let grounded = prompts
        .iter()
        .filter(|prompt| measured.grounded[*prompt])
        .count();
    let result = json!({"concurrency":concurrency,"requests":prompts.len(),"elapsed_us":elapsed_us,
        "dispatch_to_provider":summary(&latency),"client_observed_capture_ack":summary(&acknowledgements),
        "client_observed_completion":summary(&completions),"grounded_requests":grounded,
        "pending_at_end":overview.waiting,"working_at_end":overview.working,
        "extraction_calls_cumulative":measured.extraction_calls});
    drop(measured);
    driver.close();
    result
}

#[test]
#[ignore = "isolated 1k/10k/100k runtime benchmark; set SOVRA_RUNTIME_BENCH_OUTPUT"]
#[expect(
    clippy::disallowed_methods,
    reason = "opt-in benchmark reads explicit fixture destinations once"
)]
fn synthetic_runtime_retrieval_measurements() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("sovra_store_surreal=debug,sovra_runtime::remembering=debug")
        .with_test_writer()
        .try_init();
    let output =
        PathBuf::from(std::env::var("SOVRA_RUNTIME_BENCH_OUTPUT").expect("fresh report directory"));
    let seed_path = std::env::var_os("SOVRA_RUNTIME_BENCH_SEED").map(PathBuf::from);
    assert!(!output.exists());
    std::fs::create_dir_all(&output).unwrap();
    let mut report = json!({"suite":"synthetic-memory-runtime-v1","status":"running",
        "engine":"SurrealDB 3.3.4 / SurrealKV","model":"local deterministic HTTP fixture",
        "embedding_service":false,"model_cost_usd":0,"provider_tokens":0,
        "query_counts":null,"transaction_conflicts":null,"oldest_queue_age_ms":null,
        "limitations":["Dispatch-to-provider includes command handling, retrieval and loopback HTTP overhead, not model inference.",
            "Capture/completion durations are client-observed upper bounds; events can wait behind an earlier request in the batch.",
            "Historical fixture extraction is cancelled before measurement; newly captured queries use normal workers.",
            "SQL query counts, transaction conflicts and oldest queue age are not instrumented.",
            "Synthetic source-only evidence; no answer-quality or semantic-retrieval score."],"stages":[]});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    for size in [1_000, 10_000, 100_000] {
        let measured = Arc::new(Mutex::new(Measurements::default()));
        let fixture = Fixture::with_server(server(measured.clone()));
        let copied = size == 100_000 && seed_path.is_some();
        if let Some(path) = seed_path.as_ref().filter(|_| copied) {
            let manifest: Value =
                serde_json::from_slice(&std::fs::read(path.join("report.json")).unwrap()).unwrap();
            assert_eq!(manifest["suite"], "source-storage-v1");
            assert_eq!(manifest["status"], "complete");
            assert_eq!(
                manifest["stages"].as_array().unwrap().last().unwrap()["sources"],
                100_000
            );
            copy_directory(&path.join("database"), &fixture.root.join("memory"));
        }
        let started = Instant::now();
        seed(&fixture, size, copied);
        let prepare_us = started.elapsed().as_micros();
        let stages = [1, 4, 16].map(|concurrency| measure(&fixture, &measured, concurrency));
        report["stages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"initial_sources":size,
            "reused_synthetic_seed":copied,"prepare_us":prepare_us,"requests":stages}));
        std::fs::write(
            output.join("report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
    let grounded = report["stages"].as_array().unwrap().iter().all(|stage| {
        stage["requests"]
            .as_array()
            .unwrap()
            .iter()
            .all(|run| run["grounded_requests"] == run["requests"])
    });
    report["all_expected_evidence_retrieved"] = json!(grounded);
    report["status"] = json!("complete");
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(
        grounded,
        "some expected source evidence did not reach the provider; see report"
    );
}

fn copy_directory(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

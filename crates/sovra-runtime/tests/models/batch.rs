//! Batch catalog entries must never reach the live chat transport.

use super::*;

const BATCH: &str = "google/gemini-3.8-flash:batch";

#[tokio::test]
async fn batch_activation_is_rejected_without_replacing_the_working_choice() {
    let server = Server::new(|_| {
        Reply::json(&json!({"data":[
            {"id":"live", "architecture":{"output_modalities":["text"]}},
            {"id":BATCH, "architecture":{"output_modalities":["text"]}, "supported_parameters":["tools"]}
        ]}))
    });
    let mut connection = ModelConnection::new("router", ProviderKind::OpenRouter);
    connection.base_url = format!("{}/api/v1", server.url());
    connection.has_credential = true;
    let fixture = Fixture::new(connection, Some("test-only"));
    let client = fixture.start();
    let snapshot = catalog(&client).await;
    assert_eq!(snapshot.models.len(), 2, "retain the full catalog");
    let live = ModelKey {
        connection_id: "router".into(),
        model_id: "live".into(),
    };
    send(&client, ModelCommand::Activate { key: live.clone() });
    let activated = event(&client, |e| {
        matches!(
            e,
            Event::Models(ModelEvent::PreferencesChanged(_) | ModelEvent::Error(_))
        )
    })
    .await;
    assert!(
        matches!(&activated, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref() == Some(&live)),
        "{activated:?}"
    );
    send(
        &client,
        ModelCommand::Activate {
            key: ModelKey {
                connection_id: "router".into(),
                model_id: BATCH.into(),
            },
        },
    );
    let result = event(&client, |e| {
        matches!(
            e,
            Event::Models(ModelEvent::Error(_) | ModelEvent::PreferencesChanged(_))
        )
    })
    .await;
    assert!(
        matches!(result, Event::Models(ModelEvent::Error(message)) if message.contains("batch") && message.contains("Models"))
    );
    assert_eq!(fixture.repo.load().unwrap().preferences.active, Some(live));
    stop(client).await;
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.path.contains("/models"))
    );
}

#[tokio::test]
async fn batch_saved_choices_are_unavailable_from_cache_and_manual_entries() {
    for manual in [false, true] {
        let server = Server::new(|_| Reply::json(&json!({"data":[]})));
        let mut connection = ModelConnection::new("router", ProviderKind::OpenRouter);
        connection.base_url = format!("{}/api/v1", server.url());
        connection.has_credential = true;
        let model = ModelInfo {
            id: BATCH.into(),
            manual,
            output_modalities: if manual { vec![] } else { vec!["text".into()] },
            ..Default::default()
        };
        if manual {
            connection.manual_models.push(model.clone());
        }
        let fixture = Fixture::new(connection, Some("test-only"));
        fixture
            .repo
            .save_catalog(
                "router",
                &CatalogSnapshot {
                    models: if manual { vec![] } else { vec![model.clone()] },
                    fetched_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                },
            )
            .unwrap();
        let mut saved = fixture.repo.load().unwrap();
        saved.preferences.active = Some(model.key("router"));
        fixture.repo.save(&saved).unwrap();
        let client = fixture.start();
        let result = event(&client, |e| {
            matches!(e, Event::Models(ModelEvent::ActiveModelStatus { .. }))
        })
        .await;
        assert!(
            matches!(result, Event::Models(ModelEvent::ActiveModelStatus { problem: Some(message) }) if message.contains("batch") && message.contains("Models"))
        );
        stop(client).await;
        assert_eq!(
            server.requests.lock().unwrap().len(),
            0,
            "saved batch choices must fail before network IO"
        );
    }
}

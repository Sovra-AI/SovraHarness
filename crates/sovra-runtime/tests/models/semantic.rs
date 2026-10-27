//! Real HTTP and Client lifecycle; storage/ranking integrity has adapter tests.
use super::*;
use sovra_core::file_index::semantic::*;
use sovra_core::file_index::*;

#[tokio::test]
async fn router_embedding_transport_preserves_order_and_revalidates_consent() {
    let server = Server::new(|request| {
        assert_eq!(request.path, "/v1/embeddings");
        assert!(
            request
                .headers
                .to_ascii_lowercase()
                .contains("authorization: bearer fake-test-key")
        );
        assert_eq!(
            request.body,
            json!({"model":"search-helper", "input":["A jet crosses the ocean.", "A train crosses the valley."], "encoding_format":"float"})
        );
        Reply::json(&json!({"data":[{"index":1,"embedding":[0,3]},{"index":0,"embedding":[2,0]}]}))
    });
    let mut connection = ModelConnection::new("router", ProviderKind::OpenRouter);
    connection.base_url = format!("{}/v1", server.url());
    connection.has_credential = true;
    let model = ModelInfo {
        id: "search-helper".into(),
        output_modalities: vec!["embeddings".into()],
        ..Default::default()
    };
    let target = EmbeddingTarget::for_embeddings(&connection, &model).unwrap();
    connection.manual_models.push(model);
    let fixture = Fixture::new(connection, Some("fake-test-key"));
    let embedder = sovra_providers::HttpEmbedder::from_repository(&fixture.repo, &target).unwrap();
    let vectors = embedder
        .embed(&[
            "A jet crosses the ocean.".into(),
            "A train crosses the valley.".into(),
        ])
        .await
        .unwrap();
    assert_eq!(vectors[0].values(), &[1., 0.]);
    assert_eq!(vectors[1].values(), &[0., 1.]);
    let mut saved = fixture.repo.load().unwrap();
    saved.connections[0].base_url.push_str("/changed");
    fixture.repo.save(&saved).unwrap();
    assert!(sovra_providers::HttpEmbedder::from_repository(&fixture.repo, &target).is_err());
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[derive(Debug, Default)]
struct SemanticState {
    generation: u64,
    setting: Option<SemanticSetting>,
    vector: Option<EmbeddingVector>,
}
#[derive(Debug, Default)]
struct Index(Mutex<SemanticState>);

fn root() -> IndexedRoot {
    IndexedRoot {
        id: "folder".into(),
        path: "/chosen".into(),
        status: IndexStatus::Ready,
        progress: IndexProgress::default(),
        issues: vec![],
        completed_at: Some(1),
        auto_refresh: false,
    }
}
fn passage() -> IndexPassage {
    IndexPassage {
        reference: PassageRef {
            root_id: "folder".into(),
            file_id: "file".into(),
            version_id: "version".into(),
            ordinal: 0,
        },
        path: "travel.txt".into(),
        text: "A jet crosses the ocean.".into(),
        start_line: 1,
        end_line: 1,
        mime_type: "text/plain".into(),
        note: None,
        observed_at: 1,
        modified_at: None,
    }
}
fn unused() -> IndexError {
    IndexError("Not used by this semantic fixture.".into())
}

impl FileIndex for Index {
    fn semantic(&self) -> Option<&dyn SemanticIndex> {
        Some(self)
    }
    fn list(&self) -> Result<Vec<IndexedRoot>, IndexError> {
        Ok(vec![root()])
    }
    fn recover(&self) -> Result<(), IndexError> {
        Ok(())
    }
    fn add_root(&self, _: &std::path::Path) -> Result<IndexedRoot, IndexError> {
        Err(unused())
    }
    fn scan(&self, _: &str, _: &dyn ScanObserver) -> Result<IndexedRoot, IndexError> {
        Err(unused())
    }
    fn set_auto_refresh(&self, _: &str, _: bool) -> Result<(), IndexError> {
        Ok(())
    }
    fn watch(
        &self,
        _: &IndexedRoot,
        _: Arc<dyn IndexChanges>,
    ) -> Result<Box<dyn IndexWatch>, IndexError> {
        Err(unused())
    }
    fn remove(&self, _: &str) -> Result<(), IndexError> {
        self.0.lock().unwrap().setting = None;
        Ok(())
    }
    fn search(&self, query: &IndexQuery) -> Result<Vec<IndexPassage>, IndexError> {
        query.validate()?;
        Ok(if query.text == "jet" {
            vec![passage()]
        } else {
            vec![]
        })
    }
    fn read(&self, r: &PassageRef) -> Result<IndexPassage, IndexError> {
        if *r == passage().reference {
            Ok(passage())
        } else {
            Err(unused())
        }
    }
    fn catalogue(&self, _: &CatalogQuery) -> Result<CatalogPage, IndexError> {
        Err(unused())
    }
    fn history(&self, _: &FileHistoryQuery) -> Result<FileHistory, IndexError> {
        Err(unused())
    }
    fn read_history(&self, _: &HistoryPassageRef) -> Result<HistoricalPassage, IndexError> {
        Err(unused())
    }
}
impl SemanticIndex for Index {
    fn settings(&self) -> Result<Vec<SemanticSetting>, IndexError> {
        Ok(self.0.lock().unwrap().setting.clone().into_iter().collect())
    }
    fn configure(&self, root: &str, target: Option<&EmbeddingTarget>) -> Result<(), IndexError> {
        let mut state = self.0.lock().unwrap();
        state.generation += 1;
        state.setting = target.map(|target| SemanticSetting {
            id: state.generation,
            root_id: root.into(),
            target: target.clone(),
        });
        state.vector = None;
        Ok(())
    }
    fn status(&self, setting: &SemanticSetting) -> Result<SemanticStatus, IndexError> {
        let state = self.0.lock().unwrap();
        if state.setting.as_ref() != Some(setting) {
            return Err(unused());
        }
        Ok(SemanticStatus {
            setting: setting.clone(),
            total: 1,
            embedded: u64::from(state.vector.is_some()),
            message: None,
        })
    }
    fn pending(&self, setting: &SemanticSetting) -> Result<Vec<EmbeddingInput>, IndexError> {
        Ok(if self.status(setting)?.embedded == 0 {
            vec![EmbeddingInput {
                chunk_id: "chunk".into(),
                text: passage().text,
            }]
        } else {
            vec![]
        })
    }
    fn commit(
        &self,
        setting: &SemanticSetting,
        _: &[EmbeddingInput],
        vectors: &[EmbeddingVector],
    ) -> Result<(), IndexError> {
        let mut state = self.0.lock().unwrap();
        if state.setting.as_ref() != Some(setting) {
            return Err(unused());
        }
        state.vector = Some(vectors[0].clone());
        Ok(())
    }
    fn nearest(
        &self,
        setting: &SemanticSetting,
        _: &IndexQuery,
        vector: &EmbeddingVector,
    ) -> Result<Vec<IndexPassage>, IndexError> {
        self.status(setting)?;
        let state = self.0.lock().unwrap();
        Ok(
            if state
                .vector
                .as_ref()
                .is_some_and(|v| v.cosine(vector).unwrap() > 0.9)
            {
                vec![passage()]
            } else {
                vec![]
            },
        )
    }
}

fn start(fixture: &Fixture, index: Arc<Index>) -> Client {
    sovra_runtime::start_with_file_index(
        "test",
        AgentConfig::default(),
        None,
        Some(ModelsConfig {
            repository: fixture.repo.clone(),
            providers: vec![Arc::new(OllamaProvider), Arc::new(CompatibleProvider)],
        }),
        None,
        None,
        Some(index),
    )
    .unwrap()
}
fn query(client: &Client, id: u64, text: &str) {
    client
        .send(Command::FileIndex(IndexCommand::Search {
            request_id: id,
            query: IndexQuery {
                text: text.into(),
                roots: vec![],
                path_prefix: None,
                limit: 10,
            },
        }))
        .unwrap();
}
fn configure(client: &Client, target: Option<EmbeddingTarget>) {
    client
        .send(Command::FileIndex(IndexCommand::ConfigureSemantic {
            root_id: "folder".into(),
            target,
        }))
        .unwrap();
}

async fn transport(kind: ProviderKind) {
    let server = Server::new(move |request| {
        if request.path.ends_with("/embed") || request.path.ends_with("/embeddings") {
            assert!(
                request
                    .headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer embedding-test-key")
            );
            assert_eq!(request.body["model"], "embed-v1");
            let count = request.body["input"].as_array().unwrap().len();
            if kind == ProviderKind::Ollama {
                assert_eq!(request.path, "/api/embed");
                assert_eq!(request.body["truncate"], false);
                return Reply::json(&json!({"embeddings":vec![vec![1.,0.];count]}));
            }
            assert_eq!(request.path, "/v1/embeddings");
            assert_eq!(request.body["encoding_format"], "float");
            return Reply::json(
                &json!({"data":(0..count).rev().map(|index|json!({"index":index,"embedding":[1.,0.]})).collect::<Vec<_>>()}),
            );
        }
        Reply::json(&json!({"models":[],"data":[]}))
    });
    let mut connection = ModelConnection::new("embedding", kind);
    connection.base_url = format!(
        "{}{}",
        server.url(),
        if kind == ProviderKind::Ollama {
            ""
        } else {
            "/v1"
        }
    );
    connection.authentication = Authentication::Bearer;
    connection.has_credential = true;
    let model = ModelInfo {
        id: "embed-v1".into(),
        manual: true,
        output_modalities: vec!["embeddings".into()],
        ..ModelInfo::default()
    };
    let target = EmbeddingTarget::for_embeddings(&connection, &model).unwrap();
    connection.manual_models.push(model);
    let fixture = Fixture::new(connection, Some("embedding-test-key"));
    let client = start(&fixture, Arc::new(Index::default()));
    event(
        &client,
        |e| matches!(e,Event::FileIndex(IndexEvent::Semantic(s))if s.is_empty()),
    )
    .await;
    query(&client, 1, "aircraft");
    event(&client,|e|matches!(e,Event::FileIndex(IndexEvent::Results{request_id:1,passages})if passages.is_empty())).await;
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.path.ends_with("embed") || r.path.ends_with("embeddings"))
    );
    configure(&client, Some(target));
    event(&client,|e|matches!(e,Event::FileIndex(IndexEvent::Semantic(s))if s.first().is_some_and(|s|s.embedded==1))).await;
    query(&client, 2, "aircraft");
    event(&client,|e|matches!(e,Event::FileIndex(IndexEvent::Results{request_id:2,passages})if passages==&vec![passage()])).await;
    let count = server.requests.lock().unwrap().len();
    let mut saved = fixture.repo.load().unwrap();
    saved.connections[0].base_url.push_str("/changed");
    fixture.repo.save(&saved).unwrap();
    query(&client, 3, "jet");
    event(&client,|e|matches!(e,Event::FileIndex(IndexEvent::Results{request_id:3,passages})if passages==&vec![passage()])).await;
    event(&client,|e|matches!(e,Event::FileIndex(IndexEvent::SearchNotice{request_id:3,message})if message.contains("destination changed"))).await;
    assert_eq!(server.requests.lock().unwrap().len(), count);
    configure(&client, None);
    event(
        &client,
        |e| matches!(e,Event::FileIndex(IndexEvent::Semantic(s))if s.is_empty()),
    )
    .await;
    query(&client, 4, "aircraft");
    event(&client,|e|matches!(e,Event::FileIndex(IndexEvent::Results{request_id:4,passages})if passages.is_empty())).await;
    stop(client).await;
}

#[tokio::test]
async fn ollama_embeddings_require_folder_consent_and_changed_endpoints_stop_requests() {
    transport(ProviderKind::Ollama).await;
}
#[tokio::test]
async fn compatible_embeddings_require_folder_consent_and_changed_endpoints_stop_requests() {
    transport(ProviderKind::OpenAiCompatible).await;
}

#[tokio::test]
async fn disabling_during_an_embedding_request_discards_the_response() {
    let (started, received) = std::sync::mpsc::channel();
    let (release, waiting) = std::sync::mpsc::channel();
    let waiting = Arc::new(Mutex::new(waiting));
    let server = Server::new(move |request| {
        if request.path == "/api/embed" {
            started.send(()).unwrap();
            return Reply {
                status: 200,
                content_type: "application/json",
                body: "{\"embeddings\":".into(),
                tail: Some((waiting.clone(), "[[1,0]]}".into())),
            };
        }
        Reply::json(&json!({"models":[]}))
    });
    let mut connection = ModelConnection::new("embedding", ProviderKind::Ollama);
    connection.base_url = server.url();
    let model = ModelInfo {
        id: "embed".into(),
        manual: true,
        output_modalities: vec!["embeddings".into()],
        ..ModelInfo::default()
    };
    let target = EmbeddingTarget::for_embeddings(&connection, &model).unwrap();
    connection.manual_models.push(model);
    let fixture = Fixture::new(connection, None);
    let index = Arc::new(Index::default());
    let client = start(&fixture, index.clone());
    configure(&client, Some(target));
    // Drain the initial empty status before waiting for the disable acknowledgement.
    // Otherwise a startup event can masquerade as completion of the later change.
    event(&client, |event| {
        matches!(event, Event::FileIndex(IndexEvent::Semantic(status)) if !status.is_empty())
    })
    .await;
    tokio::task::spawn_blocking(move || received.recv_timeout(Duration::from_secs(10)).unwrap())
        .await
        .unwrap();
    configure(&client, None);
    event(
        &client,
        |e| matches!(e,Event::FileIndex(IndexEvent::Semantic(s))if s.is_empty()),
    )
    .await;
    release.send(()).unwrap();
    query(&client, 10, "aircraft");
    event(&client,|e|matches!(e,Event::FileIndex(IndexEvent::Results{request_id:10,passages})if passages.is_empty())).await;
    assert!(index.0.lock().unwrap().vector.is_none());
    assert!(index.0.lock().unwrap().setting.is_none());
    stop(client).await;
}

#[tokio::test]
async fn discovered_embedding_capabilities_are_saved_before_the_model_can_be_chosen() {
    let first_show = AtomicBool::new(true);
    let server = Server::new(move |request| match request.path.as_str() {
        "/api/tags" => Reply::json(
            &json!({"models":[{"model":"embed-v1","modified_at":"2026-10-25T00:00:00Z"}]}),
        ),
        "/api/show" if first_show.swap(false, Ordering::SeqCst) => Reply {
            status: 503,
            content_type: "application/json",
            body: "{}".into(),
            tail: None,
        },
        "/api/show" => Reply::json(
            &json!({"capabilities":["embedding"],"model_info":{"general.architecture":"bert","bert.context_length":8192}}),
        ),
        _ => Reply::json(&json!({"embeddings":[[1,0]]})),
    });
    let mut connection = ModelConnection::new("discovered", ProviderKind::Ollama);
    connection.base_url = server.url();
    let fixture = Fixture::new(connection.clone(), None);
    let client = fixture.start();
    let snapshot = catalog(&client).await;
    assert!(EmbeddingTarget::for_embeddings(&connection, &snapshot.models[0]).is_none());
    send(
        &client,
        ModelCommand::FetchDetails {
            key: ModelKey {
                connection_id: "discovered".into(),
                model_id: "embed-v1".into(),
            },
        },
    );
    let loaded = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::DetailsLoaded { .. }))
    })
    .await;
    let Event::Models(ModelEvent::DetailsLoaded { model, .. }) = loaded else {
        unreachable!()
    };
    let target = EmbeddingTarget::for_embeddings(&connection, &model).unwrap();
    assert!(sovra_providers::HttpEmbedder::from_repository(&fixture.repo, &target).is_ok());
    stop(client).await;
}

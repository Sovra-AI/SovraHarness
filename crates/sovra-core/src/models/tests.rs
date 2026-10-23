use super::*;

fn fixture() -> ModelsState {
    let paid = ModelInfo {
        id: "author/vision".into(),
        name: "Vision Reasoner".into(),
        author: "author".into(),
        description: "Multilingual engineering assistant".into(),
        context_length: Some(128_000),
        input_modalities: vec!["text".into(), "image".into()],
        output_modalities: vec!["text".into()],
        supported_parameters: vec!["tools".into(), "reasoning".into(), "response_format".into()],
        pricing: [
            ("prompt".into(), "0.000002".into()),
            ("completion".into(), "0.000006".into()),
        ]
        .into(),
        created: Some(20),
        ..ModelInfo::default()
    };
    let free = ModelInfo {
        id: "author/free".into(),
        name: "Free Small".into(),
        author: "author".into(),
        context_length: Some(8_192),
        output_modalities: vec!["text".into()],
        pricing: [
            ("prompt".into(), "0".into()),
            ("completion".into(), "0".into()),
        ]
        .into(),
        created: Some(10),
        ..ModelInfo::default()
    };
    let image = ModelInfo {
        id: "other/image".into(),
        name: "Image Maker".into(),
        author: "other".into(),
        output_modalities: vec!["image".into()],
        pricing: [
            ("prompt".into(), "0".into()),
            ("image".into(), "0.04".into()),
        ]
        .into(),
        ..ModelInfo::default()
    };
    let mut state = ModelsState::default();
    state.apply(&ModelEvent::Loaded {
        connections: vec![
            ModelConnection::new("router", ProviderKind::OpenRouter),
            ModelConnection::new("local", ProviderKind::Ollama),
        ],
        preferences: ModelPreferences::default(),
    });
    state.apply(&ModelEvent::CatalogLoaded {
        connection_id: "router".into(),
        snapshot: CatalogSnapshot {
            models: vec![paid.clone(), free, image],
            fetched_at: 123,
        },
        cached: false,
    });
    state.apply(&ModelEvent::CatalogLoaded {
        connection_id: "local".into(),
        snapshot: CatalogSnapshot {
            models: vec![ModelInfo {
                pricing: BTreeMap::new(),
                ..paid
            }],
            fetched_at: 123,
        },
        cached: false,
    });
    state
}

#[test]
fn filters_compose_and_keep_all_modalities_by_default() {
    let state = fixture();
    assert_eq!(state.filtered(&ModelFilter::default()).len(), 4);
    let filter = ModelFilter {
        search: "VISION engineering".into(),
        scope: CatalogScope::OpenRouter,
        author: Some("author".into()),
        input_modality: Some("image".into()),
        output_modality: Some("text".into()),
        min_context: 100_000,
        tools: true,
        reasoning: true,
        structured_output: true,
        price: PriceFilter::Paid,
        ..ModelFilter::default()
    };
    assert_eq!(
        state.filtered(&filter),
        vec![ModelKey {
            connection_id: "router".into(),
            model_id: "author/vision".into()
        }]
    );
    assert_eq!(
        state.filtered(&ModelFilter {
            min_context: 200_000,
            ..filter
        }),
        vec![]
    );
}

#[test]
fn facets_preserve_new_modalities_and_connection_filters_are_exact() {
    let mut state = fixture();
    state.catalogs.get_mut("local").unwrap().snapshot.models[0]
        .output_modalities
        .push("future-kind".into());
    let local = state.facets(CatalogScope::Ollama);
    assert!(local.output_modalities.contains("future-kind"));
    assert!(
        !state
            .facets(CatalogScope::OpenRouter)
            .output_modalities
            .contains("future-kind")
    );
    assert!(state.facets(CatalogScope::Favorites).authors.is_empty());
    let rows = state.filtered(&ModelFilter {
        connection_id: Some("local".into()),
        ..ModelFilter::default()
    });
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].connection_id, "local");
    assert_eq!(
        state
            .filtered(&ModelFilter {
                connection_id: Some("missing".into()),
                ..ModelFilter::default()
            })
            .len(),
        0
    );
}

#[test]
fn unknown_and_non_token_prices_are_not_free() {
    let state = fixture();
    let free = state.filtered(&ModelFilter {
        price: PriceFilter::Free,
        ..ModelFilter::default()
    });
    assert_eq!(free.len(), 1);
    assert_eq!(free[0].model_id, "author/free");
    let rows = state.filtered(&ModelFilter {
        sort: ModelSort::InputPrice,
        ..ModelFilter::default()
    });
    assert_eq!(rows.last().unwrap().connection_id, "local");
    let invalid = ModelInfo {
        pricing: [("prompt".into(), "NaN".into())].into(),
        ..ModelInfo::default()
    };
    assert_eq!(invalid.token_price("prompt"), None);
    assert!(!invalid.is_free());
}

#[test]
fn favorites_and_selection_are_scoped_to_connection() {
    let mut state = fixture();
    let key = ModelKey {
        connection_id: "local".into(),
        model_id: "author/vision".into(),
    };
    state.apply(&ModelEvent::PreferencesChanged(ModelPreferences {
        favorites: [key.clone()].into(),
        active: Some(key.clone()),
    }));
    assert_eq!(
        state.filtered(&ModelFilter {
            scope: CatalogScope::Favorites,
            ..ModelFilter::default()
        }),
        vec![key]
    );
    state.apply(&ModelEvent::ConnectionDeleted { id: "local".into() });
    assert_eq!(state.preferences.active, None);
    assert_eq!(state.preferences.favorites.len(), 0);
    assert_eq!(state.filtered(&ModelFilter::default()).len(), 3);
}

#[test]
fn failed_refresh_preserves_cached_results_and_details_enrich_one_model() {
    let mut state = fixture();
    state.apply(&ModelEvent::CatalogLoading {
        connection_id: "local".into(),
    });
    state.apply(&ModelEvent::CatalogFailed {
        connection_id: "local".into(),
        message: "Server offline".into(),
    });
    assert_eq!(state.catalogs["local"].snapshot.models.len(), 1);
    assert!(!state.catalogs["local"].loading);
    let key = ModelKey {
        connection_id: "local".into(),
        model_id: "author/vision".into(),
    };
    let mut model = state.model(&key).unwrap().clone();
    model.quantization = Some("Q4_K_M".into());
    state.apply(&ModelEvent::DetailsLoaded {
        key: key.clone(),
        model: Box::new(model),
    });
    assert_eq!(
        state.model(&key).unwrap().quantization.as_deref(),
        Some("Q4_K_M")
    );
    assert!(state.model(&key).unwrap().can_chat());
}

#[test]
fn defaults_and_operation_events_are_consistent_and_secrets_are_redacted() {
    for kind in [
        ProviderKind::OpenRouter,
        ProviderKind::Ollama,
        ProviderKind::OpenAiCompatible,
        ProviderKind::Anthropic,
    ] {
        let connection = ModelConnection::new("id", kind);
        assert_eq!(connection.name, kind.label());
        assert_eq!(connection.base_url, kind.default_url());
    }
    let secret = ModelSecret("never-log-this".into());
    assert_eq!(format!("{secret:?}"), "[REDACTED]");
    let mut state = ModelsState::default();
    state.apply(&ModelEvent::OperationStarted);
    assert!(state.busy);
    state.apply(&ModelEvent::Error("No connection".into()));
    assert!(!state.busy);
    state.apply(&ModelEvent::Notice("Saved".into()));
    assert_eq!(state.error, None);
    state.apply(&ModelEvent::PullProgress(Some("Downloading".into())));
    assert_eq!(state.pull_progress.as_deref(), Some("Downloading"));
    state.apply(&ModelEvent::PullProgress(None));
    assert_eq!(state.pull_progress, None);
}

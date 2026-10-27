use super::*;

#[test]
fn starter_recommendation_never_picks_paid_unknown_or_non_chat_models() {
    let mut state = fixture();
    assert!(
        state.recommended_chat_model().is_none(),
        "requires a connected online account"
    );
    state.connections[0].has_credential = true;
    assert_eq!(
        state
            .recommended_chat_model()
            .expect("free chat choice")
            .model_id,
        "author/free"
    );
    let catalog = &mut state
        .catalogs
        .get_mut("router")
        .expect("catalog")
        .snapshot
        .models;
    catalog[1].pricing.remove("completion");
    assert!(
        state.recommended_chat_model().is_none(),
        "unknown reply cost is not free"
    );
    let free = &mut state
        .catalogs
        .get_mut("router")
        .expect("catalog")
        .snapshot
        .models[1];
    free.pricing.insert("completion".into(), "0".into());
    free.output_modalities = vec!["image".into()];
    assert!(
        state.recommended_chat_model().is_none(),
        "image models cannot answer chats"
    );
    state
        .catalogs
        .get_mut("router")
        .expect("catalog")
        .snapshot
        .models[1]
        .output_modalities
        .clear();
    assert!(
        state.recommended_chat_model().is_none(),
        "unknown capabilities need explicit user choice"
    );
}

#[test]
fn starter_recommendation_is_stable_across_catalog_order() {
    let mut state = fixture();
    state.connections[0].has_credential = true;
    let catalog = &mut state
        .catalogs
        .get_mut("router")
        .expect("catalog")
        .snapshot
        .models;
    let mut router = catalog[1].clone();
    router.id = "openrouter/free".into();
    catalog.push(router);
    catalog.reverse();
    let expected = ModelKey {
        connection_id: "router".into(),
        model_id: "openrouter/free".into(),
    };
    assert_eq!(state.recommended_chat_model(), Some(expected.clone()));
    state
        .catalogs
        .get_mut("router")
        .expect("catalog")
        .snapshot
        .models
        .reverse();
    assert_eq!(state.recommended_chat_model(), Some(expected));
}

#[test]
fn everyday_choice_is_the_newest_affordable_tool_capable_paid_model() {
    let mut state = fixture();
    state.connections[0].has_credential = true;
    let mut old_name = state.catalogs["router"].snapshot.models[0].clone();
    old_name.id = "anthropic/claude-3.5-sonnet".into();
    old_name.supported_parameters = vec!["tools".into()];
    old_name.created = Some(1);
    let mut newer = old_name.clone();
    newer.id = "example/current-choice".into();
    newer.created = Some(50);
    let mut too_expensive = newer.clone();
    too_expensive.id = "example/too-expensive".into();
    too_expensive.created = Some(99);
    too_expensive.pricing = [
        ("prompt".into(), "0.0001".into()),
        ("completion".into(), "0.0001".into()),
    ]
    .into();
    let mut no_tools = newer.clone();
    no_tools.id = "example/no-tools".into();
    no_tools.created = Some(98);
    no_tools.supported_parameters.clear();
    let mut unknown = newer.clone();
    unknown.id = "aaa/unknown-price".into();
    unknown.created = Some(97);
    unknown.pricing.clear();
    let mut no_capabilities = newer.clone();
    no_capabilities.id = "aaa/unknown-capabilities".into();
    no_capabilities.created = Some(96);
    no_capabilities.output_modalities.clear();
    state
        .catalogs
        .get_mut("router")
        .expect("catalog")
        .snapshot
        .models
        .extend([
            old_name,
            newer,
            too_expensive,
            no_tools,
            unknown,
            no_capabilities,
        ]);
    let choice = state
        .recommended_model_cards()
        .into_iter()
        .find(|card| card.kind == RecommendationKind::EverydaySmart)
        .expect("known text model");
    assert_eq!(choice.key.model_id, "example/current-choice");
}

#[test]
fn prices_read_as_cost_per_message() {
    let priced = |prompt: &str, completion: &str| ModelInfo {
        output_modalities: vec!["text".into()],
        pricing: [
            ("prompt".into(), prompt.into()),
            ("completion".into(), completion.into()),
        ]
        .into(),
        ..ModelInfo::default()
    };
    assert_eq!(priced("0", "0").plain_price(), "Free");
    // $3 / $15 per million tokens: 0.45¢ + 0.75¢.
    assert_eq!(
        priced("0.000003", "0.000015").plain_price(),
        "About 1¢ a message"
    );
    assert_eq!(
        priced("0.000002", "0.000004").plain_price(),
        "About 0.5¢ a message"
    );
    assert_eq!(
        priced("0.0000001", "0.0000001").plain_price(),
        "Less than 0.1¢ a message"
    );
    assert_eq!(
        priced("0.00002", "0.00004").plain_price(),
        "About 5¢ a message"
    );
    assert_eq!(
        priced("0.001", "0.001").plain_price(),
        "About $2.00 a message"
    );
    assert_eq!(ModelInfo::default().plain_price(), "Price not listed");
}

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
        ..ModelPreferences::default()
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

#[test]
fn masked_key_preview_and_agent_settings_compose_instructions() {
    assert_eq!(ModelSecret(String::new()).masked_preview(), "");
    assert_eq!(ModelSecret("abc".into()).masked_preview(), "••••••••");
    assert_eq!(
        ModelSecret("sk-or-v1-9988776655441234".into()).masked_preview(),
        "sk-or-••••1234"
    );

    let mut conn = ModelConnection::new("openrouter", ProviderKind::OpenRouter);
    assert_eq!(conn.active_key_display(), None);
    conn.has_credential = true;
    conn.key_preview = "sk-or-••••1234".into();
    assert_eq!(conn.active_key_display().as_deref(), Some("sk-or-••••1234"));

    let settings = AgentSettings {
        always_context: "I am a retired teacher who loves gardening.".into(),
        response_style: ResponseStyle::StepByStep,
        memory_enabled: true,
        tools_enabled: true,
        web_search_enabled: true,
        confirm_actions: true,
    };
    let composed = settings.compose_system_instruction("Base instruction.");
    assert!(composed.contains("Base instruction."));
    assert!(composed.contains("numbered steps"));
    assert!(composed.contains("retired teacher who loves gardening"));
    assert!(composed.contains("Safety guardrail"));
}

#[test]
fn setup_stage_and_curated_recommendations_progress_deterministically() {
    let mut state = ModelsState::default();
    assert_eq!(state.setup_stage(), SetupStage::Loading);

    let router = ModelConnection::new("router", ProviderKind::OpenRouter);
    state.apply(&ModelEvent::Loaded {
        connections: vec![router],
        preferences: ModelPreferences::default(),
    });
    assert_eq!(state.setup_stage(), SetupStage::NeedsConnection);

    // Once the user saves an OpenRouter key, SetupStage advances to NeedsModelSelection.
    state.connections[0].has_credential = true;
    state.connections[0].key_preview = "sk-or-••••9876".into();
    assert_eq!(
        state.setup_stage(),
        SetupStage::NeedsModelSelection {
            connected_service: "OpenRouter".into(),
            key_preview: Some("sk-or-••••9876".into()),
        }
    );

    // Load a catalog with Free, Everyday, and Reasoning models.
    let free = ModelInfo {
        id: "openrouter/free".into(),
        name: "OpenRouter Free Starter".into(),
        author: "OpenRouter".into(),
        context_length: Some(32_000),
        output_modalities: vec!["text".into()],
        pricing: [
            ("prompt".into(), "0".into()),
            ("completion".into(), "0".into()),
        ]
        .into(),
        ..ModelInfo::default()
    };
    let everyday = ModelInfo {
        id: "anthropic/claude-3.5-sonnet".into(),
        name: "Claude 3.5 Sonnet".into(),
        author: "Anthropic".into(),
        context_length: Some(200_000),
        output_modalities: vec!["text".into()],
        supported_parameters: vec!["tools".into()],
        pricing: [
            ("prompt".into(), "0.000003".into()),
            ("completion".into(), "0.000015".into()),
        ]
        .into(),
        ..ModelInfo::default()
    };
    let thinker = ModelInfo {
        id: "deepseek/deepseek-r1".into(),
        name: "DeepSeek R1".into(),
        author: "DeepSeek".into(),
        context_length: Some(128_000),
        output_modalities: vec!["text".into()],
        supported_parameters: vec!["tools".into(), "reasoning".into()],
        pricing: [
            ("prompt".into(), "0.000001".into()),
            ("completion".into(), "0.000002".into()),
        ]
        .into(),
        ..ModelInfo::default()
    };
    state.apply(&ModelEvent::CatalogLoaded {
        connection_id: "router".into(),
        snapshot: CatalogSnapshot {
            models: vec![free, everyday.clone(), thinker],
            fetched_at: 100,
        },
        cached: false,
    });

    let cards = state.recommended_model_cards();
    assert_eq!(cards.len(), 3);
    assert_eq!(cards[0].kind, RecommendationKind::FreeStarter);
    assert_eq!(cards[0].key.model_id, "openrouter/free");
    assert_eq!(cards[1].kind, RecommendationKind::EverydaySmart);
    assert_eq!(cards[1].key.model_id, "anthropic/claude-3.5-sonnet");
    assert_eq!(cards[2].kind, RecommendationKind::DeepReasoning);
    assert_eq!(cards[2].key.model_id, "deepseek/deepseek-r1");

    // Selecting a model transitions SetupStage to Ready.
    let active_key = everyday.key("router");
    state.preferences.active = Some(active_key.clone());
    assert_eq!(
        state.setup_stage(),
        SetupStage::Ready { active: active_key }
    );

    // If the key is removed while a model is selected, SetupStage becomes NeedsModelKey.
    state.connections[0].has_credential = false;
    assert_eq!(
        state.setup_stage(),
        SetupStage::NeedsModelKey {
            model_id: "anthropic/claude-3.5-sonnet".into(),
            connection_id: "router".into(),
            connection_name: "OpenRouter".into(),
        }
    );
}

#[test]
fn interface_scale_steps_through_fixed_zoom_levels_and_clamps_at_the_ends() {
    let default = InterfaceScale::default();
    assert_eq!(default.percent(), 100);
    assert_eq!(default.larger().percent(), 110);
    assert_eq!(default.smaller().percent(), 90);
    let mut largest = default;
    let mut smallest = default;
    for _ in 0..20 {
        largest = largest.larger();
        smallest = smallest.smaller();
    }
    assert_eq!(largest, InterfaceScale::LARGEST);
    assert_eq!(smallest, InterfaceScale::SMALLEST);
    assert!((InterfaceScale::from_percent(150).factor() - 1.5).abs() < f32::EPSILON);
}

#[test]
fn interface_scale_snaps_unknown_saved_values_to_the_nearest_level() {
    assert_eq!(InterfaceScale::from_percent(0), InterfaceScale::SMALLEST);
    assert_eq!(InterfaceScale::from_percent(1000), InterfaceScale::LARGEST);
    assert_eq!(InterfaceScale::from_percent(104).percent(), 100);
    assert_eq!(InterfaceScale::from_percent(106).percent(), 110);
    let saved: InterfaceScale = serde_json::from_str("120").expect("number");
    assert_eq!(saved.percent(), 125);
    assert_eq!(serde_json::to_string(&saved).expect("serialize"), "125");
    let older: ModelPreferences = serde_json::from_str("{\"favorites\":[],\"active\":null}")
        .expect("preferences saved before interface scale existed");
    assert_eq!(older.interface_scale, InterfaceScale::default());
}

#[test]
fn models_are_grouped_by_what_they_make_and_the_library_can_show_only_chat() {
    let model = |id: &str, outputs: &[&str]| ModelInfo {
        id: id.into(),
        name: id.into(),
        output_modalities: outputs.iter().map(|&o| o.to_owned()).collect(),
        ..ModelInfo::default()
    };
    let models = vec![
        model("chat", &["text"]),
        model("chat-and-pictures", &["image", "text"]),
        model("unknown", &[]),
        model("pictures", &["image"]),
        model("video", &["video"]),
        model("voice", &["speech"]),
        model("listen", &["transcription"]),
        model("search", &["embeddings"]),
        model("rerank", &["rerank"]),
    ];
    let kinds: Vec<_> = models.iter().map(ModelInfo::kind).collect();
    assert_eq!(
        kinds,
        [
            ModelKind::Chat,
            ModelKind::Chat,
            ModelKind::Chat,
            ModelKind::Image,
            ModelKind::Video,
            ModelKind::Speech,
            ModelKind::Transcription,
            ModelKind::Embeddings,
            ModelKind::Other,
        ]
    );
    for kind in ModelKind::ALL {
        assert_eq!(ModelKind::from_value(kind.value()), Some(kind));
    }

    let mut state = ModelsState::default();
    state.apply(&ModelEvent::Loaded {
        connections: vec![ModelConnection::new("router", ProviderKind::OpenRouter)],
        preferences: ModelPreferences::default(),
    });
    state.apply(&ModelEvent::CatalogLoaded {
        connection_id: "router".into(),
        snapshot: CatalogSnapshot {
            models,
            fetched_at: 1,
        },
        cached: false,
    });
    let chat = state.filtered(&ModelFilter {
        kind: Some(ModelKind::Chat),
        ..ModelFilter::default()
    });
    assert_eq!(chat.len(), 3);
    assert_eq!(state.filtered(&ModelFilter::default()).len(), 9);
}

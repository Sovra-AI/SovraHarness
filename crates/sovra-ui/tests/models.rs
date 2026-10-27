//! Model-library interactions exercise native controls, real layout and runtime messages.

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, Entity, ScrollDelta, TestAppContext, WindowHandle, point, px, size,
};
use sovra_core::models::{
    CatalogSnapshot, ModelCommand, ModelConnection, ModelEvent, ModelInfo, ModelPreferences,
    ProviderKind,
};
use sovra_core::{Command, Event};
use sovra_runtime::Client;
use sovra_ui::Shell;

fn open_shell(
    cx: &mut TestAppContext,
    client: Client,
    width: f32,
    height: f32,
) -> (WindowHandle<Root>, Entity<Shell>) {
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| Shell::new(window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    (window, shell.expect("shell was created"))
}

fn model(id: &str, name: &str, output: &str, tools: bool) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        name: name.into(),
        author: "Example".into(),
        description: "A model supplied by the provider's catalog.".into(),
        context_length: Some(128_000),
        input_modalities: vec!["text".into()],
        output_modalities: vec![output.into()],
        supported_parameters: if tools {
            vec!["tools".into()]
        } else {
            Vec::new()
        },
        details_loaded: true,
        ..ModelInfo::default()
    }
}

fn fixture_events(models: Vec<ModelInfo>) -> [ModelEvent; 2] {
    [
        ModelEvent::Loaded {
            connections: vec![ModelConnection::new("router", ProviderKind::OpenRouter)],
            preferences: ModelPreferences::default(),
        },
        ModelEvent::CatalogLoaded {
            connection_id: "router".into(),
            snapshot: CatalogSnapshot {
                models,
                fetched_at: 1,
            },
            cached: false,
        },
    ]
}

#[gpui_kit::test]
fn search_filters_favorites_and_non_chat_details(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    // Exercise the wide layout with filters and the catalog visible together.
    let (window, shell) = open_shell(cx, client, 1600., 820.);
    let models = vec![
        model("example/alpha", "Alpha", "text", true),
        model("example/beta", "Beta", "text", false),
        model("example/video", "Video", "video", false),
        model("example/embed", "Embeddings", "embeddings", false),
    ];
    let alpha = models[0].key("router");
    for event in fixture_events(models) {
        events
            .send_blocking(Event::Models(event))
            .expect("test operation succeeds");
    }
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        window.click("models-browse-all", cx);
        assert!(window.find("sovra-models-view").visible());
        // Only models that can chat are listed until the user asks for other types.
        assert_eq!(shell.read(cx).models().read(cx).visible_models().len(), 2);
        assert!(window.find(("catalog-row", 0usize)).visible());
        shell.read(cx).models().update(cx, |view, cx| {
            view.show_kind(None, window, cx);
        });
        assert_eq!(shell.read(cx).models().read(cx).visible_models().len(), 4);
        shell.read(cx).models().update(cx, |view, cx| {
            view.show_kind(Some(sovra_core::models::ModelKind::Chat), window, cx);
        });
        window.click("models-search", cx);
        window.input("BETA", cx);
    })
    .expect("test operation succeeds");
    assert!(matches!(
        commands.try_recv().expect("test operation succeeds"),
        Command::Models(ModelCommand::Load)
    ));
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).models().read(cx).visible_models()[0].model_id,
            "example/beta"
        );
        assert_eq!(shell.read(cx).models().read(cx).visible_models().len(), 1);
        window.click("models-toggle-filters", cx);
        window.click("models-clear-filters", cx);
        window.click("models-filter-tools", cx);
        assert_eq!(
            shell.read(cx).models().read(cx).visible_models(),
            std::slice::from_ref(&alpha)
        );
        window.click(("model-favorite", 0usize), cx);
        let command = commands.try_recv();
        assert!(
            matches!(&command, Ok(Command::Models(ModelCommand::SetFavorite { key, favorite: true })) if key == &alpha),
            "{command:?}; favorite={:?}; row={:?}; tools={:?}; price={:?}",
            window.find(("model-favorite", 0usize)),
            window.find(("catalog-row", 0usize)),
            window.find("models-filter-tools"),
            window.find("Price"),
        );
    })
    .expect("test operation succeeds");
    events
        .send_blocking(Event::Models(ModelEvent::PreferencesChanged(
            ModelPreferences {
                favorites: [alpha.clone()].into(),
                active: Some(alpha.clone()),
                ..ModelPreferences::default()
            },
        )))
        .expect("test operation succeeds");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("models-active-banner").visible());
        assert!(window.find("models-active-inspect").visible());
        window.click("models-clear-filters", cx);
        window.click(("model-scope", 4usize), cx);
        assert_eq!(shell.read(cx).models().read(cx).visible_models(), &[alpha]);
        window.click(("model-scope", 1usize), cx);
        shell.read(cx).models().update(cx, |view, cx| {
            view.set_search("video", window, cx);
            view.show_kind(Some(sovra_core::models::ModelKind::Video), window, cx);
        });
        window.render_frame(cx);
        window.click(("model-row", 0usize), cx);
        assert!(window.find("model-details").visible());
        let action = window.find("model-activate");
        assert_eq!(action.label(), Some("Can't be used in chat"));
        assert_eq!(action.disabled(), Some(true));
    })
    .expect("test operation succeeds");
}

#[gpui_kit::test]
fn setup_starts_with_one_connection_action_and_no_catalog_controls(cx: &mut TestAppContext) {
    let (client, _commands, events) = Client::loopback();
    let (window, _) = open_shell(cx, client, 1280., 820.);
    for event in fixture_events(Vec::new()) {
        events
            .send_blocking(Event::Models(event))
            .expect("event delivered");
    }
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        assert!(window.find("models-setup-home").visible());
        assert!(window.find("models-home-connect").visible());
        assert!(window.try_find("models-search").is_none());
        assert!(window.try_find("models-home-chat").is_none());
        window.click("models-home-connect", cx);
        assert!(window.find("settings-key-form").visible());
        assert!(window.find("connection-key").visible());
    })
    .expect("window exists");
}

#[gpui_kit::test]
fn choosing_a_model_waits_for_confirmation_then_goes_straight_to_chat(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    let (window, shell) = open_shell(cx, client, 1280., 820.);
    let mut free = model("example/free", "Free conversation", "text", true);
    free.pricing = [
        ("prompt".into(), "0".into()),
        ("completion".into(), "0".into()),
    ]
    .into();
    let key = free.key("router");
    let mut connection = ModelConnection::new("router", ProviderKind::OpenRouter);
    connection.has_credential = true;
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![connection],
            preferences: ModelPreferences::default(),
        }))
        .expect("connection delivered");
    events
        .send_blocking(Event::Models(ModelEvent::CatalogLoaded {
            connection_id: "router".into(),
            snapshot: CatalogSnapshot {
                models: vec![free],
                fetched_at: 1,
            },
            cached: false,
        }))
        .expect("catalog delivered");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        let choose = ("models-home-rec-btn", 0usize);
        assert_eq!(window.find(choose).label(), Some("Use this model"));
        window.click(choose, cx);
        assert_eq!(window.find(choose).disabled(), Some(true));
        assert!(window.try_find("models-home-chat").is_none());
    })
    .expect("window exists");
    let mut sent = Vec::new();
    while let Ok(command) = commands.try_recv() {
        sent.push(command);
    }
    assert!(sent.iter().any(|command| matches!(command, Command::Models(ModelCommand::Activate { key: selected }) if selected == &key)));
    events
        .send_blocking(Event::Models(ModelEvent::PreferencesChanged(
            ModelPreferences {
                active: Some(key),
                ..ModelPreferences::default()
            },
        )))
        .expect("activation confirmed");
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).workspace().active_tab,
            sovra_core::NavTab::Models,
            "nothing moves until the runtime confirms the choice"
        );
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).workspace().active_tab,
            sovra_core::NavTab::Chat
        );
    });
}

fn assert_setup_fits_minimum_window(cx: &mut TestAppContext, mode: ThemeMode) {
    let (client, _commands, events) = Client::loopback();
    let (window, _) = open_shell(cx, client, 640., 420.);
    cx.update(|cx| Theme::change(mode, None, cx));
    for event in fixture_events(Vec::new()) {
        events
            .send_blocking(Event::Models(event))
            .expect("setup loaded");
    }
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        let home = window.find("models-setup-home").bounds();
        let connect = window.find("models-home-connect");
        assert!(
            connect.visible(),
            "the first setup action must be visible without scrolling: {connect:?}"
        );
        assert!(
            connect.bounds().bottom() <= home.bottom(),
            "primary action is clipped: {connect:?}; home={home:?}"
        );
        assert!(connect.bounds().right() <= home.right());
        assert!(window.find("models-browse-all").visible());
    })
    .expect("window exists");
}

#[gpui_kit::test]
fn setup_first_action_fits_minimum_dark_window(cx: &mut TestAppContext) {
    assert_setup_fits_minimum_window(cx, ThemeMode::Dark);
}

#[gpui_kit::test]
fn selected_custom_connection_setup_asks_for_its_own_key(cx: &mut TestAppContext) {
    let (client, _commands, events) = Client::loopback();
    let (window, shell) = open_shell(cx, client, 1280., 820.);
    let mut connection = ModelConnection::new("my-service", ProviderKind::OpenAiCompatible);
    connection.name = "My service".into();
    connection.base_url = "https://models.example.test/v1".into();
    connection.authentication = sovra_core::models::Authentication::Bearer;
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![connection],
            preferences: ModelPreferences {
                active: Some(sovra_core::models::ModelKey {
                    connection_id: "my-service".into(),
                    model_id: "chat".into(),
                }),
                ..ModelPreferences::default()
            },
        }))
        .expect("saved setup delivered");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        assert!(window.find("models-home-add-key").visible());
        assert!(window.try_find("models-home-connect").is_none());
        window.click("models-home-add-key", cx);
    })
    .expect("window exists");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("connection-url").value(),
            Some("https://models.example.test/v1")
        );
        assert!(
            window.try_find("settings-key-form").is_none(),
            "must not open OpenRouter key setup"
        );
        shell.update(cx, |shell, cx| {
            shell.handle_llm_setup_action("selected-key", window, cx);
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("connection-url").value(),
            Some("https://models.example.test/v1")
        );
    })
    .expect("window exists");
}

#[gpui_kit::test]
fn setup_first_action_fits_minimum_light_window(cx: &mut TestAppContext) {
    assert_setup_fits_minimum_window(cx, ThemeMode::Light);
}

#[gpui_kit::test]
fn next_setup_actions_fit_minimum_window_in_both_themes(cx: &mut TestAppContext) {
    let (client, _commands, events) = Client::loopback();
    let (window, _) = open_shell(cx, client, 640., 420.);
    let mut free = model("openrouter/free", "Free conversation", "text", false);
    free.pricing = [
        ("prompt".into(), "0".into()),
        ("completion".into(), "0".into()),
    ]
    .into();
    let key = free.key("router");
    for theme in [ThemeMode::Dark, ThemeMode::Light] {
        cx.update(|cx| Theme::change(theme, None, cx));
        for (active, action) in [
            (
                None,
                gpui_kit::ElementId::from(("models-home-rec-btn", 0usize)),
            ),
            (
                Some(key.clone()),
                gpui_kit::ElementId::from("models-home-chat"),
            ),
        ] {
            let mut connection = ModelConnection::new("router", ProviderKind::OpenRouter);
            connection.has_credential = true;
            events
                .send_blocking(Event::Models(ModelEvent::Loaded {
                    connections: vec![connection],
                    preferences: ModelPreferences {
                        active,
                        ..ModelPreferences::default()
                    },
                }))
                .expect("saved setup delivered");
            events
                .send_blocking(Event::Models(ModelEvent::CatalogLoaded {
                    connection_id: "router".into(),
                    snapshot: CatalogSnapshot {
                        models: vec![free.clone()],
                        fetched_at: 1,
                    },
                    cached: false,
                }))
                .expect("catalog delivered");
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.click("nav-models", cx);
                let home = window.find("models-setup-home").bounds();
                let button = window.find(action.clone());
                assert!(button.visible(), "next action must be visible: {button:?}");
                assert!(
                    button.bounds().bottom() <= home.bottom(),
                    "next action is clipped: {button:?}; home={home:?}"
                );
                assert!(button.bounds().right() <= home.right());
            })
            .expect("window exists");
        }
    }
}

fn assert_large_catalog_layout(cx: &mut TestAppContext, width: f32, height: f32, mode: ThemeMode) {
    let (client, _commands, events) = Client::loopback();
    let (window, shell) = open_shell(cx, client, width, height);
    cx.update(|cx| Theme::change(mode, None, cx));
    let models = (0..1000)
        .map(|i| {
            model(
                &format!("example/model-{i:04}"),
                &format!("Model {i:04} with a detailed provider name"),
                "text",
                true,
            )
        })
        .collect();
    for event in fixture_events(models) {
        events
            .send_blocking(Event::Models(event))
            .expect("test operation succeeds");
    }
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        window.click("models-browse-all", cx);
        assert_eq!(
            shell.read(cx).models().read(cx).visible_models().len(),
            1000
        );
        let page = window.find("sovra-models-view").bounds();
        assert!(page.right() <= px(width) && page.bottom() <= px(height));
        assert!(window.find("models-search").visible());
        let rows: Vec<_> = window
            .elements()
            .into_iter()
            .filter(|element| {
                element
                    .label()
                    .is_some_and(|label| label.starts_with("View details for example/"))
            })
            .collect();
        assert!(
            !rows.is_empty(),
            "the catalog needs visible, usable rows at minimum size"
        );
        assert!(rows.len() < 20, "large catalogs must stay virtualized");
        assert!(rows.iter().any(gpui_kit::test::ElementSnapshot::visible));
        for row in rows {
            assert!(row.bounds().left() >= page.left());
            assert!(row.bounds().right() <= page.right());
        }
    })
    .expect("test operation succeeds");
}

#[gpui_kit::test]
fn large_catalog_dark_desktop(cx: &mut TestAppContext) {
    assert_large_catalog_layout(cx, 1280., 820., ThemeMode::Dark);
}

#[gpui_kit::test]
fn large_catalog_light_desktop(cx: &mut TestAppContext) {
    assert_large_catalog_layout(cx, 1280., 820., ThemeMode::Light);
}

#[gpui_kit::test]
fn large_catalog_dark_minimum(cx: &mut TestAppContext) {
    assert_large_catalog_layout(cx, 640., 420., ThemeMode::Dark);
}

#[gpui_kit::test]
fn large_catalog_light_minimum(cx: &mut TestAppContext) {
    assert_large_catalog_layout(cx, 640., 420., ThemeMode::Light);
}

#[gpui_kit::test]
fn minimum_window_filters_keep_apply_action_visible(cx: &mut TestAppContext) {
    let (client, _commands, events) = Client::loopback();
    let (window, _) = open_shell(cx, client, 640., 420.);
    for event in fixture_events(vec![model("a/text", "Text", "text", true)]) {
        events
            .send_blocking(Event::Models(event))
            .expect("fixture delivered");
    }
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        window.click("models-browse-all", cx);
        window.click("models-toggle-filters", cx);
        assert!(window.find("models-apply-filters").visible());
        assert!(window.find("models-apply-filters").bounds().bottom() <= px(420.));
        window.scroll(
            "models-filter-panel",
            ScrollDelta::Pixels(point(px(0.), px(-1000.))),
            cx,
        );
        assert!(window.find("models-filter-tools").visible());
        window.click("models-filter-tools", cx);
        window.click("models-apply-filters", cx);
        assert!(window.find(("model-row", 0usize)).visible());
    })
    .expect("window exists");
}

#[gpui_kit::test]
fn ollama_installed_models_and_download_panel_fit_minimum_window(cx: &mut TestAppContext) {
    let (client, _commands, events) = Client::loopback();
    let (window, _) = open_shell(cx, client, 640., 420.);
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![ModelConnection::new("ollama", ProviderKind::Ollama)],
            preferences: ModelPreferences::default(),
        }))
        .expect("connections delivered");
    events
        .send_blocking(Event::Models(ModelEvent::CatalogLoaded {
            connection_id: "ollama".into(),
            snapshot: CatalogSnapshot {
                models: vec![model("local:8b", "Local model", "text", true)],
                fetched_at: 1,
            },
            cached: false,
        }))
        .expect("catalog delivered");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        window.click("models-browse-all", cx);
        window.click(("model-scope", 2usize), cx);
        assert!(window.find(("model-row", 0usize)).visible());
        window.click("ollama-toggle-download", cx);
        assert!(window.find("ollama-download-back").visible());
        assert!(window.find("ollama-pull").visible());
        window.click("ollama-download-back", cx);
        assert!(window.find(("model-row", 0usize)).visible());
    })
    .expect("window exists");
}

#[gpui_kit::test]
fn connection_preset_and_save_acknowledgement_preserve_draft(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    let (window, _) = open_shell(cx, client, 1280., 1000.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        window.click("models-browse-all", cx);
        window.click("models-connections", cx);
        window.click("connection-add", cx);
        window.click(("connection-preset", 3usize), cx);
    })
    .expect("window exists");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("connection-url").value(),
            Some("http://localhost:8000/v1")
        );
        window.scroll(
            "model-connection-form",
            ScrollDelta::Pixels(point(px(0.), px(-2000.))),
            cx,
        );
        window.click("connection-save", cx);
        assert_eq!(window.find("connection-save").disabled(), Some(true));
    })
    .expect("window exists");
    let saved = loop {
        if let Command::Models(ModelCommand::SaveConnection {
            connection,
            credential,
        }) = commands.try_recv().expect("save dispatched")
        {
            break (connection, credential);
        }
    };
    assert_eq!(saved.0.base_url, "http://localhost:8000/v1");
    assert_eq!(
        saved.0.authentication,
        sovra_core::models::Authentication::None
    );
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![],
            preferences: ModelPreferences::default(),
        }))
        .expect("replay delivered");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("model-connection-form").visible(),
            "an unrelated Load is not save confirmation"
        );
    })
    .expect("window exists");
    events
        .send_blocking(Event::Models(ModelEvent::Error(
            "Credential store unavailable".into(),
        )))
        .expect("error delivered");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("model-connection-form").visible());
        assert_ne!(window.find("connection-save").disabled(), Some(true));
        window.click("connection-save", cx);
    })
    .expect("window exists");
    events
        .send_blocking(Event::Models(ModelEvent::ConnectionSaved {
            id: saved.0.id,
        }))
        .expect("ack delivered");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("model-connection-form").is_none());
    })
    .expect("window exists");
}

#[gpui_kit::test]
fn settings_saves_openrouter_key_through_secure_connection_protocol(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    let (window, _) = open_shell(cx, client, 1000., 760.);
    events
        .send_blocking(Event::Models(fixture_events(vec![])[0].clone()))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-settings", cx);
        window.render_frame(cx);
        assert!(window.find("sovra-settings-view").visible());
        window.click("settings-openrouter", cx);
        window.render_frame(cx);
        window.click("connection-key", cx);
        window.input("test-openrouter-key", cx);
        window.click("connection-save", cx);
    })
    .unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::Models(ModelCommand::Load)
    ));
    let Command::Models(ModelCommand::SaveConnection {
        connection,
        credential,
    }) = commands.try_recv().unwrap()
    else {
        panic!("Settings must dispatch a connection save");
    };
    assert_eq!(connection.id, "router");
    assert_eq!(connection.kind, ProviderKind::OpenRouter);
    assert!(
        matches!(credential, sovra_core::models::CredentialChange::Replace(secret) if secret.0 == "test-openrouter-key")
    );
}

#[gpui_kit::test]
fn saved_openrouter_key_stays_visible_and_can_be_tested_replaced_and_removed(
    cx: &mut TestAppContext,
) {
    let (client, commands, events) = Client::loopback();
    let (window, _) = open_shell(cx, client, 1000., 760.);
    let mut connection = ModelConnection::new("router", ProviderKind::OpenRouter);
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![connection.clone()],
            preferences: ModelPreferences::default(),
        }))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-settings", cx);
        window.click("settings-openrouter", cx);
        window.click("connection-key", cx);
        window.input("test-key", cx);
        window.click("connection-save", cx);
    })
    .unwrap();
    commands.try_recv().unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::Models(ModelCommand::SaveConnection { .. })
    ));
    connection.has_credential = true;
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![connection.clone()],
            preferences: ModelPreferences::default(),
        }))
        .unwrap();
    events
        .send_blocking(Event::Models(ModelEvent::ConnectionSaved {
            id: connection.id.clone(),
        }))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("settings-key-status")
                .label()
                .unwrap()
                .contains("saved in this computer's keychain")
        );
        assert!(window.find("connection-key").visible());
        assert_eq!(
            window.find("connection-save").disabled(),
            Some(true),
            "saved secret must be cleared from draft"
        );
        window.click("connection-cancel", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("settings-openrouter-status")
                .label()
                .unwrap()
                .contains("saved in this computer's keychain")
        );
        window.click("settings-openrouter", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("settings-key-status")
                .label()
                .unwrap()
                .contains("saved in this computer's keychain")
        );
        window.click("connection-test", cx);
    })
    .unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::Models(ModelCommand::TestConnection {
            credential: sovra_core::models::CredentialChange::Keep,
            ..
        })
    ));
    cx.update_window(window.into(), |_, window, cx| {
        window.click("connection-key", cx);
        window.input("replacement-key", cx);
        window.click("connection-save", cx);
    })
    .unwrap();
    assert!(
        matches!(commands.try_recv().unwrap(), Command::Models(ModelCommand::SaveConnection { credential: sovra_core::models::CredentialChange::Replace(secret), .. }) if secret.0 == "replacement-key")
    );
    events
        .send_blocking(Event::Models(ModelEvent::ConnectionSaved {
            id: connection.id.clone(),
        }))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-remove-key", cx);
    })
    .unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::Models(ModelCommand::SaveConnection {
            credential: sovra_core::models::CredentialChange::Remove,
            ..
        })
    ));
    connection.has_credential = false;
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![connection.clone()],
            preferences: ModelPreferences::default(),
        }))
        .unwrap();
    events
        .send_blocking(Event::Models(ModelEvent::ConnectionSaved {
            id: connection.id,
        }))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("settings-key-status").label(),
            Some("Not connected yet")
        );
        assert!(window.find("connection-key").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_keeps_advanced_and_technical_options_out_of_the_way(cx: &mut TestAppContext) {
    fn reveal(window: &mut gpui_kit::Window, id: &'static str, cx: &mut gpui_kit::App) {
        for _ in 0..100 {
            window.render_frame(cx);
            let item = window.find(id);
            let bounds = item.bounds();
            let viewport = window.find("settings-sections").bounds();
            if item.visible()
                && bounds.top() >= viewport.top()
                && bounds.bottom() <= viewport.bottom()
            {
                return;
            }
            let delta = if bounds.top() < viewport.top() {
                60.
            } else {
                -60.
            };
            window.scroll(
                "settings-sections",
                ScrollDelta::Pixels(point(px(0.), px(delta))),
                cx,
            );
        }
        panic!("{id} must be reachable in Settings");
    }
    let (client, _commands, events) = Client::loopback();
    let (window, _) = open_shell(cx, client, 1000., 760.);
    events
        .send_blocking(Event::Models(fixture_events(vec![])[0].clone()))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-settings", cx);
        window.render_frame(cx);
        assert!(window.find("settings-appearance").visible());
        assert!(window.find("settings-ai").visible());
        assert_eq!(
            window.find("settings-openrouter-status").label(),
            Some("Not connected yet")
        );
        assert!(
            window.try_find("connection-add").is_none(),
            "advanced connections stay folded away until asked for"
        );
        reveal(window, "settings-advanced-toggle", cx);
        window.click("settings-advanced-toggle", cx);
        window.render_frame(cx);
        reveal(window, "connection-add", cx);
        assert!(window.find("connection-add").visible());
        window.click("connection-add", cx);
        window.render_frame(cx);
        assert!(window.find("connection-url").visible());
        assert!(
            window.try_find("connection-timeout").is_none(),
            "technical fields are hidden by default"
        );
        assert!(window.try_find("connection-clear-key").is_none());
        window.click("connection-more-options", cx);
        window.render_frame(cx);
        window.scroll(
            "model-connection-form",
            ScrollDelta::Pixels(point(px(0.), px(-2000.))),
            cx,
        );
        window.render_frame(cx);
        assert!(window.find("connection-timeout").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn appearance_choice_is_saved_and_restored(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    use sovra_core::models::Appearance;

    let (client, commands, events) = Client::loopback();
    let (window, _) = open_shell(cx, client, 1000., 760.);
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![],
            preferences: ModelPreferences {
                appearance: Appearance::Light,
                ..ModelPreferences::default()
            },
        }))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        assert!(!cx.theme().is_dark(), "saved Light choice applies on load");
        window.render_frame(cx);
        window.click("nav-settings", cx);
        window.render_frame(cx);
        window.click("settings-theme-dark", cx);
        assert!(cx.theme().is_dark());
    })
    .unwrap();
    let saved = loop {
        if let Command::Models(ModelCommand::SetAppearance { appearance }) =
            commands.try_recv().expect("appearance saved")
        {
            break appearance;
        }
    };
    assert_eq!(saved, Appearance::Dark);
    cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
}

#[gpui_kit::test]
fn setup_with_saved_key_only_shows_step_two_and_curated_recommendations(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    let (window, _) = open_shell(cx, client, 1280., 820.);

    let mut free = model("openrouter/free", "Free Starter", "text", false);
    free.pricing = [
        ("prompt".into(), "0".into()),
        ("completion".into(), "0".into()),
    ]
    .into();
    let mut smart = model("anthropic/claude-3.5-sonnet", "Claude Sonnet", "text", true);
    smart.pricing = [
        ("prompt".into(), "0.000003".into()),
        ("completion".into(), "0.000015".into()),
    ]
    .into();
    let mut thinker = model("deepseek/deepseek-r1", "DeepSeek R1", "text", true);
    thinker.supported_parameters.push("reasoning".into());
    thinker.pricing = [
        ("prompt".into(), "0.000002".into()),
        ("completion".into(), "0.000004".into()),
    ]
    .into();

    let mut connection = ModelConnection::new("router", ProviderKind::OpenRouter);
    connection.has_credential = true;
    connection.key_preview = "sk-or-••••1234".into();

    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![connection],
            preferences: ModelPreferences::default(),
        }))
        .unwrap();
    events
        .send_blocking(Event::Models(ModelEvent::CatalogLoaded {
            connection_id: "router".into(),
            snapshot: CatalogSnapshot {
                models: vec![free, smart, thinker],
                fetched_at: 10,
            },
            cached: false,
        }))
        .unwrap();
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        window.render_frame(cx);

        // Step 1 connection action must be hidden when key is already connected!
        assert!(window.try_find("models-home-connect").is_none());
        assert!(window.try_find("models-home-local").is_none());
        // Step 1 completion badge and Step 2 curated recommendation cards must be visible.
        assert!(window.find("models-home-step1-done").visible());
        assert!(window.find(("models-home-rec", 0usize)).visible());
        assert!(window.find(("models-home-rec", 1usize)).visible());
        assert!(window.find(("models-home-rec", 2usize)).visible());
        // Prices read as cost per message, never per-million-token rates.
        let prices: Vec<_> = (0..3usize)
            .map(|index| {
                window
                    .find(("models-home-rec-price", index))
                    .label()
                    .map(str::to_owned)
            })
            .collect();
        assert_eq!(
            prices.iter().map(Option::as_deref).collect::<Vec<_>>(),
            [
                Some("Free"),
                Some("About 1¢ a message"),
                Some("About 0.5¢ a message")
            ]
        );

        // Also verify direct 1-click model activation in the full catalog row.
        window.click("models-browse-all", cx);
        window.render_frame(cx);
        assert!(window.find(("model-use-row", 0usize)).visible());
        window.click(("model-use-row", 0usize), cx);
    })
    .unwrap();

    let mut activated = false;
    while let Ok(cmd) = commands.try_recv() {
        if matches!(cmd, Command::Models(ModelCommand::Activate { .. })) {
            activated = true;
        }
    }
    assert!(
        activated,
        "clicking Use Model on a catalog row must dispatch Activate"
    );
}

/// Saving a key during setup leads straight on: to the chat when a model was already
/// chosen, or to picking one otherwise. Nobody is left on a settings form.
#[gpui_kit::test]
fn saving_a_key_during_setup_continues_to_the_next_step(cx: &mut TestAppContext) {
    for chosen in [true, false] {
        let (client, commands, events) = Client::loopback();
        let (window, shell) = open_shell(cx, client, 1280., 1000.);
        let chat = model("example/chat", "Chat", "text", false);
        let key = chat.key("openrouter");
        let connection = ModelConnection::new("openrouter", ProviderKind::OpenRouter);
        let preferences = ModelPreferences {
            active: chosen.then(|| key.clone()),
            ..ModelPreferences::default()
        };
        for event in [
            ModelEvent::Loaded {
                connections: vec![connection.clone()],
                preferences: preferences.clone(),
            },
            ModelEvent::CatalogLoaded {
                connection_id: "openrouter".into(),
                snapshot: CatalogSnapshot {
                    models: vec![chat],
                    fetched_at: 1,
                },
                cached: false,
            },
        ] {
            events.send_blocking(Event::Models(event)).unwrap();
        }
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("nav-models", cx);
            window.click(
                if chosen {
                    "models-home-add-key"
                } else {
                    "models-home-connect"
                },
                cx,
            );
            window.render_frame(cx);
            window.click("connection-key", cx);
            window.input("sk-or-test-key", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("connection-save", cx);
        })
        .unwrap();
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok())
                .any(|c| matches!(c, Command::Models(ModelCommand::SaveConnection { .. }))),
            "the key is saved"
        );
        let mut saved = connection.clone();
        saved.has_credential = true;
        for event in [
            ModelEvent::Loaded {
                connections: vec![saved],
                preferences,
            },
            ModelEvent::ConnectionSaved {
                id: "openrouter".into(),
            },
        ] {
            events.send_blocking(Event::Models(event)).unwrap();
        }
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let tab = shell.read(cx).workspace().active_tab;
            if chosen {
                assert_eq!(tab, sovra_core::NavTab::Chat);
            } else {
                assert_eq!(tab, sovra_core::NavTab::Models);
                assert!(window.find("models-setup-home").visible());
                assert!(window.find("models-home-step1-done").visible());
            }
        })
        .unwrap();
    }
}

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
    let (window, shell) = open_shell(cx, client, 1280., 820.);
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
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
        assert!(window.find("sovra-models-view").visible());
        assert_eq!(shell.read(cx).models().read(cx).visible_models().len(), 4);
        window.click("models-search", cx);
        window.input("BETA", cx);
    })
    .expect("test operation succeeds");
    assert!(matches!(
        commands.try_recv().expect("test operation succeeds"),
        Command::Models(ModelCommand::Load)
    ));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).models().read(cx).visible_models()[0].model_id,
            "example/beta"
        );
        assert_eq!(shell.read(cx).models().read(cx).visible_models().len(), 1);
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
                active: None,
            },
        )))
        .expect("test operation succeeds");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("models-clear-filters", cx);
        window.click(("model-scope", 4usize), cx);
        assert_eq!(shell.read(cx).models().read(cx).visible_models(), &[alpha]);
        window.click(("model-scope", 1usize), cx);
        shell
            .read(cx)
            .models()
            .update(cx, |view, cx| view.set_search("video", window, cx));
        window.render_frame(cx);
        window.click(("model-row", 0usize), cx);
        assert!(window.find("model-details").visible());
        let action = window.find("model-activate");
        assert_eq!(action.label(), Some("Catalog information only"));
        assert_eq!(action.disabled(), Some(true));
    })
    .expect("test operation succeeds");
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
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
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
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
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
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-models", cx);
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
        window.click("models-connections", cx);
        window.click("connection-add", cx);
        window.click(("connection-preset", 3usize), cx);
    })
    .expect("window exists");
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
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("settings-key-status")
                .label()
                .unwrap()
                .contains("Key saved in OS keychain")
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
                .contains("Key saved in OS keychain")
        );
        window.click("settings-openrouter", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("settings-key-status")
                .label()
                .unwrap()
                .contains("Key saved in OS keychain")
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
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("settings-key-status").label(),
            Some("No saved API key")
        );
        assert!(window.find("connection-key").visible());
    })
    .unwrap();
}

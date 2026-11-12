//! The Connections tab: the real shell, the real catalogue, both themes.

use std::collections::BTreeMap;

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::connection_catalogue::{first_screen_asks_for_secret, whatsapp_phase};
use sovra_core::connections::{ConnectionEvent, LinkedDevice, TelegramStatus, WhatsAppStatus};
use sovra_core::google::{GoogleEvent, GoogleState};
use sovra_core::mcp::McpService;
use sovra_core::models::{CatalogState, ModelConnection};
use sovra_core::{CatalogueKind, CatalogueSources, Event, LinkPhase, Timestamp, catalogue_from};

fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

fn fresh_sources<'a>(
    connections: &'a [ModelConnection],
    catalogs: &'a BTreeMap<String, CatalogState>,
    whatsapp: &'a WhatsAppStatus,
    telegram: &'a TelegramStatus,
    google: &'a GoogleState,
    services: &'a [McpService],
    probed: &'a BTreeMap<CatalogueKind, LinkPhase>,
) -> CatalogueSources<'a> {
    CatalogueSources {
        connections,
        catalogs,
        whatsapp,
        telegram,
        google,
        github: &sovra_core::github::GitHubState::NotConnected,
        services,
        probed,
    }
}

#[gpui_kit::test]
fn connections_lists_every_shipped_kind_and_settings_does_not(cx: &mut TestAppContext) {
    let (client, _commands, events) = sovra_runtime::Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let window = cx.open_window(size(px(1100.), px(2400.)), |window, cx| {
        let shell = cx.new(|cx| sovra_ui::Shell::new(0, window, cx));
        Root::new(shell, window, cx)
    });
    events
        .send_blocking(Event::Google(GoogleEvent::Changed(
            GoogleState::NotConnected,
        )))
        .expect("loopback accepts the scripted sign-in state");
    events
        .send_blocking(Event::GitHub(sovra_core::github::GitHubEvent::Changed {
            state: sovra_core::github::GitHubState::NotConnected,
            device_sign_in: true,
        }))
        .expect("loopback accepts the scripted GitHub state");
    settle(cx);

    let connections = Vec::new();
    let catalogs = BTreeMap::new();
    let whatsapp = WhatsAppStatus::Disconnected;
    let telegram = TelegramStatus::Disconnected;
    let google = GoogleState::NotConnected;
    let services = Vec::new();
    let probed = BTreeMap::new();
    let expected = catalogue_from(&fresh_sources(
        &connections,
        &catalogs,
        &whatsapp,
        &telegram,
        &google,
        &services,
        &probed,
    ));
    let secret_words = expected.first_screen_copy();
    assert!(
        !first_screen_asks_for_secret(&secret_words),
        "catalogue copy asks for a secret: {secret_words}"
    );

    let mut seen = Vec::new();
    cx.update_window(window.into(), |_, window, cx| {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            Theme::change(mode, Some(window), cx);
            window.render_frame(cx);
            window.click("nav-connections", cx);
            window.render_frame(cx);
            assert!(window.find("connections-page").visible());
            assert!(window.find("connections-refresh").visible());
            let mut labels = Vec::new();
            for card in expected.cards() {
                let slug = card.kind.slug();
                let title = window.find(format!("connection-title-{slug}"));
                let sentence = window.find(format!("connection-sentence-{slug}"));
                let status = window.find(format!("connection-status-{slug}"));
                let action = window.find(format!("connection-action-{slug}"));
                assert!(title.visible(), "{slug} title");
                assert!(sentence.visible(), "{slug} sentence");
                assert!(status.visible(), "{slug} status");
                assert!(action.visible(), "{slug} action");
                assert_eq!(title.label(), Some(card.title), "{slug}");
                assert_eq!(sentence.label(), Some(card.sentence), "{slug}");
                assert_eq!(status.label(), Some(card.phase.label()), "{slug}");
                assert_eq!(action.label(), Some(card.action), "{slug}");
                labels.push((card.title, card.sentence, card.phase.label(), card.action));
            }
            for id in [
                "connection-key",
                "telegram-token-input",
                "google-client-id",
                "mcp-service-address",
                "connection-url",
                "connection-card-microsoft",
            ] {
                assert!(
                    window.try_find(id).is_none(),
                    "{id} is not on the first screen"
                );
            }
            seen.push(labels);
        }
        assert_eq!(seen[0], seen[1], "both themes show the same catalogue");

        window.click("connection-action-whatsapp", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("whatsapp-connect-btn").label(),
            Some("Connect WhatsApp")
        );
        window.click("connections-back", cx);
        window.render_frame(cx);
        assert!(window.find("connection-card-whatsapp").visible());
    })
    .unwrap();

    let linked = WhatsAppStatus::Connected {
        device: LinkedDevice {
            phone_display: "+1 555-0142".into(),
            device_jid: "15550142:1@s.whatsapp.net".into(),
            companion_name: "Sovra".into(),
            linked_at: Timestamp(10_000),
        },
        online: true,
    };
    events
        .send_blocking(Event::Connections(ConnectionEvent::StatusChanged(
            linked.clone(),
        )))
        .expect("loopback accepts the scripted WhatsApp status");
    settle(cx);
    let after = catalogue_from(&fresh_sources(
        &connections,
        &catalogs,
        &linked,
        &telegram,
        &google,
        &services,
        &probed,
    ));
    assert_eq!(whatsapp_phase(&linked), LinkPhase::Linked);
    assert_eq!(
        after.card(CatalogueKind::Telegram).phase,
        LinkPhase::NotLinked
    );

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("connection-status-whatsapp").label(),
            Some(after.card(CatalogueKind::WhatsApp).phase.label())
        );
        assert_eq!(
            window.find("connection-status-telegram").label(),
            Some(after.card(CatalogueKind::Telegram).phase.label())
        );
        window.click("nav-settings", cx);
        window.render_frame(cx);
        assert!(window.find("sovra-settings-view").visible());
        assert!(window.find("settings-appearance").visible());
        assert!(window.find("settings-open-connections").visible());
        for id in [
            "settings-ai",
            "connection-key",
            "telegram-token-input",
            "google-client-id",
            "google-connect",
            "mcp-service-address",
            "connection-url",
            "whatsapp-connect-btn",
        ] {
            assert!(
                window.try_find(id).is_none(),
                "{id} is not edited from Settings"
            );
        }
    })
    .unwrap();
}

//! The Google card in Settings → Connections, end to end through the shell with a scripted runtime.

use std::collections::BTreeSet;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle, px, size};
use sovra_core::google::{GoogleAccount, GoogleCommand, GoogleEvent, GoogleService, GoogleState};
use sovra_core::models::InterfaceScale;
use sovra_core::{Command, Event, NavTab};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

type Opened = (
    WindowHandle<Root>,
    Entity<Shell>,
    async_channel::Receiver<Command>,
    async_channel::Sender<Event>,
);

fn open(cx: &mut TestAppContext, width: f32, height: f32) -> Opened {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| Shell::new(window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    settle(cx);
    (
        window,
        shell.expect("headless test fixture is available"),
        commands,
        events,
    )
}

fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

fn google(events: &async_channel::Sender<Event>, state: GoogleState, cx: &mut TestAppContext) {
    events
        .send_blocking(Event::Google(GoogleEvent::Changed(state)))
        .expect("headless test fixture is available");
    settle(cx);
}

fn google_commands(commands: &async_channel::Receiver<Command>) -> Vec<GoogleCommand> {
    std::iter::from_fn(|| commands.try_recv().ok())
        .filter_map(|command| match command {
            Command::Google(command) => Some(command),
            _ => None,
        })
        .collect()
}

fn account() -> GoogleAccount {
    GoogleAccount {
        email: "sam.lee@example.com".into(),
        name: "Sam Lee".into(),
        granted: BTreeSet::from([GoogleService::Mail, GoogleService::Calendar]),
        turned_off: BTreeSet::new(),
    }
}

#[gpui_kit::test]
fn one_button_connects_and_every_service_has_a_switch(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1100., 2400.);
    google(&events, GoogleState::NotConnected, cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click("nav-settings", cx);
        window.render_frame(cx);
        assert!(window.find("sovra-settings-view").visible());
        assert_eq!(
            window.find("google-connect").label(),
            Some("Connect Google")
        );
        window.click("google-connect", cx);
    })
    .unwrap();
    assert_eq!(google_commands(&commands), [GoogleCommand::Connect]);

    google(
        &events,
        GoogleState::Connecting {
            url: "https://accounts.google.com/o/oauth2/v2/auth?x".into(),
            adding: None,
        },
        cx,
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("google-open-again").visible());
        assert!(window.find("google-copy-link").visible());
        window.click("google-cancel", cx);
    })
    .unwrap();
    assert_eq!(google_commands(&commands), [GoogleCommand::Cancel]);

    google(&events, GoogleState::Connected(account()), cx);
    shell.read_with(cx, |shell, _| {
        assert!(
            shell
                .workspace()
                .google
                .notice
                .as_deref()
                .is_some_and(|t| t.contains("sam.lee@example.com")),
            "the person is told it worked"
        );
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("google-notice").visible());
        for index in 0..GoogleService::ALL.len() {
            assert!(
                window.find(("google-service", index)).visible(),
                "row {index}"
            );
        }
        // Email off; Files was never allowed, so turning it on asks Google.
        window.click(("google-service-switch", 0usize), cx);
        window.click(("google-service-switch", 2usize), cx);
    })
    .unwrap();
    assert_eq!(
        google_commands(&commands),
        [
            GoogleCommand::SetService {
                service: GoogleService::Mail,
                on: false
            },
            GoogleCommand::SetService {
                service: GoogleService::Files,
                on: true
            },
        ]
    );

    cx.update_window(window.into(), |_, window, cx| {
        window.click(("google-try", 1usize), cx);
        window.render_frame(cx);
    })
    .unwrap();
    shell.read_with(cx, |shell, _| {
        assert_eq!(
            shell.workspace().active_tab,
            NavTab::Chat,
            "examples open Chat"
        );
    });

    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        window.click("google-disconnect", cx);
        window.render_frame(cx);
        assert!(
            window.find("google-disconnect-confirm").visible(),
            "asks first"
        );
    })
    .unwrap();
    assert!(
        google_commands(&commands).is_empty(),
        "nothing until confirmed"
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.click("google-disconnect-yes", cx);
    })
    .unwrap();
    assert_eq!(google_commands(&commands), [GoogleCommand::Disconnect]);
}

#[gpui_kit::test]
fn an_expired_sign_in_offers_one_button_to_fix_it(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1100., 1600.);
    google(&events, GoogleState::NeedsSignIn(account()), cx);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
        window.render_frame(cx);
        assert!(window.find("google-account").visible());
        window.click("google-reconnect", cx);
    })
    .unwrap();
    assert_eq!(google_commands(&commands), [GoogleCommand::Connect]);
}

#[gpui_kit::test]
fn google_card_fits_minimum_wide_and_double_size(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (1280., 820., 200)] {
        let (window, shell, _commands, events) = open(cx, width, height);
        google(&events, GoogleState::Connected(account()), cx);
        cx.update(|cx| design::set_interface_scale(InterfaceScale::from_percent(scale), cx));
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
            window.render_frame(cx);
            let page = window.find("sovra-settings-view").bounds();
            let card = window.find("google-card").bounds();
            let at = format!("{width}x{height}@{scale}");
            assert!(
                card.left() >= page.left() && card.right() <= page.right(),
                "{at}: card"
            );
            for id in ["google-status", "google-account", "google-disconnect"] {
                let element = window.find(id).bounds();
                assert!(element.right() <= card.right(), "{at}: {id} overflows");
            }
            let row = window.find(("google-service-switch", 4usize)).bounds();
            assert!(row.right() <= card.right(), "{at}: switch overflows");
            if width > 2000. {
                let cap = design::tokens(cx)
                    .layout
                    .reading_width
                    .to_pixels(window.rem_size());
                assert!(card.size.width <= cap, "{at}: the card stays readable");
            }
        })
        .unwrap();
        cx.update(|cx| design::set_interface_scale(InterfaceScale::default(), cx));
    }
}

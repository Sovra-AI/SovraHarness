//! Microsoft controls stay reachable and emit only the requested commands.

use super::*;
use sovra_core::microsoft::{
    MicrosoftAccount, MicrosoftClient, MicrosoftCommand, MicrosoftEvent, MicrosoftService,
    MicrosoftSetupKind, MicrosoftState, RETURN_ADDRESS,
};
use sovra_core::{Command, Event, NavTab};

fn reveal(window: &mut gpui_kit::Window, id: &'static str, cx: &mut gpui_kit::App) {
    reveal_in(window, id, None, cx);
}

fn reveal_in(
    window: &mut gpui_kit::Window,
    id: &'static str,
    child: Option<usize>,
    cx: &mut gpui_kit::App,
) {
    for _ in 0..80 {
        window.render_frame(cx);
        let pane = window.find("connections-scroll").bounds();
        let item = if let Some(child) = child {
            window.within(id).find(child).bounds()
        } else {
            window.find(id).bounds()
        };
        assert!(
            item.left() >= pane.left() && item.right() <= pane.right(),
            "{id}: {item:?} outside {pane:?}"
        );
        if item.top() >= pane.top() && item.bottom() <= pane.bottom() {
            return;
        }
        let delta = if item.top() < pane.top() {
            pane.top() - item.top()
        } else {
            pane.bottom() - item.bottom()
        };
        window.scroll(
            "connections-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), delta)),
            cx,
        );
    }
    panic!("{id} must remain reachable");
}

fn state(events: &async_channel::Sender<Event>, state: MicrosoftState, cx: &mut TestAppContext) {
    events
        .send_blocking(Event::Microsoft(MicrosoftEvent::Changed(state)))
        .expect("fixture event accepted");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn microsoft_flow_fits_minimum_wide_and_large_text_in_both_themes(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(
                    sovra_core::models::InterfaceScale::from_percent(scale),
                    cx,
                );
            });
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| Shell::new(0, window, cx));
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Connections, cx));
                Root::new(shell, window, cx)
            });
            state(&events, MicrosoftState::NeedsSetup, cx);
            while commands.try_recv().is_ok() {}
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "connection-action-microsoft", cx);
                window.click("connection-action-microsoft", cx);
            })
            .unwrap();
            assert!(
                commands.try_recv().is_err(),
                "setup cannot start sign-in before IDs are supplied"
            );
            let kind = if mode == ThemeMode::Light {
                MicrosoftSetupKind::Work
            } else {
                MicrosoftSetupKind::Personal
            };
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "microsoft-setup-step", cx);
                assert!(
                    window
                        .find("microsoft-setup-step")
                        .label()
                        .unwrap()
                        .contains("Step 1 of 4")
                );
                if kind == MicrosoftSetupKind::Personal {
                    reveal_in(window, "microsoft-account-kind", Some(1), cx);
                    window.within("microsoft-account-kind").click(1usize, cx);
                }
                window.render_frame(cx);
                assert_eq!(
                    window
                        .within("microsoft-account-kind")
                        .find(0usize)
                        .checked(),
                    Some(kind == MicrosoftSetupKind::Work)
                );
                assert_eq!(
                    window
                        .within("microsoft-account-kind")
                        .find(1usize)
                        .checked(),
                    Some(kind == MicrosoftSetupKind::Personal)
                );
                reveal(window, "microsoft-copy-help", cx);
                window.click("microsoft-copy-help", cx);
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().as_deref(),
                    Some(kind.help_request())
                );
                reveal(window, "microsoft-setup-next", cx);
                window.click("microsoft-setup-next", cx);
                reveal(window, "microsoft-account-choice", cx);
                assert_eq!(
                    window.find("microsoft-account-choice").label().unwrap(),
                    kind.account_choice()
                );
                reveal(window, "microsoft-open-setup", cx);
                reveal(window, "microsoft-setup-next", cx);
                window.click("microsoft-setup-next", cx);
                reveal(window, "microsoft-setup-next", cx);
                assert_eq!(window.find("microsoft-setup-next").disabled(), Some(true));
                reveal(window, "microsoft-copy-return", cx);
                window.click("microsoft-copy-return", cx);
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().as_deref(),
                    Some(RETURN_ADDRESS)
                );
                reveal(window, "microsoft-return-confirmed", cx);
                window.click("microsoft-return-confirmed", cx);
                reveal(window, "microsoft-setup-next", cx);
                window.click("microsoft-setup-next", cx);
                reveal(window, "microsoft-save-connect", cx);
                assert_eq!(window.find("microsoft-save-connect").disabled(), Some(true));
                reveal(window, "microsoft-app-id", cx);
                window.click("microsoft-app-id", cx);
                window.input("10000000-0000-4000-8000-000000000001", cx);
                if kind == MicrosoftSetupKind::Work {
                    reveal(window, "microsoft-save-connect", cx);
                    assert_eq!(window.find("microsoft-save-connect").disabled(), Some(true));
                    reveal(window, "microsoft-tenant-id", cx);
                    window.click("microsoft-tenant-id", cx);
                    window.input("20000000-0000-4000-8000-000000000002", cx);
                } else {
                    assert!(window.try_find("microsoft-tenant-id").is_none());
                }
                reveal(window, "microsoft-setup-back", cx);
                window.click("microsoft-setup-back", cx);
                reveal(window, "microsoft-setup-next", cx);
                window.click("microsoft-setup-next", cx);
                reveal(window, "microsoft-save-connect", cx);
                assert_ne!(
                    window.find("microsoft-save-connect").disabled(),
                    Some(true),
                    "Back keeps entered IDs"
                );
                window.click("microsoft-save-connect", cx);
            })
            .unwrap();
            let config = MicrosoftClient::from_setup(
                kind,
                "10000000-0000-4000-8000-000000000001",
                "20000000-0000-4000-8000-000000000002",
            )
            .unwrap();
            assert!(
                matches!(commands.try_recv(), Ok(Command::Microsoft(MicrosoftCommand::SaveAndConnect(saved))) if saved == config)
            );
            events
                .send_blocking(Event::Microsoft(MicrosoftEvent::SetupChanged(Some(config))))
                .unwrap();

            state(
                &events,
                MicrosoftState::Connecting { url: String::new() },
                cx,
            );
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "microsoft-open-browser", cx);
                assert_eq!(window.find("microsoft-open-browser").disabled(), Some(true));
                reveal(window, "microsoft-cancel", cx);
                window.click("microsoft-cancel", cx);
            })
            .unwrap();
            assert!(matches!(
                commands.try_recv(),
                Ok(Command::Microsoft(MicrosoftCommand::Cancel))
            ));
            let account = MicrosoftAccount {
                id: "fixture".into(),
                name: "Sam".into(),
                email: "sam@example.com".into(),
                granted: MicrosoftService::defaults(),
                turned_off: std::collections::BTreeSet::new(),
            };
            state(&events, MicrosoftState::Connected(account.clone()), cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "microsoft-services", cx);
                window.click("microsoft-services", cx);
                reveal(window, "microsoft-disconnect", cx);
                window.click("microsoft-disconnect", cx);
                reveal(window, "microsoft-disconnect-cancel", cx);
                window.click("microsoft-disconnect-cancel", cx);
            })
            .unwrap();
            assert!(
                commands.try_recv().is_err(),
                "opening choices or cancelling confirmation cannot disconnect"
            );
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "microsoft-disconnect", cx);
                window.click("microsoft-disconnect", cx);
                reveal(window, "microsoft-disconnect-confirm", cx);
                window.click("microsoft-disconnect-confirm", cx);
            })
            .unwrap();
            assert!(matches!(
                commands.try_recv(),
                Ok(Command::Microsoft(MicrosoftCommand::Disconnect))
            ));
            state(&events, MicrosoftState::NeedsSignIn(account), cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "microsoft-reconnect", cx);
                window.click("microsoft-reconnect", cx);
            })
            .unwrap();
            assert!(matches!(
                commands.try_recv(),
                Ok(Command::Microsoft(MicrosoftCommand::Connect))
            ));
            state(&events, MicrosoftState::NotConnected, cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "microsoft-edit-setup", cx);
                window.click("microsoft-edit-setup", cx);
                reveal(window, "microsoft-have-ids", cx);
                window.click("microsoft-have-ids", cx);
                reveal(window, "microsoft-save-connect", cx);
                assert_eq!(
                    window.find("microsoft-save-connect").disabled(),
                    Some(true),
                    "saved IDs do not silently confirm a changed return address"
                );
                reveal(window, "microsoft-edit-cancel", cx);
                window.click("microsoft-edit-cancel", cx);
                reveal(window, "microsoft-forget", cx);
                window.click("microsoft-forget", cx);
                reveal(window, "microsoft-forget-cancel", cx);
                window.click("microsoft-forget-cancel", cx);
            })
            .unwrap();
            assert!(commands.try_recv().is_err());
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "microsoft-forget", cx);
                window.click("microsoft-forget", cx);
                reveal(window, "microsoft-forget-confirm", cx);
                window.click("microsoft-forget-confirm", cx);
            })
            .unwrap();
            assert!(matches!(
                commands.try_recv(),
                Ok(Command::Microsoft(MicrosoftCommand::ForgetSetup))
            ));
            state(&events, MicrosoftState::NeedsSetup, cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "microsoft-setup-step", cx);
                assert!(
                    window
                        .find("microsoft-setup-step")
                        .label()
                        .unwrap()
                        .contains("Step 1 of 4")
                );
            })
            .unwrap();
            cx.update(|cx| {
                design::set_interface_scale(sovra_core::models::InterfaceScale::default(), cx);
            });
        }
    }
}

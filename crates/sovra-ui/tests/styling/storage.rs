//! Online storage: adding, browsing and unlinking a place stays reachable at the
//! minimum window, an ultrawide window and 200% text, in both themes, and the
//! page sends only the commands the person asked for.

use super::*;
use sovra_core::storage::{
    Listing, PlaceStatus, PlaceView, StorageCommand, StorageEntry, StorageEvent, StoragePlace,
    StorageSecret, StorageService,
};
use sovra_core::{Command, Event, NavTab};

fn reveal(
    window: &mut gpui_kit::Window,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut gpui_kit::App,
) {
    let id: gpui_kit::ElementId = id.into();
    for _ in 0..80 {
        window.render_frame(cx);
        let pane = window.find("connections-scroll").bounds();
        let item = window.find(id.clone()).bounds();
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
    window.render_frame(cx);
    panic!(
        "{id} must remain reachable: {:?} in {:?}",
        window.find(id.clone()).bounds(),
        window.find("connections-scroll").bounds()
    );
}

fn send(events: &async_channel::Sender<Event>, event: StorageEvent, cx: &mut TestAppContext) {
    events
        .send_blocking(Event::Storage(event))
        .expect("fixture event accepted");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

/// The storage commands sent since the last call, in order.
fn storage_commands(commands: &async_channel::Receiver<Command>) -> Vec<StorageCommand> {
    let mut sent = Vec::new();
    while let Ok(command) = commands.try_recv() {
        if let Command::Storage(command) = command {
            sent.push(command);
        }
    }
    sent
}

fn saved_place() -> StoragePlace {
    StoragePlace {
        id: "place-1".into(),
        name: "Family photos".into(),
        bucket: "family-photos".into(),
        user: "AKIDEXAMPLE".into(),
        ..StoragePlace::new(StorageService::AmazonS3)
    }
}

#[gpui_kit::test]
fn storage_flow_fits_minimum_wide_and_large_text_in_both_themes(cx: &mut TestAppContext) {
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
            send(&events, StorageEvent::Places(Vec::new()), cx);
            let _ = storage_commands(&commands);

            cx.update_window(window.into(), |_, window, cx| {
                for kind in sovra_core::CatalogueKind::ALL {
                    reveal(
                        window,
                        gpui_kit::SharedString::from(format!("connection-action-{}", kind.slug())),
                        cx,
                    );
                }
                reveal(window, "connections-summary", cx);
                reveal(window, "connection-action-storage", cx);
                assert_eq!(
                    window.find("connection-status-storage").label(),
                    Some("Not linked")
                );
                window.click("connection-action-storage", cx);
                reveal(window, "storage-empty", cx);
                reveal(window, "storage-add", cx);
                window.click("storage-add", cx);
                window.render_frame(cx);
                assert!(window.try_find("storage-choose-service").is_some());
                for slug in ["cloudflare-r2", "azure-blob", "nextcloud", "other-webdav"] {
                    reveal(
                        window,
                        gpui_kit::SharedString::from(format!("storage-service-{slug}")),
                        cx,
                    );
                }
                reveal(window, "storage-service-amazon-s3", cx);
                window.click("storage-service-amazon-s3", cx);
                reveal(window, "storage-keys-help", cx);
                for (field, text) in [
                    ("storage-field-name", "Family photos"),
                    ("storage-field-bucket", "family-photos"),
                    ("storage-field-user", "AKIDEXAMPLE"),
                    ("storage-field-secret", "s3cret"),
                ] {
                    reveal(window, field, cx);
                    window.click(field, cx);
                    window.input(text, cx);
                }
                reveal(window, "storage-address-preview", cx);
                assert!(
                    window
                        .find("storage-address-preview")
                        .label()
                        .is_none_or(|text| !text.contains("s3cret"))
                );
                reveal(window, "storage-allow-saving", cx);
                window.click("storage-allow-saving", cx);
                reveal(window, "storage-save", cx);
                window.click("storage-save", cx);
            })
            .unwrap();
            cx.run_until_parked();
            let sent = storage_commands(&commands);
            let [StorageCommand::Save { place, secret }] = sent.as_slice() else {
                panic!("one save, got {sent:?}");
            };
            assert_eq!(place.service, StorageService::AmazonS3);
            assert_eq!(place.name, "Family photos");
            assert_eq!(place.bucket, "family-photos");
            assert_eq!(place.region, "us-east-1");
            assert!(place.allow_saving);
            assert_eq!(secret.as_ref().map(StorageSecret::as_str), Some("s3cret"));

            // A refusal keeps the form open with the reason.
            send(
                &events,
                StorageEvent::SaveRefused("The bucket was not found.".into()),
                cx,
            );
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "storage-form-error", cx);
                assert_eq!(
                    window.find("storage-form-error").label(),
                    Some("The bucket was not found.")
                );
            })
            .unwrap();

            let view = PlaceView {
                place: saved_place(),
                has_secret: true,
            };
            send(&events, StorageEvent::Places(vec![view]), cx);
            send(
                &events,
                StorageEvent::Saved {
                    id: "place-1".into(),
                    name: "Family photos".into(),
                },
                cx,
            );
            send(
                &events,
                StorageEvent::Status {
                    id: "place-1".into(),
                    status: PlaceStatus::Works,
                },
                cx,
            );
            cx.update_window(window.into(), |_, window, cx| {
                assert!(window.try_find("storage-form").is_none(), "form closed");
                reveal(window, "storage-notice", cx);
                reveal(window, ("storage-place-status", 0usize), cx);
                assert_eq!(
                    window.find(("storage-place-status", 0usize)).label(),
                    Some("Working")
                );
                reveal(window, ("storage-browse", 0usize), cx);
                window.click(("storage-browse", 0usize), cx);
            })
            .unwrap();
            cx.run_until_parked();
            assert_eq!(
                storage_commands(&commands),
                [StorageCommand::Browse {
                    id: "place-1".into(),
                    folder: String::new()
                }]
            );
            let entry = |path: &str, folder| StorageEntry {
                path: path.into(),
                name: StorageEntry::name_of(path),
                folder,
                size: (!folder).then_some(2048),
                modified: Some("2026-11-01T10:00:00.000Z".into()),
            };
            send(
                &events,
                StorageEvent::Listing {
                    id: "place-1".into(),
                    folder: String::new(),
                    result: Ok(Listing {
                        entries: vec![
                            entry("Holiday 2026", true),
                            entry(
                                "a very long file name that must truncate inside the pane instead of pushing the layout wider than the window.jpg",
                                false,
                            ),
                        ],
                        more: false,
                    }),
                },
                cx,
            );
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "storage-browser-location", cx);
                reveal(window, ("storage-entry", 0usize), cx);
                window.click(("storage-entry", 0usize), cx);
            })
            .unwrap();
            cx.run_until_parked();
            assert_eq!(
                storage_commands(&commands),
                [StorageCommand::Browse {
                    id: "place-1".into(),
                    folder: "Holiday 2026".into()
                }]
            );

            // Unlinking asks twice.
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "storage-browser-close", cx);
                window.click("storage-browser-close", cx);
                reveal(window, ("storage-remove", 0usize), cx);
                window.click(("storage-remove", 0usize), cx);
            })
            .unwrap();
            cx.run_until_parked();
            assert!(
                storage_commands(&commands).is_empty(),
                "first press only asks"
            );
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, ("storage-remove", 0usize), cx);
                window.click(("storage-remove", 0usize), cx);
            })
            .unwrap();
            cx.run_until_parked();
            assert_eq!(
                storage_commands(&commands),
                [StorageCommand::Remove {
                    id: "place-1".into()
                }]
            );
            cx.update_window(window.into(), |_, window, _| window.remove_window())
                .unwrap();
        }
    }
}

//! Knowledge → People: one person, several ways to reach them, and a review before sending.

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ScrollDelta, TestAppContext, point, px, size};
use sovra_core::communications::{MessageDelivery, OutboundMessage};
use sovra_core::connections::{ChannelKind, ConnectionCommand, ConnectionEvent};
use sovra_core::models::InterfaceScale;
use sovra_core::people::{PeopleBook, Reach};
use sovra_core::{Command, Event, Timestamp};
use sovra_runtime::Client;
use sovra_ui::Shell;

fn saved_person() -> (PeopleBook, String) {
    let mut book = PeopleBook::default();
    assert!(book.observe_handle(Reach::WhatsApp, "+447700900111", "Sam Rivera"));
    let id = book.people[0].id.clone();
    book.attach(&id, Reach::Email, "sam@school.example")
        .expect("email joins the same person");
    assert_eq!(book.people.len(), 1);
    (book, id)
}

fn scroll_into(
    window: &mut gpui_kit::Window,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut gpui_kit::App,
) {
    let id = id.into();
    for _ in 0..40 {
        window.render_frame(cx);
        let row = window.find(id.clone());
        let pane = window.find("sovra-knowledge-view").bounds();
        if row.visible()
            && row.bounds().top() >= pane.top()
            && row.bounds().bottom() <= pane.bottom()
        {
            break;
        }
        let delta = if row.bounds().top() < pane.top() {
            80.
        } else {
            -80.
        };
        window.scroll(
            "sovra-knowledge-view",
            ScrollDelta::Pixels(point(px(0.), px(delta))),
            cx,
        );
    }
}

#[gpui_kit::test]
fn people_list_uses_the_same_words_in_both_themes(cx: &mut TestAppContext) {
    let mut first = None;
    for _run in 0..2 {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
            });
            let (mut book, person_id) = saved_person();
            assert!(book.observe_handle(Reach::WhatsApp, "+447700900222", "Sam Rivera"));
            let mut shell = None;
            let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.expect("shell");
            events
                .send_blocking(Event::Connections(ConnectionEvent::PeopleChanged(book)))
                .expect("people");
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            let session = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.open_knowledge_people(cx);
                    shell.people_message_draft().update(cx, |input, cx| {
                        input.set_value("The school gate opens at 3.", window, cx);
                    });
                });
                window.render_frame(cx);
                assert!(window.try_find("people-empty").is_none());
                scroll_into(window, ("person-prepare", 0usize), cx);
                let found = labels_in(window);
                if let Some(earlier) = &first {
                    assert_eq!(&found, earlier, "{mode:?}");
                } else {
                    first = Some(found);
                }
                let words = first.clone().expect("labels");
                let joined = words.join(" ").to_ascii_lowercase();
                assert!(joined.contains("sam rivera"));
                assert!(joined.contains("whatsapp"));
                assert!(joined.contains("email"));
                assert!(joined.contains("+447700900111"));
                assert!(joined.contains("sam@school.example"));
                assert!(joined.contains("nothing is sent until you press send"));
                let first_card = window
                    .find(("person-channels", 0usize))
                    .label()
                    .unwrap_or("")
                    .to_owned();
                let second_card = window
                    .find(("person-channels", 1usize))
                    .label()
                    .unwrap_or("")
                    .to_owned();
                assert!(second_card.contains("+447700900222"));
                assert_ne!(first_card, second_card);
                for forbidden in ["token", "secret", "client id", "persona", "export"] {
                    assert!(
                        !joined.contains(forbidden),
                        "people list says {forbidden}: {joined}"
                    );
                }
                window.click(("person-prepare", 0usize), cx);
            })
            .expect("people");
            let prepared: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
            assert!(prepared.iter().any(|command| {
                matches!(
                    command,
                    Command::Connections(ConnectionCommand::PrepareMessage {
                        recipient,
                        text,
                        channel: ChannelKind::WhatsApp,
                        ..
                    }) if recipient == &format!("person:{person_id}")
                        && text == "The school gate opens at 3."
                )
            }));
            assert!(
                !prepared.iter().any(|command| matches!(
                    command,
                    Command::Connections(ConnectionCommand::ReviewMessage { send: true, .. })
                )),
                "preparing must not send"
            );
            events
                .send_blocking(Event::Connections(ConnectionEvent::OutboundChanged(
                    OutboundMessage {
                        id: "review-sam".into(),
                        session_id: session,
                        channel: ChannelKind::WhatsApp,
                        sender: "+447700900123".into(),
                        recipient: "447700900111@s.whatsapp.net".into(),
                        recipient_label: "Sam Rivera".into(),
                        text: "The school gate opens at 3.".into(),
                        delivery: MessageDelivery::Review,
                        created_at: Timestamp(0),
                        expires_at: Timestamp(i64::MAX),
                    },
                )))
                .expect("review");
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                assert_eq!(
                    shell.read(cx).workspace().active_tab,
                    sovra_core::NavTab::Chat,
                    "Message on WhatsApp opens the review"
                );
                window.render_frame(cx);
                for _ in 0..40 {
                    window.render_frame(cx);
                    let send = window.find(("message-review-send", 0usize));
                    let pane = window.find("chat-scroll-area").bounds();
                    if send.visible()
                        && send.bounds().top() >= pane.top()
                        && send.bounds().bottom() <= pane.bottom()
                    {
                        break;
                    }
                    let delta = if send.bounds().top() < pane.top() {
                        30.
                    } else {
                        -30.
                    };
                    window.scroll(
                        "chat-scroll-area",
                        ScrollDelta::Pixels(point(px(0.), px(delta))),
                        cx,
                    );
                }
                assert!(window.find(("message-review-send", 0usize)).visible());
            })
            .expect("review");
            assert!(
                !std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                    command,
                    Command::Connections(ConnectionCommand::ReviewMessage { send: true, .. })
                )),
                "showing the review must not send"
            );
        }
    }
}

fn labels_in(window: &gpui_kit::Window) -> Vec<String> {
    [
        "people-intro",
        "people-join-hint",
        "people-safety",
        "people-telegram-note",
        "people-reach-prompt",
    ]
    .into_iter()
    .map(|id| window.find(id).label().unwrap_or("").to_owned())
    .chain([
        window
            .find(("person-card", 0usize))
            .label()
            .unwrap_or("")
            .to_owned(),
        window
            .find(("person-channels", 0usize))
            .label()
            .unwrap_or("")
            .to_owned(),
        window
            .find(("person-prepare", 0usize))
            .label()
            .unwrap_or("")
            .to_owned(),
    ])
    .collect()
}

#[gpui_kit::test]
fn people_list_stays_usable_at_minimum_wide_and_double_text(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (client, _commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                sovra_ui::design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            let (book, _) = saved_person();
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.expect("shell");
            events
                .send_blocking(Event::Connections(ConnectionEvent::PeopleChanged(book)))
                .expect("people");
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, Shell::open_knowledge_people);
                window.render_frame(cx);
                scroll_into(window, ("person-card", 0usize), cx);
                let card = window.find(("person-card", 0usize));
                assert!(
                    card.visible(),
                    "{mode:?} {width}x{height} @{scale}: {card:?}"
                );
                assert!(card.bounds().left() >= px(0.));
                assert!(card.bounds().right() <= px(width));
                scroll_into(window, ("person-prepare", 0usize), cx);
                let prepare = window.find(("person-prepare", 0usize));
                let pane = window.find("sovra-knowledge-view").bounds();
                assert!(prepare.visible(), "{mode:?} {width} @{scale}");
                assert!(prepare.bounds().left() >= px(0.));
                assert!(prepare.bounds().right() <= px(width));
                assert!(prepare.bounds().top() >= pane.top());
                assert!(prepare.bounds().bottom() <= pane.bottom());
                assert!(prepare.bounds().bottom() <= px(height));
            })
            .expect("layout");
        }
    }
}

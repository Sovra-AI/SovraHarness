//! A web page on an artifact card stays inside the card as source lines.

#![allow(
    clippy::unwrap_used,
    reason = "headless UI fixtures fail loudly when the scripted runtime is gone"
)]

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Pixels, TestAppContext, Window, px, size};
use sovra_core::artifacts::{Artifact, ArtifactDraft, ArtifactEvent, ArtifactKind, ArtifactSource};
use sovra_core::group::{
    DetailStatus, DeviceId, GroupEvent, GroupId, GroupState, MembershipBody, OpHash, PersonId,
    Role, SharedArtifact, Signature, SignatureVerifier, Space, UnsignedOp, fold,
};
use sovra_core::models::InterfaceScale;
use sovra_core::{Event, NavTab, Timestamp};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

const PAGE: &str = "<!DOCTYPE html> <html lang=\"en\"> <head> <meta charset=\"UTF-8\"> <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"> <title>Cyber Council</title></head></html>";

struct AcceptAll;

impl SignatureVerifier for AcceptAll {
    fn verify(&self, _: &DeviceId, _: &[u8], _: &Signature) -> bool {
        true
    }
}

fn family() -> GroupState {
    let mum = DeviceId([1; 32]);
    let dad = DeviceId([2; 32]);
    let nonce = [9; 16];
    let group = GroupId::derive(&mum, &nonce);
    let unsigned = UnsignedOp {
        group,
        author: mum,
        seq: 1,
        prev: None,
        deps: Vec::new(),
        at: Timestamp(1),
        body: MembershipBody::Genesis {
            name: "Family".into(),
            nonce,
            founder: PersonId([10; 32]),
            founder_name: "Mum".into(),
            device_name: "Mum's laptop".into(),
        },
    };
    let mut ops = vec![unsigned.sign(Signature([0; 64]))];
    for body in [
        MembershipBody::AddPerson {
            person: PersonId([20; 32]),
            name: "Dad".into(),
            role: Role::Member,
        },
        MembershipBody::AddDevice {
            person: PersonId([20; 32]),
            device: dad,
            name: "Dad's desktop".into(),
        },
    ] {
        let draft = fold(group, &ops, &AcceptAll).draft(mum, Timestamp(2), body);
        ops.push(draft.sign(Signature([0; 64])));
    }
    fold(group, &ops, &AcceptAll)
}

fn webpage() -> SharedArtifact {
    SharedArtifact {
        id: OpHash([9; 32]),
        title: "Website".into(),
        kind: ArtifactKind::Webpage,
        language: "html".into(),
        content: PAGE.into(),
        author: DeviceId([2; 32]),
        person: Some(PersonId([20; 32])),
        shared_by: "shaun".into(),
        computer: "Dad's desktop".into(),
        shared_at: Timestamp(4),
        replaces: Vec::new(),
        status: DetailStatus::Current,
        from_former_member: false,
        origin: "art-web".into(),
        space: Space::Group,
    }
}

fn saved(id: &str, title: &str, kind: ArtifactKind, content: &str, at: i64) -> Artifact {
    Artifact::from_draft(
        id.into(),
        ArtifactDraft {
            title: title.into(),
            kind,
            language: if kind == ArtifactKind::Webpage {
                "html".into()
            } else {
                String::new()
            },
            content: content.into(),
            source: ArtifactSource::default(),
        },
        Timestamp(at),
    )
}

#[gpui_kit::test]
fn code_artifact_cards_show_source_inside_the_card(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (client, _commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.unwrap();
            events
                .send_blocking(Event::Group(Box::new(GroupEvent::Loaded {
                    this_computer: DeviceId([1; 32]),
                    group: Some(family()),
                })))
                .unwrap();
            events
                .send_blocking(Event::Group(Box::new(GroupEvent::Artifacts(vec![
                    webpage(),
                ]))))
                .unwrap();
            events
                .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![
                    saved("page", "Website", ArtifactKind::Webpage, PAGE, 2),
                    saved(
                        "note",
                        "Reminder",
                        ArtifactKind::Note,
                        "Remember the bins tomorrow morning.",
                        1,
                    ),
                ])))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
                window.render_frame(cx);
                let where_ = format!("{width}×{height} at {scale}% {mode:?}");
                let note = window.find(("artifact-blurb", 1usize));
                let note_label = note.label().unwrap_or_default();
                assert!(
                    note_label.contains("Remember the bins"),
                    "a note should stay a sentence at {where_}: {note_label}"
                );
                assert!(
                    !note_label.contains("DOCTYPE"),
                    "a note was drawn as source at {where_}"
                );
                let page = assert_source(
                    window,
                    ("artifact-card", 0usize),
                    ("artifact-blurb", 0usize),
                    ("artifact-open", 0usize),
                    &where_,
                );
                assert!(
                    page.size.height > note.bounds().size.height * 2.,
                    "the page should be several source lines at {where_}: {page:?} vs {:?}",
                    note.bounds()
                );
                assert_source(
                    window,
                    ("shared-artifact-card", 0usize),
                    ("shared-artifact-blurb", 0usize),
                    ("shared-artifact-open", 0usize),
                    &where_,
                );
                window.remove_window();
            })
            .unwrap();
        }
    }
}

fn assert_source(
    window: &mut Window,
    card_id: (&'static str, usize),
    blurb_id: (&'static str, usize),
    open_id: (&'static str, usize),
    where_: &str,
) -> Bounds<Pixels> {
    let card = window.find(card_id).bounds();
    let blurb = window.find(blurb_id);
    let label = blurb.label().unwrap_or_default();
    assert!(
        label.contains("<!DOCTYPE html>") && label.contains("Cyber Council"),
        "source dropped the page at {where_}: {label}"
    );
    assert!(
        !label.contains("initial-scale=1\"> <title"),
        "the page is still one smashed line at {where_}: {label}"
    );
    let bounds = blurb.bounds();
    assert!(
        bounds.left() >= card.left() - px(1.)
            && bounds.right() <= card.right() + px(1.)
            && bounds.top() >= card.top() - px(1.)
            && bounds.bottom() <= card.bottom() + px(1.),
        "source runs outside the card at {where_}: source {bounds:?} card {card:?}"
    );
    let open = window.find(open_id).bounds();
    assert!(
        bounds.bottom() <= open.top() + px(1.),
        "source covers Open at {where_}: source {bounds:?} open {open:?}"
    );
    bounds
}

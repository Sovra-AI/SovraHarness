//! Settings → Groups: start or join a group, see who and which computers are in it,
//! invite people, and share details everyone's helpers can use. Plain words only:
//! "computer", "group", "share"; the rules live in `sovra_core::group`.

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window,
    div,
};
use sovra_core::Command;
use sovra_core::group::{
    DetailStatus, FederatedSettings, GroupCommand, InviteKind, PairingStage, Role, SharedDetail,
    removal_warning,
};

use crate::RuntimeClient;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::models_view::{ModelsView, caption, pill};
use crate::settings_view::settings_section;

/// The text boxes on the Groups section.
pub(crate) struct GroupsForm {
    group_name: Entity<InputState>,
    your_name: Entity<InputState>,
    computer_name: Entity<InputState>,
    join_link: Entity<InputState>,
    invite_name: Entity<InputState>,
    share: Entity<InputState>,
    ask: Entity<InputState>,
    join_code: Entity<InputState>,
    /// Share with the whole group (`None`) or just this person.
    share_with: Option<sovra_core::group::PersonId>,
    /// Which person or computer is waiting for "Yes, remove".
    confirm_remove: Option<String>,
}

impl GroupsForm {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<ModelsView>) -> Self {
        let input =
            |placeholder: &'static str, window: &mut Window, cx: &mut Context<ModelsView>| {
                cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
            };
        Self {
            group_name: input("Group name, like Family", window, cx),
            your_name: input("Your name", window, cx),
            computer_name: input("This computer's name, like Mum's laptop", window, cx),
            join_link: input("Paste the sovra://join link you were sent", window, cx),
            invite_name: input("Their name", window, cx),
            share: input(
                "Something your group should know, like “The boiler service is on 12 March”",
                window,
                cx,
            ),
            ask: input(
                "Ask everyone's memories, like “Who is our dentist?”",
                window,
                cx,
            ),
            join_code: input("The six digits on the other computer", window, cx),
            share_with: None,
            confirm_remove: None,
        }
    }
}

fn value(input: &Entity<InputState>, cx: &Context<ModelsView>) -> String {
    input.read(cx).value().trim().to_owned()
}

fn send(command: GroupCommand, cx: &mut Context<ModelsView>) {
    let _ = cx.global::<RuntimeClient>().0.send(Command::Group(command));
}

fn status_label(detail: &SharedDetail) -> Option<&'static str> {
    if detail.from_former_member {
        return Some("From someone no longer in the group — not used");
    }
    match detail.status {
        DetailStatus::Current => None,
        DetailStatus::Replaced { .. } => Some("Corrected — not used"),
        DetailStatus::Disputed { .. } => Some("People disagree — both are shown"),
        DetailStatus::Withdrawn => Some("Stopped — not used"),
        DetailStatus::Redacted => Some("Words erased"),
    }
}

impl ModelsView {
    pub(crate) fn render_groups_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let view = &self.groups;
        let mut section = settings_section(
            "settings-groups",
            "Groups",
            "Share helpers and knowledge with your family or team, computer to computer. Nothing leaves your computers through a Sovra server, nothing is shared until you choose to share it, and what you share is locked so only computers in your group can read it.",
            cx,
        );
        if let Some(problem) = &view.problem {
            section = section.child(
                div()
                    .id("groups-problem")
                    .role(gpui_kit::Role::Status)
                    .aria_label(problem.message().to_owned())
                    .test_support()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(cx.theme().danger)
                    .child(problem.message().to_owned()),
            );
        }
        if !view.loaded {
            return section
                .child(caption("Getting your group ready…", cx))
                .into_any_element();
        }
        if let Some(check) = self.render_picture_check(cx) {
            section = section.child(check);
        }
        match &view.group {
            None => section
                .child(self.render_start_group(cx))
                .child(self.render_join_group(cx))
                .into_any_element(),
            Some(group) => {
                let mut body = section.child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap(t.space.small)
                        .child(
                            div()
                                .sovra_text(TextRole::Body, cx)
                                .child(group.name.clone()),
                        )
                        .child(pill(
                            format!(
                                "{} people · {} computers",
                                group.people.len(),
                                group
                                    .people
                                    .values()
                                    .map(|m| m.devices.len())
                                    .sum::<usize>()
                            ),
                            cx,
                        )),
                );
                if view.show_second_admin_banner() {
                    body = body.child(caption(
                        "Add a second admin so you're never locked out of your group.",
                        cx,
                    ));
                }
                body.child(self.render_roster(cx))
                    .child(self.render_invites(cx))
                    .child(self.render_knowledge(cx))
                    .child(self.render_templates(cx))
                    .child(self.render_federated(cx))
                    .child(
                        h_flex().child(
                            design::button("groups-leave", "Leave this group", cx)
                                .ghost()
                                .small()
                                .accessibility_label("Leave this group on this computer")
                                .on_click(cx.listener(|_, _, _, cx| send(GroupCommand::Leave, cx))),
                        ),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_start_group(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let form = &self.groups_form;
        v_flex()
            .id("groups-start")
            .test_support()
            .w_full()
            .gap(t.space.compact)
            .p(t.space.compact)
            .sovra_surface(Surface::Subtle, cx)
            .child(div().sovra_text(TextRole::Body, cx).child("Start a group"))
            .child(
                Input::new(&form.group_name)
                    .id("groups-start-name")
                    .aria_label("Group name"),
            )
            .child(
                Input::new(&form.your_name)
                    .id("groups-start-your-name")
                    .aria_label("Your name"),
            )
            .child(
                Input::new(&form.computer_name)
                    .id("groups-start-computer-name")
                    .aria_label("This computer's name"),
            )
            .child(h_flex().child(
                design::primary_button("groups-create", "Start the group", cx).on_click(
                    cx.listener(|view, _, _, cx| {
                        let form = &view.groups_form;
                        let command = GroupCommand::Create {
                            name: value(&form.group_name, cx),
                            your_name: value(&form.your_name, cx),
                            computer_name: value(&form.computer_name, cx),
                        };
                        send(command, cx);
                    }),
                ),
            ))
    }

    fn render_join_group(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let form = &self.groups_form;
        let joining = matches!(self.groups.pairing, PairingStage::Connecting);
        v_flex()
            .id("groups-join")
            .test_support()
            .w_full()
            .gap(t.space.compact)
            .p(t.space.compact)
            .sovra_surface(Surface::Subtle, cx)
            .child(div().sovra_text(TextRole::Body, cx).child("Join a group"))
            .child(caption(
                "Ask someone in the group for an invite link, then paste it here.",
                cx,
            ))
            .child(
                Input::new(&form.join_link)
                    .id("groups-join-link")
                    .aria_label("Invite link"),
            )
            .child(
                Input::new(&form.computer_name)
                    .id("groups-join-computer-name")
                    .aria_label("This computer's name"),
            )
            .child(
                h_flex()
                    .gap(t.space.small)
                    .items_center()
                    .child(
                        design::button("groups-join-button", "Join", cx).on_click(cx.listener(
                            |view, _, _, cx| {
                                let form = &view.groups_form;
                                let command = GroupCommand::Join {
                                    link: value(&form.join_link, cx),
                                    computer_name: value(&form.computer_name, cx),
                                };
                                send(command, cx);
                            },
                        )),
                    )
                    .children(joining.then(|| caption("Connecting to the other computer…", cx))),
            )
            .child(caption(
                "In the same room? Ask them to show a code in their Groups, then type the six digits here.",
                cx,
            ))
            .child(
                Input::new(&form.join_code)
                    .id("groups-join-code")
                    .aria_label("Six-digit code from the other computer"),
            )
            .child(h_flex().child(
                design::button("groups-join-code-button", "Join with code", cx).on_click(
                    cx.listener(|view, _, _, cx| {
                        let form = &view.groups_form;
                        let command = GroupCommand::JoinWithCode {
                            code: value(&form.join_code, cx),
                            computer_name: value(&form.computer_name, cx),
                        };
                        send(command, cx);
                    }),
                ),
            ))
    }

    /// "Do these pictures match the other screen?" — both people answer before a
    /// computer joins by code.
    fn render_picture_check(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let PairingStage::ConfirmCode { code, here, .. } = &self.groups.pairing else {
            return None;
        };
        let t = design::tokens(cx);
        let pictures: String = code
            .emoji
            .iter()
            .map(|(picture, _)| *picture)
            .collect::<Vec<_>>()
            .join("  ");
        let mut check = v_flex()
            .id("groups-picture-check")
            .test_support()
            .w_full()
            .gap(t.space.small)
            .p(t.space.medium)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .child("Check the other screen shows the same four pictures and numbers."),
            )
            .child(
                div()
                    .id("groups-pictures")
                    .test_support()
                    .aria_label(code.spoken())
                    .sovra_text(TextRole::PageTitle, cx)
                    .child(pictures),
            )
            .child(caption(format!("{} · {}", code.spoken(), code.digits), cx));
        check = match here {
            None => check.child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        design::primary_button("groups-pictures-match", "They match", cx).on_click(
                            cx.listener(|_, _, _, cx| {
                                send(GroupCommand::ConfirmCode { matches: true }, cx);
                            }),
                        ),
                    )
                    .child(
                        design::button("groups-pictures-differ", "They don't", cx).on_click(
                            cx.listener(|_, _, _, cx| {
                                send(GroupCommand::ConfirmCode { matches: false }, cx);
                            }),
                        ),
                    ),
            ),
            Some(_) => check.child(caption("Waiting for the other person to check…", cx)),
        };
        Some(check)
    }

    fn render_roster(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let manage = self.groups.can_manage();
        let confirm = self.groups_form.confirm_remove.clone();
        v_flex()
            .id("groups-roster")
            .test_support()
            .w_full()
            .gap(t.space.compact)
            .children(
                self.groups
                    .rows()
                    .into_iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let person = row.person;
                        let person_key = format!("person:{}", person.to_hex());
                        let mine = row
                            .computers
                            .iter()
                            .any(|computer| computer.is_this_computer);
                        let mut card = v_flex()
                            .id(("groups-person", index))
                            .test_support()
                            .w_full()
                            .min_w_0()
                            .gap(t.space.compact)
                            .p(t.space.compact)
                            .sovra_surface(Surface::Subtle, cx)
                            .child(
                                h_flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(t.space.small)
                                    .child(
                                        div()
                                            .sovra_text(TextRole::Body, cx)
                                            .child(row.name.clone()),
                                    )
                                    .child(caption(row.role_label, cx)),
                            );
                        for (slot, computer) in row.computers.iter().enumerate() {
                            let device = computer.device;
                            let key = format!("computer:{}", device.to_hex());
                            let label = if computer.is_this_computer {
                                format!(
                                    "{} (this computer) · {}",
                                    computer.name, computer.fingerprint
                                )
                            } else {
                                format!("{} · {}", computer.name, computer.fingerprint)
                            };
                            let name = computer.name.clone();
                            let mut line = h_flex()
                                .w_full()
                                .min_w_0()
                                .flex_wrap()
                                .items_center()
                                .gap(t.space.small)
                                .child(caption(label, cx).flex_1().min_w_0().truncate());
                            if (manage || mine) && !computer.is_this_computer {
                                if confirm.as_deref() == Some(key.as_str()) {
                                    line = line
                                        .child(
                                            caption(removal_warning(&name), cx).whitespace_normal(),
                                        )
                                        .child(
                                            design::button(
                                                ("groups-remove-computer-yes", index * 100 + slot),
                                                "Yes, remove",
                                                cx,
                                            )
                                            .small()
                                            .on_click(
                                                cx.listener(move |view, _, _, cx| {
                                                    view.groups_form.confirm_remove = None;
                                                    send(
                                                        GroupCommand::RemoveComputer { device },
                                                        cx,
                                                    );
                                                }),
                                            ),
                                        );
                                } else {
                                    line = line.child(
                                        design::button(
                                            ("groups-remove-computer", index * 100 + slot),
                                            "Remove",
                                            cx,
                                        )
                                        .ghost()
                                        .xsmall()
                                        .accessibility_label(format!(
                                            "Remove {name} from the group"
                                        ))
                                        .on_click(
                                            cx.listener(move |view, _, _, cx| {
                                                view.groups_form.confirm_remove = Some(key.clone());
                                                cx.notify();
                                            }),
                                        ),
                                    );
                                }
                            }
                            card = card.child(line);
                        }
                        if manage && !mine {
                            let make_admin = row.role != Role::Admin;
                            card = card.child(
                                h_flex()
                                    .flex_wrap()
                                    .gap(t.space.small)
                                    .child(
                                        design::button(
                                            ("groups-role", index),
                                            if make_admin {
                                                "Make admin"
                                            } else {
                                                "Make member"
                                            },
                                            cx,
                                        )
                                        .ghost()
                                        .xsmall()
                                        .on_click(
                                            cx.listener(move |_, _, _, cx| {
                                                let role = if make_admin {
                                                    Role::Admin
                                                } else {
                                                    Role::Member
                                                };
                                                send(GroupCommand::ChangeRole { person, role }, cx);
                                            }),
                                        ),
                                    )
                                    .child(if confirm.as_deref() == Some(person_key.as_str()) {
                                        design::button(
                                            ("groups-remove-person-yes", index),
                                            "Yes, remove them",
                                            cx,
                                        )
                                        .small()
                                        .on_click(
                                            cx.listener(move |view, _, _, cx| {
                                                view.groups_form.confirm_remove = None;
                                                send(GroupCommand::RemovePerson { person }, cx);
                                            }),
                                        )
                                    } else {
                                        design::button(
                                            ("groups-remove-person", index),
                                            "Remove person",
                                            cx,
                                        )
                                        .ghost()
                                        .xsmall()
                                        .on_click(
                                            cx.listener(move |view, _, _, cx| {
                                                view.groups_form.confirm_remove =
                                                    Some(person_key.clone());
                                                cx.notify();
                                            }),
                                        )
                                    }),
                            );
                        }
                        card
                    }),
            )
    }

    fn render_invites(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let manage = self.groups.can_manage();
        let mut invites = v_flex()
            .id("groups-invites")
            .test_support()
            .w_full()
            .gap(t.space.compact)
            .child(div().sovra_text(TextRole::Body, cx).child("Invite"));
        if manage {
            invites = invites
                .child(
                    Input::new(&self.groups_form.invite_name)
                        .id("groups-invite-name")
                        .aria_label("Their name"),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            design::button("groups-invite-member", "Invite as member", cx)
                                .small()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    let name = value(&view.groups_form.invite_name, cx);
                                    send(
                                        GroupCommand::InvitePerson {
                                            name,
                                            role: Role::Member,
                                        },
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            design::button("groups-invite-admin", "Invite as admin", cx)
                                .small()
                                .ghost()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    let name = value(&view.groups_form.invite_name, cx);
                                    send(
                                        GroupCommand::InvitePerson {
                                            name,
                                            role: Role::Admin,
                                        },
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            design::button(
                                "groups-code-person",
                                "In the same room? Show a code",
                                cx,
                            )
                            .small()
                            .ghost()
                            .on_click(cx.listener(|view, _, _, cx| {
                                let name = value(&view.groups_form.invite_name, cx);
                                send(
                                    GroupCommand::ShowShortCode {
                                        kind: InviteKind::Person {
                                            name,
                                            role: Role::Member,
                                        },
                                    },
                                    cx,
                                );
                            })),
                        ),
                );
        }
        invites = invites.child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("groups-invite-computer", "Add another of my computers", cx)
                        .small()
                        .ghost()
                        .on_click(
                            cx.listener(|_, _, _, cx| send(GroupCommand::InviteComputer, cx)),
                        ),
                )
                .child(
                    design::button(
                        "groups-code-computer",
                        "Show a code for my other computer",
                        cx,
                    )
                    .small()
                    .ghost()
                    .on_click(cx.listener(|_, _, _, cx| {
                        send(
                            GroupCommand::ShowShortCode {
                                kind: InviteKind::OwnDevice,
                            },
                            cx,
                        );
                    })),
                ),
        );
        if let Some((code, expires_at)) = self.groups.short_code.clone() {
            let minutes = (expires_at.millis() - now_millis()).max(0) / 60_000;
            invites = invites
                .child(caption(
                    format!(
                        "Type this on the other computer, on the same Wi-Fi. It works for one try, for about {} minutes.",
                        minutes + 1
                    ),
                    cx,
                ))
                .child(
                    div()
                        .id("groups-short-code")
                        .test_support()
                        .aria_label(code.clone())
                        .sovra_text(TextRole::PageTitle, cx)
                        .child(code),
                );
        }
        if let Some((link, expires_at)) = self.groups.invite.clone() {
            let mono = cx.theme().mono_font_family.clone();
            let minutes = (expires_at.millis() - now_millis()).max(0) / 60_000;
            invites = invites
                .child(caption(
                    format!(
                        "Send this link to them. It works once and expires in about {} minutes.",
                        minutes + 1
                    ),
                    cx,
                ))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(t.space.small)
                        .child(
                            div()
                                .id("groups-invite-link")
                                .test_support()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(mono)
                                .sovra_text(TextRole::Caption, cx)
                                .child(link.clone()),
                        )
                        .child(
                            design::button("groups-copy-link", "Copy", cx)
                                .xsmall()
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                                }),
                        ),
                );
            if !self.groups.invite_qr.is_empty() {
                invites = invites
                    .child(caption("Or scan this with the other device's camera.", cx))
                    .child(h_flex().child(crate::connections_view::scan_code(
                        "groups-invite-qr",
                        self.groups.invite_qr.clone(),
                        cx,
                    )));
            }
        }
        invites
    }

    fn render_knowledge(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let me = self.groups.this_computer;
        let manage = self.groups.can_manage();
        let my_person = me.and_then(|device| {
            self.groups
                .group
                .as_ref()
                .and_then(|group| group.owner_of(&device))
                .map(|member| member.person)
        });
        let can_add = self.groups.my_role().is_some_and(Role::writes);
        let artifacts = self
            .groups
            .artifacts
            .iter()
            .filter(|artifact| artifact.is_latest())
            .count();
        let mut section = v_flex()
            .id("groups-knowledge")
            .test_support()
            .w_full()
            .gap(t.space.compact)
            .child(div().sovra_text(TextRole::Body, cx).child("Shared with the group"))
            .child(caption(
                "Helpers on every computer in the group can use these, and they say who shared each one. To share something you already saved, open it in Knowledge or Artifacts and press Share.",
                cx,
            ))
            .child(
                div()
                    .id("groups-shared-artifacts")
                    .test_support()
                    .child(caption(
                        match artifacts {
                            0 => "No documents, lists or pictures shared yet. They appear on the Artifacts page.".to_owned(),
                            1 => "1 document, list or picture is shared — see the Artifacts page.".to_owned(),
                            n => format!("{n} documents, lists and pictures are shared — see the Artifacts page."),
                        },
                        cx,
                    )),
            );
        if can_add {
            let chosen = self.groups_form.share_with;
            let mut audience = h_flex()
                .id("groups-share-with")
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("groups-share-everyone", "Everyone", cx)
                        .small()
                        .selected(chosen.is_none())
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.groups_form.share_with = None;
                            cx.notify();
                        })),
                );
            for (index, (person, name)) in self.groups.others().into_iter().enumerate() {
                audience = audience.child(
                    design::button(("groups-share-just", index), format!("Just {name}"), cx)
                        .small()
                        .selected(chosen == Some(person))
                        .accessibility_label(format!("Share only with {name}"))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.groups_form.share_with = Some(person);
                            cx.notify();
                        })),
                );
            }
            section = section
                .child(
                    Input::new(&self.groups_form.share)
                        .id("groups-share-input")
                        .aria_label("Something to share with your group"),
                )
                .child(audience)
                .child(h_flex().child(
                    design::primary_button("groups-share", "Share", cx).on_click(cx.listener(
                        |view, _, window, cx| {
                            let text = value(&view.groups_form.share, cx);
                            view.groups_form
                                .share
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            send(
                                GroupCommand::Share {
                                    text,
                                    about: Vec::new(),
                                    with: view.groups_form.share_with,
                                },
                                cx,
                            );
                        },
                    )),
                ));
        }
        section
            .children(self.groups.knowledge.iter().enumerate().map(|(index, detail)| {
                let id = detail.id;
                let mine = detail.person.is_some() && detail.person == my_person;
                let mut row = v_flex()
                    .id(("groups-detail", index))
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.compact)
                    .p(t.space.compact)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(
                        div()
                            .whitespace_normal()
                            .sovra_text(TextRole::Body, cx)
                            .child(if detail.text.is_empty() { "(erased)".to_owned() } else { detail.text.clone() }),
                    )
                    .child(caption(
                        if detail.computer.is_empty() {
                            format!("Shared by {}", detail.shared_by)
                        } else {
                            format!("Shared by {} from {}", detail.shared_by, detail.computer)
                        },
                        cx,
                    ));
                if let Some(status) = status_label(detail) {
                    row = row.child(h_flex().child(pill(status, cx)));
                }
                if let Some(label) = self.groups.space_label(&detail.space) {
                    row = row.child(
                        h_flex().child(
                            pill(label.clone(), cx)
                                .id(("groups-detail-space", index))
                                .aria_label(label)
                                .test_support(),
                        ),
                    );
                }
                if detail.usable() {
                    let mut actions = h_flex().flex_wrap().gap(t.space.small);
                    if !mine {
                        actions = actions.child(
                            design::button(("groups-agree", index), "I agree", cx)
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(move |_, _, _, cx| send(GroupCommand::Agree { detail: id }, cx))),
                        );
                    }
                    if mine || manage {
                        actions = actions
                            .child(
                                design::button(("groups-withdraw", index), "Stop using", cx)
                                    .ghost()
                                    .xsmall()
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        send(GroupCommand::Withdraw { detail: id }, cx);
                                    })),
                            )
                            .child(
                                design::button(("groups-erase", index), "Erase the words", cx)
                                    .ghost()
                                    .xsmall()
                                    .accessibility_label("Erase this detail's words on every computer in the group")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        send(GroupCommand::Erase { detail: id }, cx);
                                    })),
                            );
                    }
                    row = row.child(actions);
                }
                row
            }))
    }
}

/// Wall-clock milliseconds, for "expires in about N minutes" only.
pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

impl ModelsView {
    /// "Ask each other's memories": federated recall. Memory stays on each computer;
    /// a question goes out and a few attributed answers come back. Both directions are
    /// separate choices, and both start off.
    fn render_federated(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let settings = self.groups.federated;
        let switch = |id: &'static str, on: bool, label: &str, cx: &mut Context<Self>| {
            design::button(
                id,
                if on {
                    format!("{label}: on")
                } else {
                    format!("{label}: off")
                },
                cx,
            )
            .small()
            .selected(on)
        };
        let mut section = v_flex()
            .id("groups-federated")
            .test_support()
            .w_full()
            .gap(t.space.compact)
            .child(div().sovra_text(TextRole::Body, cx).child("Ask each other's memories"))
            .child(caption(
                "Your memories stay on your computer. When this is on, a question from someone's helper in your group is searched here, and only a few short answers go back, with your name on them. You can see every question below.",
                cx,
            ))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        switch(
                            "groups-federated-answer",
                            settings.answer,
                            "Let my group's helpers ask my memory",
                            cx,
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            send(
                                GroupCommand::SetFederated(FederatedSettings {
                                    answer: !settings.answer,
                                    ..settings
                                }),
                                cx,
                            );
                        })),
                    )
                    .child(
                        switch(
                            "groups-federated-ask",
                            settings.ask,
                            "Ask my group when I chat",
                            cx,
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            send(
                                GroupCommand::SetFederated(FederatedSettings {
                                    ask: !settings.ask,
                                    ..settings
                                }),
                                cx,
                            );
                        })),
                    ),
            )
            .child(caption(
                "Asking sends your question to the computers in your group that answer.",
                cx,
            ))
            .child(
                Input::new(&self.groups_form.ask)
                    .id("groups-ask-input")
                    .aria_label("Ask your group's memories"),
            )
            .child(
                h_flex().child(
                    design::button("groups-ask", "Ask my group", cx).on_click(cx.listener(
                        |view, _, window, cx| {
                            let question = value(&view.groups_form.ask, cx);
                            if question.is_empty() {
                                return;
                            }
                            view.groups_form
                                .ask
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            send(GroupCommand::AskGroup { question }, cx);
                        },
                    )),
                ),
            );
        if let Some((question, answers)) = &self.groups.answers {
            let mut list = v_flex()
                .id("groups-answers")
                .test_support()
                .w_full()
                .gap(t.space.compact)
                .p(t.space.small)
                .sovra_surface(Surface::Subtle, cx)
                .child(caption(format!("You asked: {question}"), cx));
            if answers.is_empty() {
                list = list.child(caption(
                    "Nobody's computer had an answer. Their computer may be off, or they haven't turned on answering.",
                    cx,
                ));
            }
            for (index, answer) in answers.iter().enumerate() {
                list = list.child(
                    v_flex()
                        .id(("groups-answer", index))
                        .test_support()
                        .min_w_0()
                        .child(
                            div()
                                .whitespace_normal()
                                .sovra_text(TextRole::Body, cx)
                                .child(answer.statement.clone()),
                        )
                        .child(caption(
                            format!("From {}'s memory · {}", answer.from, answer.computer),
                            cx,
                        )),
                );
            }
            section = section.child(list);
        }
        if !self.groups.recent_asks.is_empty() {
            let mut log = v_flex()
                .id("groups-asked-log")
                .test_support()
                .w_full()
                .gap(t.space.compact)
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .child("Questions your group asked"),
                );
            for (index, ask) in self.groups.recent_asks.iter().take(10).enumerate() {
                let answered = match ask.answered {
                    0 => "nothing sent".to_owned(),
                    1 => "1 answer sent".to_owned(),
                    n => format!("{n} answers sent"),
                };
                log = log.child(
                    div()
                        .id(("groups-asked", index))
                        .test_support()
                        .whitespace_normal()
                        .child(caption(
                            format!(
                                "{} ({}): “{}” — {answered}",
                                ask.asked_by, ask.computer, ask.question
                            ),
                            cx,
                        )),
                );
            }
            section = section.child(log);
        }
        section
    }
}

impl ModelsView {
    /// Helpers and schedules people shared: read the words, then use your own copy.
    fn render_templates(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let manage = self.groups.can_manage();
        let my_person = self.groups.my_person();
        let mut section = v_flex()
            .id("groups-templates")
            .test_support()
            .w_full()
            .gap(t.space.compact)
            .child(div().sovra_text(TextRole::Body, cx).child("Shared helpers and schedules"))
            .child(caption(
                "Read what a helper or schedule says before you use it. Using one makes your own copy on this computer; it never brings tools, and nothing runs until you choose.",
                cx,
            ));
        let shown: Vec<_> = self
            .groups
            .templates
            .iter()
            .filter(|shared| shared.is_latest())
            .cloned()
            .collect();
        if shown.is_empty() {
            return section.child(caption(
                "Nothing yet. Share one from the Team or Schedules page.",
                cx,
            ));
        }
        for (index, shared) in shown.into_iter().enumerate() {
            let mine = shared.person.is_some() && shared.person == my_person;
            let id = shared.id;
            let (title, kind) = match &shared.template {
                sovra_core::group::Template::Helper {
                    icon, name, role, ..
                } => (format!("{icon} {name}"), format!("Helper · {role}")),
                sovra_core::group::Template::Schedule { name, cadence, .. } => {
                    (name.clone(), format!("Schedule · {cadence}"))
                }
                sovra_core::group::Template::Skill { name, .. } => {
                    (name.clone(), "Skill".to_owned())
                }
            };
            let mut caption_text = format!("{kind} · shared by {}", shared.shared_by);
            if let Some(label) = self.groups.space_label(&shared.space) {
                caption_text.push_str(" · ");
                caption_text.push_str(&label.to_lowercase());
            }
            let use_button = match shared.template.clone() {
                sovra_core::group::Template::Helper {
                    name,
                    icon,
                    role,
                    tagline,
                    instructions,
                    expertise,
                } => {
                    let member = sovra_core::teams::TeamMember {
                        id: shared.helper_id(),
                        name,
                        icon,
                        role,
                        tagline,
                        instructions,
                        expertise,
                        conductor: false,
                        active: true,
                        builtin: false,
                    };
                    design::button(("groups-template-use", index), "Use this helper", cx)
                        .small()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let _ = cx.global::<RuntimeClient>().0.send(Command::Teams(
                                sovra_core::teams::TeamCommand::SaveMember(member.clone()),
                            ));
                        }))
                }
                sovra_core::group::Template::Schedule {
                    name,
                    cadence,
                    prompt,
                } => design::button(("groups-template-use", index), "Add to my schedules", cx)
                    .small()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let _ = cx.global::<RuntimeClient>().0.send(Command::Schedule(
                            sovra_core::ScheduleCommand::Create {
                                name: name.clone(),
                                cadence: cadence.clone(),
                                prompt: prompt.clone(),
                                send: None,
                            },
                        ));
                    })),
                sovra_core::group::Template::Skill {
                    name,
                    description,
                    instructions,
                } => {
                    let skill = shared_skill(shared.helper_id(), name, description, instructions);
                    design::button(("groups-template-use", index), "Use this skill", cx)
                        .small()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let _ = cx.global::<RuntimeClient>().0.send(Command::Skills(
                                sovra_core::skills::SkillCommand::Save(Box::new(skill.clone())),
                            ));
                        }))
                }
            };
            let mut actions = h_flex().flex_wrap().gap(t.space.small).child(use_button);
            if mine || manage {
                actions = actions.child(
                    design::button(("groups-template-withdraw", index), "Stop sharing", cx)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            send(GroupCommand::Withdraw { detail: id }, cx);
                        })),
                );
            }
            section = section.child(
                v_flex()
                    .id(("groups-template", index))
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.compact)
                    .p(t.space.compact)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(div().sovra_text(TextRole::Body, cx).child(title))
                    .child(caption(caption_text, cx))
                    .child(
                        div()
                            .id(("groups-template-words", index))
                            .test_support()
                            .whitespace_normal()
                            .sovra_text(TextRole::Caption, cx)
                            .child(shared.template.body().to_owned()),
                    )
                    .child(actions),
            );
        }
        section
    }
}

/// A shared skill as this person's own: no tools (they grant those here), off the
/// automatic list until they choose, under a stable name so using it twice updates it.
fn shared_skill(
    name: String,
    title: String,
    description: String,
    instructions: String,
) -> sovra_core::skills::Skill {
    sovra_core::skills::Skill {
        name,
        title,
        description,
        category: "Your skills".into(),
        aliases: Vec::new(),
        kind: sovra_core::skills::SkillKind::Procedure,
        instructions,
        tools: Vec::new(),
        argument_hint: String::new(),
        requires_input: false,
        enabled: true,
        auto_apply: false,
        builtin: false,
        license: None,
        compatibility: None,
        metadata: std::collections::BTreeMap::new(),
    }
}

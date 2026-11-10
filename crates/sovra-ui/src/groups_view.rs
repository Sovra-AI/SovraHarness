//! Settings → Groups: start or join a group, see who and which computers are in it,
//! invite people, and share details everyone's helpers can use. Plain words only:
//! "computer", "group", "share"; the rules live in `sovra_core::group`.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use sovra_core::Command;
use sovra_core::group::{
    DetailStatus, GroupCommand, GroupEvent, PairingStage, Role, SharedDetail, removal_warning,
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
    /// Which person or computer is waiting for "Yes, remove".
    confirm_remove: Option<String>,
    copy_pending: bool,
    copied_link: Option<String>,
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
            confirm_remove: None,
            copy_pending: false,
            copied_link: None,
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
    pub(crate) fn apply_group_event(&mut self, event: GroupEvent, cx: &mut Context<Self>) {
        if let GroupEvent::InviteReady { link, .. } = &event {
            if self.groups_form.copy_pending {
                cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                self.groups_form.copied_link = Some(link.clone());
            }
            self.groups_form.copy_pending = false;
        } else if matches!(
            event,
            GroupEvent::Problem(_) | GroupEvent::LeftGroup | GroupEvent::Loaded { .. }
        ) {
            self.groups_form.copy_pending = false;
            self.groups_form.copied_link = None;
        }
        self.groups.apply(event);
        cx.notify();
    }

    fn create_and_copy_invite(&mut self, command: GroupCommand, cx: &mut Context<Self>) {
        // Clipboard ownership follows this explicit click, never an unsolicited group event.
        self.groups_form.copy_pending = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Group(command))
            .is_ok();
        self.groups_form.copied_link = None;
        cx.notify();
    }

    pub(crate) fn render_groups_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let view = &self.groups;
        let mut section = settings_section(
            "settings-groups",
            "Groups",
            "Share helpers and knowledge with your family or team, computer to computer. Nothing leaves your computers through a Sovra server, and nothing is shared until you choose to share it.",
            cx,
        );
        if let Some(problem) = &view.problem {
            section = section.child(
                div()
                    .id("groups-problem")
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
            .child(Input::new(&form.group_name))
            .child(Input::new(&form.your_name))
            .child(Input::new(&form.computer_name))
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
            .child(Input::new(&form.join_link))
            .child(Input::new(&form.computer_name))
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
            .child(div().sovra_text(TextRole::Body, cx).child("Share an invite link"))
            .child(caption("Create a one-time link, then paste it into a message. Keep Sovra open while the other person joins.", cx));
        if manage {
            invites = invites
                .child(
                    Input::new(&self.groups_form.invite_name)
                        .id("groups-invite-name")
                        .aria_label("Name of the person to invite"),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            design::primary_button(
                                "groups-invite-member",
                                "Create and copy invite link",
                                cx,
                            )
                            .small()
                            .disabled(self.groups_form.copy_pending)
                            .on_click(cx.listener(|view, _, _, cx| {
                                let name = value(&view.groups_form.invite_name, cx);
                                view.create_and_copy_invite(
                                    GroupCommand::InvitePerson {
                                        name,
                                        role: Role::Member,
                                    },
                                    cx,
                                );
                            })),
                        )
                        .child(
                            design::button("groups-invite-admin", "Create and copy admin link", cx)
                                .small()
                                .ghost()
                                .disabled(self.groups_form.copy_pending)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    let name = value(&view.groups_form.invite_name, cx);
                                    view.create_and_copy_invite(
                                        GroupCommand::InvitePerson {
                                            name,
                                            role: Role::Admin,
                                        },
                                        cx,
                                    );
                                })),
                        ),
                );
        }
        invites = invites.child(
            h_flex().child(
                design::button("groups-invite-computer", "Add another of my computers", cx)
                    .small()
                    .ghost()
                    .disabled(self.groups_form.copy_pending)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.create_and_copy_invite(GroupCommand::InviteComputer, cx);
                    })),
            ),
        );
        if self.groups_form.copy_pending {
            invites = invites.child(caption("Creating your invite link…", cx));
        }
        if let Some((link, expires_at)) = self.groups.invite.clone() {
            let mono = cx.theme().mono_font_family.clone();
            let remaining = (expires_at.millis() - now_millis()).max(0);
            let minutes = (remaining + 59_999) / 60_000;
            if self.groups_form.copied_link.as_ref() == Some(&link) {
                invites = invites.child(caption(
                    "Invite link copied. Paste it into a message to share it.",
                    cx,
                ));
            }
            invites = invites
                .child(caption(
                    format!(
                        "Send this link to them. It works once and expires in about {minutes} minutes."
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
                            design::button("groups-copy-link", "Copy invite link", cx)
                                .xsmall()
                                .accessibility_label("Copy group invite link")
                                .disabled(remaining == 0 || self.groups_form.copy_pending)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                                    view.groups_form.copied_link = Some(link.clone());
                                    cx.notify();
                                })),
                        ),
                );
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
        v_flex()
            .id("groups-knowledge")
            .test_support()
            .w_full()
            .gap(t.space.compact)
            .child(div().sovra_text(TextRole::Body, cx).child("Shared with the group"))
            .child(caption(
                "Helpers on every computer in the group can use these, and they say who shared each one.",
                cx,
            ))
            .child(Input::new(&self.groups_form.share))
            .child(
                h_flex().child(
                    design::primary_button("groups-share", "Share with the group", cx).on_click(
                        cx.listener(|view, _, window, cx| {
                            let text = value(&view.groups_form.share, cx);
                            view.groups_form
                                .share
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            send(GroupCommand::Share { text, about: Vec::new() }, cx);
                        }),
                    ),
                ),
            )
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

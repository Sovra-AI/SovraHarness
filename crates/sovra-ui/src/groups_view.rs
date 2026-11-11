//! The Groups page: what a group is, how to start or join one, and — once in a
//! group — one plain page at a time (people, invites, shared notes, helpers, asking,
//! borrowing an AI, departments, history), each saying what it is for and how to use
//! it. Plain words only: "computer", "group", "share". The wording and the rules live
//! in `sovra_core::group` (`guide` for the explanations).

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div,
};
use sovra_core::Command;
use sovra_core::group::{
    BENEFITS, DetailStatus, FederatedSettings, GroupCommand, GroupEvent, GroupPage, HOW_IT_WORKS,
    InviteKind, PRIVACY_PROMISE, PairingStage, Role, SharedDetail, WHAT_IS_A_GROUP,
    removal_warning,
};
use sovra_core::models::ChromeKey;

use crate::RuntimeClient;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::model_connection_editor::field;
use crate::models_view::{ModelsView, caption, pill};

/// Outside a group: which of the two paths the person picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Setup {
    /// "What would you like to do?" — nothing picked yet.
    Choose,
    Start,
    /// Joining with a link someone sent.
    JoinLink,
    /// Joining with six numbers read out in the same room.
    JoinCode,
}

/// The text boxes and choices on the Groups page.
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
    /// Or just this department (takes precedence when set).
    share_department: Option<sovra_core::group::OpHash>,
    department_name: Entity<InputState>,
    /// People ticked for the department being made.
    department_members: std::collections::BTreeSet<sovra_core::group::PersonId>,
    /// Which person or computer is waiting for "Yes, remove".
    confirm_remove: Option<String>,
    /// "Leave this group" waits for a second, explicit "Yes, leave".
    confirm_leave: bool,
    copy_pending: bool,
    copied_link: Option<String>,
    pub(crate) setup: Setup,
    /// The page inside a group; `None` follows [`GroupsView::starting_page`].
    pub(crate) page: Option<GroupPage>,
    /// Sections the person has opened.
    open: std::collections::BTreeSet<Panel>,
}

/// Parts of the page that stay folded until someone asks for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Panel {
    /// "How do groups work?" opened again from inside a group.
    Explain,
    /// "More…" shows the rarely used pages beside the everyday four.
    MorePages,
    /// "More ways to invite": an admin link and adding your own computers.
    MoreInvite,
}

impl GroupsForm {
    fn is_open(&self, panel: Panel) -> bool {
        self.open.contains(&panel)
    }

    fn toggle(&mut self, panel: Panel) {
        if !self.open.remove(&panel) {
            self.open.insert(panel);
        }
    }
}

impl GroupsForm {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<ModelsView>) -> Self {
        let input =
            |placeholder: &'static str, window: &mut Window, cx: &mut Context<ModelsView>| {
                cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
            };
        Self {
            group_name: input("For example: Family", window, cx),
            your_name: input("For example: Mum", window, cx),
            computer_name: input("For example: Mum's laptop", window, cx),
            join_link: input("Paste the link that starts with sovra://join", window, cx),
            invite_name: input("For example: Grandad", window, cx),
            share: input("For example: The boiler service is on 12 March", window, cx),
            ask: input("For example: Who is our dentist?", window, cx),
            join_code: input("For example: 123 456", window, cx),
            share_with: None,
            share_department: None,
            department_name: input("For example: Kids, or Legal", window, cx),
            department_members: std::collections::BTreeSet::new(),
            confirm_remove: None,
            confirm_leave: false,
            copy_pending: false,
            copied_link: None,
            setup: Setup::Choose,
            page: None,
            open: std::collections::BTreeSet::new(),
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
        DetailStatus::Withdrawn => Some("No longer shared"),
        DetailStatus::Redacted => Some("Erased"),
    }
}

/// A short heading inside a page.
fn heading(text: impl Into<SharedString>, cx: &gpui_kit::App) -> gpui_kit::Div {
    div()
        .whitespace_normal()
        .sovra_text(TextRole::Body, cx)
        .child(text.into())
}

/// Plain sentences that should wrap, not truncate.
fn words(text: impl Into<SharedString>, cx: &gpui_kit::App) -> gpui_kit::Div {
    caption(text, cx).w_full().min_w_0().whitespace_normal()
}

/// "1  Do this", "2  Then this": numbered steps anyone can follow.
fn numbered(id: &'static str, steps: &[&'static str], cx: &gpui_kit::App) -> AnyElement {
    let t = design::tokens(cx);
    v_flex()
        .id(id)
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.compact)
        .children(steps.iter().enumerate().map(|(index, step)| {
            h_flex()
                .w_full()
                .min_w_0()
                .items_start()
                .gap(t.space.small)
                .child(pill(format!("{}", index + 1), cx).flex_shrink_0())
                .child(words(*step, cx).flex_1())
        }))
        .into_any_element()
}

/// A setting that is on or off, said in words with one button to change it.
fn switch_row(
    title: &'static str,
    detail: &'static str,
    on: bool,
    button: Button,
    cx: &gpui_kit::App,
) -> gpui_kit::Div {
    let t = design::tokens(cx);
    h_flex()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .items_center()
        .gap(t.space.small)
        .p(t.space.compact)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(t.space.hair)
                .child(heading(title, cx))
                .child(words(detail, cx)),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap(t.space.small)
                .child(pill(if on { "On" } else { "Off" }, cx))
                .child(button.small().selected(on)),
        )
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
        if matches!(
            event,
            GroupEvent::InviteReady { .. } | GroupEvent::ShortCodeReady { .. }
        ) {
            // The person just asked for it; keep the link or the six numbers in view.
            self.groups_form.page = Some(GroupPage::Invite);
        }
        if matches!(event, GroupEvent::LeftGroup) {
            self.groups_form.setup = Setup::Choose;
            self.groups_form.page = None;
            self.groups_form.confirm_leave = false;
        }
        self.groups.apply(event);
        cx.notify();
    }

    /// Opens one page inside the group (from the Skills page or a "Do this" step).
    pub(crate) fn open_group_page(&mut self, page: GroupPage, cx: &mut Context<Self>) {
        self.groups_form.page = Some(page);
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

    fn current_page(&self) -> GroupPage {
        self.groups_form
            .page
            .unwrap_or_else(|| self.groups.starting_page())
    }

    /// The whole Groups tab: a centred, scrolling reading column.
    pub(crate) fn render_groups_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = design::tokens(cx);
        let title = design::chrome(cx, ChromeKey::NavGroups);
        let column = v_flex()
            .id("groups-column")
            .test_support()
            .w_full()
            .max_w(t.layout.reading_width)
            .min_w_0()
            .mx_auto()
            .flex_shrink_0()
            .gap(t.space.medium)
            .child(
                design::page_header(
                    title,
                    "Share with your family or team, simply and privately.",
                    cx,
                )
                .id("groups-page-title")
                .test_support()
                .aria_label(title),
            )
            .child(self.render_groups_body(cx));
        v_flex()
            .id("sovra-groups-view")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .p(t.space.large)
            .child(
                v_flex()
                    .id("groups-scroll")
                    .test_support()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .overflow_y_scroll()
                    .child(column),
            )
            .into_any_element()
    }

    fn render_groups_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = design::tokens(cx);
        let view = &self.groups;
        let mut body = v_flex()
            .id("settings-groups")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.medium);
        if let Some(problem) = &view.problem {
            body = body.child(
                div()
                    .id("groups-problem")
                    .role(gpui_kit::Role::Status)
                    .aria_label(problem.message().to_owned())
                    .test_support()
                    .w_full()
                    .whitespace_normal()
                    .p(t.space.compact)
                    .sovra_surface(Surface::Subtle, cx)
                    .sovra_text(TextRole::Body, cx)
                    .text_color(cx.theme().danger)
                    .child(problem.message().to_owned()),
            );
        }
        if !view.loaded {
            return body
                .child(caption("Getting your group ready…", cx))
                .into_any_element();
        }
        if let Some(check) = self.render_picture_check(cx) {
            body = body.child(check);
        }
        if view.group.is_none() {
            // The choice comes first; the longer explanation follows for anyone unsure.
            return body
                .child(
                    v_flex()
                        .id("groups-what")
                        .test_support()
                        .w_full()
                        .gap(t.space.small)
                        .child(
                            div()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child("What is a group?"),
                        )
                        .child(
                            div()
                                .w_full()
                                .whitespace_normal()
                                .sovra_text(TextRole::Body, cx)
                                .child(WHAT_IS_A_GROUP),
                        ),
                )
                .child(self.render_setup(cx))
                .child(Self::render_explainer("groups-intro", false, cx))
                .into_any_element();
        }
        body = body.child(self.render_group_header(cx));
        if self.groups_form.is_open(Panel::Explain) {
            body = body.child(Self::render_explainer("groups-intro", true, cx));
        }
        if let Some(steps) = self.render_first_steps(cx) {
            body = body.child(steps);
        }
        body.child(self.render_page_tabs(cx))
            .child(self.render_group_page(self.current_page(), cx))
            .into_any_element()
    }

    /// "What is a group?": what it is, what you can do, how it works, why it's safe.
    /// `intro` repeats the one-sentence answer; outside a group that sentence already
    /// sits above the two choices.
    fn render_explainer(
        id: &'static str,
        intro: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        v_flex()
            .id(id)
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.medium)
            .p(t.space.medium)
            .sovra_surface(Surface::Matte, cx)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child(if intro {
                        "What is a group?"
                    } else {
                        "More about groups"
                    }),
            )
            .children(intro.then(|| {
                div()
                    .w_full()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .child(WHAT_IS_A_GROUP)
            }))
            .child(heading("In a group you can:", cx))
            .child(
                v_flex()
                    .id("groups-benefits")
                    .test_support()
                    .w_full()
                    .gap(t.space.compact)
                    .children(BENEFITS.iter().map(|benefit| {
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_start()
                            .gap(t.space.small)
                            .p(t.space.compact)
                            .sovra_surface(Surface::Subtle, cx)
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .sovra_text(TextRole::SectionTitle, cx)
                                    .child(benefit.icon),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(t.space.hair)
                                    .child(heading(benefit.title, cx))
                                    .child(words(benefit.example, cx)),
                            )
                    })),
            )
            .child(heading("How it works", cx))
            .child(numbered("groups-how-it-works", &HOW_IT_WORKS, cx))
            .child(
                div()
                    .id("groups-privacy")
                    .test_support()
                    .w_full()
                    .whitespace_normal()
                    .p(t.space.compact)
                    .sovra_surface(Surface::Subtle, cx)
                    .sovra_text(TextRole::Caption, cx)
                    .child(format!("🔒 {PRIVACY_PROMISE}")),
            )
    }

    /// Outside a group: pick a path, then fill in one short form.
    fn render_setup(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.groups_form.setup {
            Setup::Choose => Self::render_choose(cx).into_any_element(),
            Setup::Start => self.render_start_group(cx).into_any_element(),
            Setup::JoinLink | Setup::JoinCode => self.render_join_group(cx).into_any_element(),
        }
    }

    fn render_choose(cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let choice = |id: &'static str,
                      title: &'static str,
                      detail: &'static str,
                      button: Button,
                      cx: &mut Context<Self>| {
            v_flex()
                .id(id)
                .test_support()
                .flex_1()
                .min_w(t.layout.content_min_width / 2.)
                .gap(t.space.small)
                .p(t.space.medium)
                .sovra_surface(Surface::Matte, cx)
                .child(div().sovra_text(TextRole::SectionTitle, cx).child(title))
                .child(words(detail, cx))
                .child(h_flex().child(button))
        };
        v_flex()
            .id("groups-choose")
            .test_support()
            .w_full()
            .gap(t.space.small)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("What would you like to do?"),
            )
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(choice(
                        "groups-start",
                        "Start a new group",
                        "You're the first one. You'll give the group a name, then invite the others.",
                        design::primary_button("groups-choose-start", "Start a new group", cx)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.groups_form.setup = Setup::Start;
                                cx.notify();
                            })),
                        cx,
                    ))
                    .child(choice(
                        "groups-join",
                        "Join a group",
                        "Someone already started a group and invited you, with a link or six numbers.",
                        design::primary_button("groups-choose-join", "Join a group", cx).on_click(
                            cx.listener(|view, _, _, cx| {
                                view.groups_form.setup = Setup::JoinLink;
                                cx.notify();
                            }),
                        ),
                        cx,
                    )),
            )
    }

    fn back_to_choose(cx: &mut Context<Self>) -> Button {
        design::button("groups-setup-back", "Back", cx)
            .small()
            .accessibility_label("Back to start or join")
            .on_click(cx.listener(|view, _, _, cx| {
                view.groups_form.setup = Setup::Choose;
                cx.notify();
            }))
    }

    fn render_start_group(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let form = &self.groups_form;
        v_flex()
            .id("groups-start-form")
            .test_support()
            .w_full()
            .gap(t.space.medium)
            .p(t.space.medium)
            .sovra_surface(Surface::Matte, cx)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Start a new group"),
            )
            .child(words(
                "Answer three short questions. You can invite people straight after.",
                cx,
            ))
            .child(field(
                "groups-start-name",
                "1. What should the group be called?",
                &form.group_name,
                "Everyone in the group sees this name.",
                cx,
            ))
            .child(field(
                "groups-start-your-name",
                "2. What's your name?",
                &form.your_name,
                "This is how the others will see you.",
                cx,
            ))
            .child(field(
                "groups-start-computer-name",
                "3. What do you call this computer?",
                &form.computer_name,
                "So you can tell your computers apart later.",
                cx,
            ))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
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
                    )
                    .child(Self::back_to_choose(cx)),
            )
    }

    fn render_join_group(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let form = &self.groups_form;
        let by_code = form.setup == Setup::JoinCode;
        let joining = matches!(self.groups.pairing, PairingStage::Connecting);
        let way = |id: &'static str, label: &'static str, setup: Setup, cx: &mut Context<Self>| {
            design::button(id, label, cx)
                .small()
                .selected(form.setup == setup)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.groups_form.setup = setup;
                    cx.notify();
                }))
        };
        let mut join = v_flex()
            .id("groups-join-form")
            .test_support()
            .w_full()
            .gap(t.space.medium)
            .p(t.space.medium)
            .sovra_surface(Surface::Matte, cx)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Join a group"),
            )
            .child(heading("How did they invite you?", cx))
            .child(
                h_flex()
                    .id("groups-join-way")
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(way(
                        "groups-join-way-link",
                        "They sent me a link",
                        Setup::JoinLink,
                        cx,
                    ))
                    .child(way(
                        "groups-join-way-code",
                        "We're in the same room",
                        Setup::JoinCode,
                        cx,
                    )),
            );
        join = if by_code {
            join.child(field(
                "groups-join-code",
                "1. Type the six numbers from their screen",
                &form.join_code,
                "On their computer they open Groups → Invite someone and press They're sitting next to me. Both computers need to be on the same Wi-Fi.",
                cx,
            ))
        } else {
            join.child(field(
                "groups-join-link",
                "1. Paste the invite link",
                &form.join_link,
                "Copy the whole link from their message or email, then paste it here.",
                cx,
            ))
        };
        join.child(field(
            "groups-join-computer-name",
            "2. What do you call this computer?",
            &form.computer_name,
            "So the group can tell your computers apart, like Grandad's tablet.",
            cx,
        ))
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .items_center()
                .child(if by_code {
                    design::primary_button("groups-join-code-button", "Join the group", cx)
                        .on_click(cx.listener(|view, _, _, cx| {
                            let form = &view.groups_form;
                            let command = GroupCommand::JoinWithCode {
                                code: value(&form.join_code, cx),
                                computer_name: value(&form.computer_name, cx),
                            };
                            send(command, cx);
                        }))
                } else {
                    design::primary_button("groups-join-button", "Join the group", cx).on_click(
                        cx.listener(|view, _, _, cx| {
                            let form = &view.groups_form;
                            let command = GroupCommand::Join {
                                link: value(&form.join_link, cx),
                                computer_name: value(&form.computer_name, cx),
                            };
                            send(command, cx);
                        }),
                    )
                })
                .child(Self::back_to_choose(cx))
                .children(joining.then(|| caption("Connecting to the other computer…", cx))),
        )
        .child(words(
            "No invite yet? Ask the person who started the group to open Groups, choose Invite someone, and send you a link.",
            cx,
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
            .sovra_surface(Surface::Matte, cx)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("One last check"),
            )
            .child(words(
                "Look at both screens. Do they show the same four pictures and the same numbers? This makes sure you're joining the right computer.",
                cx,
            ))
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
                        design::primary_button("groups-pictures-match", "Yes, they match", cx)
                            .on_click(cx.listener(|_, _, _, cx| {
                                send(GroupCommand::ConfirmCode { matches: true }, cx);
                            })),
                    )
                    .child(
                        design::button("groups-pictures-differ", "No, they're different", cx)
                            .on_click(cx.listener(|_, _, _, cx| {
                                send(GroupCommand::ConfirmCode { matches: false }, cx);
                            })),
                    ),
            ),
            Some(_) => check.child(caption("Waiting for the other person to check…", cx)),
        };
        Some(check)
    }

    /// The group's name, its size, who you are in it, and "How do groups work?".
    fn render_group_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = design::tokens(cx);
        let view = &self.groups;
        let Some(group) = view.group.as_ref() else {
            return v_flex().id("groups-header").into_any_element();
        };
        let computers: usize = group.people.values().map(|m| m.devices.len()).sum();
        let people = group.people.len();
        let me = view
            .my_person()
            .and_then(|person| group.people.get(&person))
            .map(|member| member.name.clone());
        let you = match (me, view.my_role()) {
            (Some(name), Some(Role::Admin)) => {
                format!(
                    "You're in this group as {name}, an admin: you can invite and remove people."
                )
            }
            (Some(name), _) => format!("You're in this group as {name}."),
            (None, _) => "This computer is in this group.".to_owned(),
        };
        let open = self.groups_form.is_open(Panel::Explain);
        v_flex()
            .id("groups-header")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .p(t.space.medium)
            .sovra_surface(Surface::Matte, cx)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .items_center()
                    .gap(t.space.small)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .sovra_text(TextRole::SectionTitle, cx)
                            .child(format!("🏠 {}", group.name)),
                    )
                    .child(pill(
                        format!(
                            "{people} {} · {computers} {}",
                            if people == 1 { "person" } else { "people" },
                            if computers == 1 {
                                "computer"
                            } else {
                                "computers"
                            },
                        ),
                        cx,
                    )),
            )
            .child(words(you, cx))
            .child(
                h_flex().child(
                    design::button(
                        "groups-explain",
                        if open {
                            "Hide the explanation"
                        } else {
                            "How do groups work?"
                        },
                        cx,
                    )
                    .small()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.groups_form.toggle(Panel::Explain);
                        cx.notify();
                    })),
                ),
            )
            .into_any_element()
    }

    /// "Getting started": what this new group still has to do, each with a button.
    fn render_first_steps(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let steps = self.groups.first_steps();
        if steps.is_empty() {
            return None;
        }
        let t = design::tokens(cx);
        let mut list = v_flex()
            .id("groups-first-steps")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.compact)
            .p(t.space.medium)
            .sovra_surface(Surface::Matte, cx)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Getting started"),
            );
        for (index, step) in steps.into_iter().enumerate() {
            let mut row = h_flex()
                .id(("groups-step", index))
                .test_support()
                .aria_label(format!(
                    "{}: {}",
                    step.title,
                    if step.done { "done" } else { "to do" }
                ))
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap(t.space.small)
                .child(
                    div()
                        .flex_shrink_0()
                        .child(if step.done { "✅" } else { "⬜" }),
                )
                .child(heading(step.title, cx).flex_1().min_w_0());
            if let Some(page) = step.page {
                row = row.child(
                    design::button(("groups-step-do", index), "Do this", cx)
                        .small()
                        .accessibility_label(format!("Do this: {}", step.title))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.open_group_page(page, cx);
                        })),
                );
            }
            list = list.child(row);
        }
        Some(list)
    }

    /// One button per page inside the group; the open one is selected.
    fn render_page_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let current = self.current_page();
        // The rarely used pages stay behind "More" unless one of them is open.
        let more = self.groups_form.is_open(Panel::MorePages) || !current.is_everyday();
        v_flex()
            .w_full()
            .gap(t.space.compact)
            .child(heading("What would you like to do?", cx))
            .child(
                h_flex()
                    .id("groups-pages")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .gap(t.space.small)
                    .children(
                        GroupPage::ALL
                            .into_iter()
                            .filter(|page| more || page.is_everyday())
                            .map(|page| {
                                design::button(
                                    SharedString::from(format!("groups-page-{}", page.id())),
                                    format!("{} {}", page.icon(), page.title()),
                                    cx,
                                )
                                .small()
                                .selected(page == current)
                                .accessibility_label(format!("Open {}", page.title()))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.open_group_page(page, cx);
                                }))
                            }),
                    )
                    .children((current.is_everyday()).then(|| {
                        design::button(
                            "groups-more-pages",
                            if more { "Fewer choices" } else { "More…" },
                            cx,
                        )
                        .small()
                        .ghost()
                        .accessibility_label(if more {
                            "Show fewer choices"
                        } else {
                            "Show more choices: shared helpers, borrow an AI, smaller circles and history"
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.groups_form.toggle(Panel::MorePages);
                            cx.notify();
                        }))
                    })),
            )
    }

    /// The open page: what it's for, how to use it, then its controls.
    fn render_group_page(
        &self,
        page: GroupPage,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let content = match page {
            GroupPage::People => self.render_people(cx).into_any_element(),
            GroupPage::Invite => self.render_invites(cx).into_any_element(),
            GroupPage::Notes => self.render_knowledge(cx).into_any_element(),
            GroupPage::Helpers => self.render_templates(cx).into_any_element(),
            GroupPage::Ask => self.render_federated(cx).into_any_element(),
            GroupPage::BorrowAi => self.render_work(cx).into_any_element(),
            GroupPage::Departments => self.render_departments(cx).into_any_element(),
            GroupPage::History => self.render_audit(cx).into_any_element(),
        };
        v_flex()
            .id(SharedString::from(format!("groups-pane-{}", page.id())))
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.medium)
            .p(t.space.medium)
            .sovra_surface(Surface::Matte, cx)
            .child(div().sovra_text(TextRole::SectionTitle, cx).child(format!(
                "{} {}",
                page.icon(),
                page.title()
            )))
            .child(
                div()
                    .w_full()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .child(page.what()),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap(t.space.compact)
                    .p(t.space.compact)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(heading("How to use it", cx))
                    .child(numbered("groups-pane-how", page.how(), cx)),
            )
            .child(content)
    }

    fn render_people(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut people = v_flex().w_full().min_w_0().gap(t.space.medium);
        if self.groups.show_second_admin_banner() {
            let alone = self
                .groups
                .group
                .as_ref()
                .is_some_and(|group| group.people.len() < 2);
            people = people.child(
                words(
                    if alone {
                        "Tip: once someone else has joined, press Make admin next to their name. A second admin can still look after the group if this computer is lost or broken."
                    } else {
                        "Tip: make someone else an admin too, so the group can still be looked after if this computer is lost or broken. Press Make admin next to their name."
                    },
                    cx,
                )
                .id("groups-second-admin")
                .p(t.space.compact)
                .sovra_surface(Surface::Subtle, cx),
            );
        }
        let leave = if self.groups_form.confirm_leave {
            v_flex()
                .id("groups-leave-confirm")
                .test_support()
                .w_full()
                .gap(t.space.small)
                .p(t.space.compact)
                .sovra_surface(Surface::Subtle, cx)
                .child(words(
                    "Are you sure? This computer will stop getting anything new from the group. What it already has stays here. You can join again later with a new invite.",
                    cx,
                ))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            design::button("groups-leave", "Yes, leave the group", cx)
                                .small()
                                .accessibility_label("Yes, leave this group on this computer")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.groups_form.confirm_leave = false;
                                    send(GroupCommand::Leave, cx);
                                })),
                        )
                        .child(
                            design::button("groups-leave-cancel", "No, stay", cx)
                                .small()
                                .ghost()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.groups_form.confirm_leave = false;
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element()
        } else {
            v_flex()
                .id("groups-leave-row")
                .child(
                    h_flex().child(
                        design::button("groups-leave-ask", "Leave this group", cx)
                            .small()
                            .accessibility_label("Leave this group on this computer")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.groups_form.confirm_leave = true;
                                cx.notify();
                            })),
                    ),
                )
                .into_any_element()
        };
        people.child(self.render_roster(cx)).child(leave)
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
                                    .child(heading(
                                        if mine {
                                            format!("{} (you)", row.name)
                                        } else {
                                            row.name.clone()
                                        },
                                        cx,
                                    ))
                                    .child(caption(row.role_label, cx)),
                            );
                        for (slot, computer) in row.computers.iter().enumerate() {
                            let device = computer.device;
                            let key = format!("computer:{}", device.to_hex());
                            let label = if computer.is_this_computer {
                                format!("💻 {} (this computer)", computer.name)
                            } else {
                                format!(
                                    "💻 {} · {}",
                                    computer.name,
                                    sovra_core::group::presence_label(
                                        computer.last_seen,
                                        sovra_core::Timestamp(now_millis())
                                    )
                                )
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
                                                "Make a member again"
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
                                            "Remove from group",
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
        let more = self.groups_form.is_open(Panel::MoreInvite) || !manage;
        let mut invites = v_flex()
            .id("groups-invites")
            .test_support()
            .w_full()
            .gap(t.space.medium);
        // What the person just asked for goes first, so it is never below the fold.
        if self.groups_form.copy_pending {
            invites = invites.child(caption("Making your invite link…", cx));
        }
        if let Some((code, expires_at)) = self.groups.short_code.clone() {
            let minutes = (expires_at.millis() - now_millis()).max(0) / 60_000;
            invites = invites.child(
                v_flex()
                    .id("groups-short-code-card")
                    .test_support()
                    .w_full()
                    .gap(t.space.small)
                    .p(t.space.medium)
                    .sovra_surface(Surface::Subtle, cx)
                    .border_1()
                    .border_color(cx.theme().primary)
                    .child(heading("🔢 Your six numbers", cx))
                    .child(
                        div()
                            .id("groups-short-code")
                            .test_support()
                            .aria_label(code.clone())
                            .sovra_text(TextRole::PageTitle, cx)
                            .child(code),
                    )
                    .child(numbered(
                        "groups-short-code-steps",
                        &[
                            "On the other computer, open Groups and choose Join a group.",
                            "Choose We're in the same room, and type these six numbers.",
                            "Both screens then show four pictures. If they match, press Yes, they match on both.",
                        ],
                        cx,
                    ))
                    .child(words(
                        format!(
                            "Both computers need to be on the same Wi-Fi. The numbers work for one try, for about {} minutes.",
                            minutes + 1
                        ),
                        cx,
                    )),
            );
        }
        if let Some((link, expires_at)) = self.groups.invite.clone() {
            let mono = cx.theme().mono_font_family.clone();
            let remaining = (expires_at.millis() - now_millis()).max(0);
            let minutes = (remaining + 59_999) / 60_000;
            let mut card = v_flex()
                .id("groups-invite-card")
                .test_support()
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .p(t.space.medium)
                .sovra_surface(Surface::Subtle, cx)
                .border_1()
                .border_color(cx.theme().primary)
                .child(heading("✉️ Your invite link is ready", cx));
            if self.groups_form.copied_link.as_ref() == Some(&link) {
                card = card.child(words(
                    "✅ Copied! Now paste it into an email or message to them.",
                    cx,
                ));
            }
            card = card
                .child(numbered(
                    "groups-invite-steps",
                    &[
                        "Paste the link into an email or a message and send it. (Right-click and choose Paste, or press Ctrl and V.)",
                        "They need Sovra on their computer. Then they open Groups, choose Join a group, and paste the link.",
                        "Keep Sovra open on this computer until they've joined.",
                    ],
                    cx,
                ))
                .child(words(
                    format!("The link works once and stops working in about {minutes} minutes."),
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
                            design::button("groups-copy-link", "Copy the link again", cx)
                                .small()
                                .accessibility_label("Copy group invite link")
                                .disabled(remaining == 0 || self.groups_form.copy_pending)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                                    view.groups_form.copied_link = Some(link.clone());
                                    cx.notify();
                                })),
                        ),
                );
            if !self.groups.invite_qr.is_empty() {
                card = card
                    .child(words(
                        "Or, if they're next to you, they can point their phone or tablet camera at this square.",
                        cx,
                    ))
                    .child(h_flex().child(crate::connections_view::scan_code(
                        "groups-invite-qr",
                        self.groups.invite_qr.clone(),
                        cx,
                    )));
            }
            invites = invites.child(card);
        }
        if manage {
            invites = invites.child(
                v_flex()
                    .w_full()
                    .gap(t.space.small)
                    .p(t.space.compact)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(heading("Invite a person", cx))
                    .child(field(
                        "groups-invite-name",
                        "What's their name?",
                        &self.groups_form.invite_name,
                        "This is how they'll appear in your group.",
                        cx,
                    ))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(t.space.small)
                            .child(
                                design::primary_button(
                                    "groups-invite-member",
                                    "Create invite link",
                                    cx,
                                )
                                .accessibility_label("Create and copy an invite link")
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
                                design::button(
                                    "groups-code-person",
                                    "They're sitting next to me",
                                    cx,
                                )
                                .accessibility_label(
                                    "They're sitting next to me: show six numbers to type on their computer",
                                )
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
                    )
                    .child(
                        div()
                            .id("groups-invite-warning")
                            .test_support()
                            .w_full()
                            .whitespace_normal()
                            .p(t.space.compact)
                            .sovra_surface(Surface::Matte, cx)
                            .sovra_text(TextRole::Body, cx)
                            .text_color(cx.theme().warning)
                            .child("⚠️ Important: leave this computer switched on with Sovra open, and ask them to use the link within 10 minutes. If it stops working, just make a new one."),
                    ),
            );
            invites = invites.child(
                h_flex().child(
                    design::button(
                        "groups-more-invite",
                        if more {
                            "Fewer ways to invite"
                        } else {
                            "More ways to invite…"
                        },
                        cx,
                    )
                    .small()
                    .ghost()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.groups_form.toggle(Panel::MoreInvite);
                        cx.notify();
                    })),
                ),
            );
        } else {
            invites = invites.child(words(
                "Only an admin (someone in charge of the group) can invite new people. Ask them to send an invite. You can still add your own computers below.",
                cx,
            ));
        }
        if !more {
            return invites;
        }
        if manage {
            invites = invites.child(
                v_flex()
                    .w_full()
                    .gap(t.space.small)
                    .p(t.space.compact)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(heading("Invite someone to help look after the group", cx))
                    .child(words(
                        "An admin can invite and remove people, like you. Type their name above, then press this instead.",
                        cx,
                    ))
                    .child(
                        h_flex().child(
                            design::button(
                                "groups-invite-admin",
                                "Create admin invite link",
                                cx,
                            )
                            .small()
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
                    ),
            );
        }
        invites.child(
            v_flex()
                .w_full()
                .gap(t.space.small)
                .p(t.space.compact)
                .sovra_surface(Surface::Subtle, cx)
                .child(heading("Add another of my own computers", cx))
                .child(words(
                    "Use this for your second laptop, desktop or tablet. It joins as you, not as a new person.",
                    cx,
                ))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            design::button(
                                "groups-invite-computer",
                                "Create a link for my other computer",
                                cx,
                            )
                            .small()
                            .disabled(self.groups_form.copy_pending)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.create_and_copy_invite(GroupCommand::InviteComputer, cx);
                            })),
                        )
                        .child(
                            design::button(
                                "groups-code-computer",
                                "Show six numbers for my other computer",
                                cx,
                            )
                            .small()
                            .on_click(cx.listener(|_, _, _, cx| {
                                send(
                                    GroupCommand::ShowShortCode {
                                        kind: InviteKind::OwnDevice,
                                    },
                                    cx,
                                );
                            })),
                        ),
                ),
        )
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
        let confirm = self.groups_form.confirm_remove.clone();
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
            .gap(t.space.medium);
        if can_add {
            let chosen = self.groups_form.share_with;
            let chosen_department = self.groups_form.share_department;
            let mut audience = h_flex()
                .id("groups-share-with")
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("groups-share-everyone", "Everyone", cx)
                        .small()
                        .selected(chosen.is_none() && chosen_department.is_none())
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.groups_form.share_with = None;
                            view.groups_form.share_department = None;
                            cx.notify();
                        })),
                );
            for (index, (person, name)) in self.groups.others().into_iter().enumerate() {
                audience = audience.child(
                    design::button(("groups-share-just", index), format!("Just {name}"), cx)
                        .small()
                        .selected(chosen_department.is_none() && chosen == Some(person))
                        .accessibility_label(format!("Share only with {name}"))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.groups_form.share_with = Some(person);
                            view.groups_form.share_department = None;
                            cx.notify();
                        })),
                );
            }
            for (index, (department, name)) in self.groups.my_departments().into_iter().enumerate()
            {
                audience = audience.child(
                    design::button(
                        ("groups-share-department", index),
                        format!("Just {name}"),
                        cx,
                    )
                    .small()
                    .selected(chosen_department == Some(department))
                    .accessibility_label(format!("Share only with the {name} circle"))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.groups_form.share_department = Some(department);
                        view.groups_form.share_with = None;
                        cx.notify();
                    })),
                );
            }
            section = section.child(
                v_flex()
                    .w_full()
                    .gap(t.space.small)
                    .p(t.space.compact)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(heading("Share a note", cx))
                    .child(
                        Input::new(&self.groups_form.share)
                            .id("groups-share-input")
                            .aria_label("What do you want to share with your group?"),
                    )
                    .child(caption("Who should see it?", cx))
                    .child(audience)
                    .child(h_flex().child(
                        design::primary_button("groups-share", "Share", cx).on_click(cx.listener(
                            |view, _, window, cx| {
                                let text = value(&view.groups_form.share, cx);
                                view.groups_form
                                    .share
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                let command = match view.groups_form.share_department {
                                    Some(department) => GroupCommand::ShareInDepartment {
                                        department,
                                        text,
                                        about: Vec::new(),
                                    },
                                    None => GroupCommand::Share {
                                        text,
                                        about: Vec::new(),
                                        with: view.groups_form.share_with,
                                    },
                                };
                                send(command, cx);
                            },
                        )),
                    )),
            );
        }
        section = section.child(
            div()
                .id("groups-shared-artifacts")
                .test_support()
                .child(words(
                    match artifacts {
                        0 => "Want to share a document, list or picture? Open it on the Artifacts page and press Share with my group.".to_owned(),
                        1 => "1 document, list or picture is shared with the group. You'll find it on the Artifacts page.".to_owned(),
                        n => format!("{n} documents, lists and pictures are shared with the group. You'll find them on the Artifacts page."),
                    },
                    cx,
                )),
        );
        section = section.child(heading("Notes shared so far", cx));
        if self.groups.knowledge.is_empty() {
            section = section.child(words(
                "Nothing shared yet. Be the first: type a note above and press Share.",
                cx,
            ));
        }
        section
            .children(self.groups.knowledge.iter().enumerate().map(|(index, detail)| {
                let id = detail.id;
                let erase_key = format!("erase:{id:?}");
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
                            design::button(("groups-agree", index), "👍 I agree", cx)
                                .small()
                                .accessibility_label("I agree this note is right")
                                .on_click(cx.listener(move |_, _, _, cx| send(GroupCommand::Agree { detail: id }, cx))),
                        );
                    }
                    if mine || manage {
                        actions = actions
                            .child(
                                design::button(("groups-withdraw", index), "Stop sharing with everyone", cx)
                                    .small()
                                    .accessibility_label("Stop sharing this note; Sovra stops using it for everyone")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        send(GroupCommand::Withdraw { detail: id }, cx);
                                    })),
                            )
                            .child(if confirm.as_deref() == Some(erase_key.as_str()) {
                                design::button(
                                    ("groups-erase-yes", index),
                                    "Yes, delete it from every computer",
                                    cx,
                                )
                                .small()
                                .accessibility_label("Yes, delete this note's words from every computer in the group")
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.groups_form.confirm_remove = None;
                                    send(GroupCommand::Erase { detail: id }, cx);
                                }))
                            } else {
                                design::button(("groups-erase", index), "Delete for everyone…", cx)
                                    .small()
                                    .accessibility_label("Delete this note's words from every computer in the group; asks first")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.groups_form.confirm_remove = Some(erase_key.clone());
                                        cx.notify();
                                    }))
                            });
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
    /// "Ask the group": federated recall. Memory stays on each computer; a question
    /// goes out and a few attributed answers come back. Both directions are separate
    /// choices, and both start off.
    fn render_federated(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let settings = self.groups.federated;
        let mut section = v_flex()
            .id("groups-federated")
            .test_support()
            .w_full()
            .gap(t.space.medium)
            .child(
                v_flex()
                    .w_full()
                    .gap(t.space.small)
                    .p(t.space.compact)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(heading("Ask a question", cx))
                    .child(
                        Input::new(&self.groups_form.ask)
                            .id("groups-ask-input")
                            .aria_label("Ask your group's memories"),
                    )
                    .child(h_flex().child(
                        design::primary_button("groups-ask", "Ask my group", cx).on_click(
                            cx.listener(|view, _, window, cx| {
                                let question = value(&view.groups_form.ask, cx);
                                if question.is_empty() {
                                    return;
                                }
                                view.groups_form
                                    .ask
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                send(GroupCommand::AskGroup { question }, cx);
                            }),
                        ),
                    ))
                    .child(words(
                        "Your question is sent to the computers in your group that have answering turned on.",
                        cx,
                    )),
            );
        if let Some((question, answers)) = &self.groups.answers {
            let mut list = v_flex()
                .id("groups-answers")
                .test_support()
                .w_full()
                .gap(t.space.compact)
                .p(t.space.small)
                .sovra_surface(Surface::Subtle, cx)
                .child(heading(format!("You asked: {question}"), cx));
            if answers.is_empty() {
                list = list.child(words(
                    "No answers came back. Their computers may be switched off, or they haven't turned on Answer questions from my group.",
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
        section = section
            .child(heading("Your choices", cx))
            .child(switch_row(
                "Answer questions from my group",
                "When on, people in your group can get answers from what you have told Sovra on this computer. Only a few short answers go back, with your name on them. Everything else stays here.",
                settings.answer,
                design::button(
                    "groups-federated-answer",
                    if settings.answer { "Turn off" } else { "Turn on" },
                    cx,
                )
                .accessibility_label(if settings.answer {
                    "Answer questions from my group: on. Turn off"
                } else {
                    "Answer questions from my group: off. Turn on"
                })
                .on_click(cx.listener(move |_, _, _, cx| {
                    send(
                        GroupCommand::SetFederated(FederatedSettings {
                            answer: !settings.answer,
                            ..settings
                        }),
                        cx,
                    );
                })),
                cx,
            ))
            .child(switch_row(
                "Ask the group while I chat",
                "When on, your helper also asks the group's computers when it doesn't know something you asked in a chat.",
                settings.ask,
                design::button(
                    "groups-federated-ask",
                    if settings.ask { "Turn off" } else { "Turn on" },
                    cx,
                )
                .accessibility_label(if settings.ask {
                    "Ask the group while I chat: on. Turn off"
                } else {
                    "Ask the group while I chat: off. Turn on"
                })
                .on_click(cx.listener(move |_, _, _, cx| {
                    send(
                        GroupCommand::SetFederated(FederatedSettings {
                            ask: !settings.ask,
                            ..settings
                        }),
                        cx,
                    );
                })),
                cx,
            ));
        if !self.groups.recent_asks.is_empty() {
            let mut log = v_flex()
                .id("groups-asked-log")
                .test_support()
                .w_full()
                .gap(t.space.compact)
                .child(heading("Questions your group asked this computer", cx));
            for (index, ask) in self.groups.recent_asks.iter().take(10).enumerate() {
                let answered = match ask.answered {
                    0 => "no answer sent".to_owned(),
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
    /// Departments: who is in each, and for admins, making or changing one.
    fn render_departments(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut section = v_flex()
            .id("groups-departments")
            .test_support()
            .w_full()
            .gap(t.space.compact);
        let Some(group) = self.groups.group.clone() else {
            return section;
        };
        let name_of = |person: &sovra_core::group::PersonId| {
            group
                .people
                .get(person)
                .map_or_else(|| "someone who left".to_owned(), |m| m.name.clone())
        };
        section = section.child(heading("Your circles", cx));
        if group.departments.is_empty() {
            section = section.child(words(
                "None yet. Most families don't need any; they're handy at work.",
                cx,
            ));
        }
        for (index, department) in group.departments.values().enumerate() {
            let members: Vec<String> = department.members.iter().map(&name_of).collect();
            let line = if members.is_empty() {
                format!("⭕ {} — nobody yet", department.name)
            } else {
                format!("⭕ {} — {}", department.name, members.join(", "))
            };
            section = section.child(
                div()
                    .id(("groups-department", index))
                    .test_support()
                    .whitespace_normal()
                    .child(caption(line, cx)),
            );
        }
        if !self.groups.can_manage() {
            return section.child(words("Only an admin can make or change a circle.", cx));
        }
        let ticked = self.groups_form.department_members.clone();
        let mut people = h_flex()
            .id("groups-department-people")
            .flex_wrap()
            .gap(t.space.small);
        for (index, member) in group.people.values().enumerate() {
            let person = member.person;
            people = people.child(
                design::button(("groups-department-person", index), member.name.clone(), cx)
                    .small()
                    .selected(ticked.contains(&person))
                    .accessibility_label(format!("Put {} in this circle", member.name))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        let members = &mut view.groups_form.department_members;
                        if !members.remove(&person) {
                            members.insert(person);
                        }
                        cx.notify();
                    })),
            );
        }
        section.child(
            v_flex()
                .w_full()
                .gap(t.space.small)
                .p(t.space.compact)
                .sovra_surface(Surface::Subtle, cx)
                .child(heading("Make or change a circle", cx))
                .child(field(
                    "groups-department-name",
                    "1. What should the circle be called?",
                    &self.groups_form.department_name,
                    "Typing the name of a circle you already have changes who is in it.",
                    cx,
                ))
                .child(caption("2. Tap the people who belong in it, then press Save circle", cx))
                .child(people)
                .child(h_flex().child(
                    design::primary_button("groups-department-save", "Save circle", cx)
                        .on_click(cx.listener(|view, _, window, cx| {
                            let name = value(&view.groups_form.department_name, cx);
                            let Some(slug) =
                                sovra_core::swarm::department::DepartmentId::from_name(&name)
                            else {
                                return;
                            };
                            let members =
                                std::mem::take(&mut view.groups_form.department_members);
                            view.groups_form
                                .department_name
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            send(GroupCommand::SetDepartment { slug, name, members }, cx);
                            cx.notify();
                        })),
                ))
                .child(words(
                    "Someone taken out of a circle stops getting its new notes. Notes they already had stay on their computer.",
                    cx,
                )),
        )
    }

    /// This computer's signed history of what it shared and gave out. Read-only:
    /// nothing here can change the record.
    fn render_audit(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let summary = &self.groups.audit;
        let mut section = v_flex()
            .id("groups-audit")
            .test_support()
            .w_full()
            .gap(t.space.compact)
            .child(
                div()
                    .id("groups-audit-headline")
                    .test_support()
                    .whitespace_normal()
                    .p(t.space.compact)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(heading(summary.headline(), cx)),
            );
        if let Some(line) = summary.witness_line() {
            section = section.child(
                div()
                    .id("groups-audit-witnesses")
                    .test_support()
                    .whitespace_normal()
                    .child(caption(line, cx)),
            );
        }
        if let Some(line) = summary.fleet_line() {
            section = section.child(
                div()
                    .id("groups-audit-fleet")
                    .test_support()
                    .whitespace_normal()
                    .child(caption(line, cx)),
            );
        }
        for (index, line) in summary.recent.iter().enumerate() {
            section = section.child(
                div()
                    .id(("groups-audit-line", index))
                    .test_support()
                    .whitespace_normal()
                    .child(caption(line.words.clone(), cx)),
            );
        }
        section.child(words(
            "For the curious: only a fingerprint of the words is kept, never the words themselves, and every record is signed by this computer.",
            cx,
        ))
    }

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
            .child(words(
                "Using one makes your own copy on this computer. It can't do anything on your computer until you choose to use it.",
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
            return section.child(words(
                "Nothing shared yet. When someone shares a helper, skill or schedule, it shows up here.",
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
                    .child(heading(title, cx))
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

impl ModelsView {
    /// "Borrow an AI": work handoff. A computer with no AI can ask a free one in the
    /// group; a helping computer answers in words only, without tools.
    fn render_work(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let settings = self.groups.work;
        let mut section = v_flex()
            .id("groups-work")
            .test_support()
            .w_full()
            .gap(t.space.compact)
            .child(heading("Your choices for this computer", cx))
            .child(switch_row(
                "Lend this computer's AI",
                "Turn this on if this computer has an AI set up. Computers in the group without one can then use it. Any AI costs come from this computer's AI account. It never opens your files or what Sovra remembers for someone else.",
                settings.help_others,
                design::button(
                    "groups-work-help",
                    if settings.help_others { "Turn off" } else { "Turn on" },
                    cx,
                )
                .accessibility_label(if settings.help_others {
                    "Lend this computer's AI: on. Turn off"
                } else {
                    "Lend this computer's AI: off. Turn on"
                })
                .on_click(cx.listener(move |_, _, _, cx| {
                    send(
                        GroupCommand::SetWork(sovra_core::group::WorkSettings {
                            help_others: !settings.help_others,
                            ..settings
                        }),
                        cx,
                    );
                })),
                cx,
            ))
            .child(switch_row(
                "Borrow an AI when this computer has none",
                "Turn this on if this computer has no AI of its own. Your chats are then answered by another computer in the group that is switched on and lending its AI.",
                settings.use_group,
                design::button(
                    "groups-work-use",
                    if settings.use_group { "Turn off" } else { "Turn on" },
                    cx,
                )
                .accessibility_label(if settings.use_group {
                    "Borrow an AI when this computer has none: on. Turn off"
                } else {
                    "Borrow an AI when this computer has none: off. Turn on"
                })
                .on_click(cx.listener(move |_, _, _, cx| {
                    send(
                        GroupCommand::SetWork(sovra_core::group::WorkSettings {
                            use_group: !settings.use_group,
                            ..settings
                        }),
                        cx,
                    );
                })),
                cx,
            ));
        if !self.groups.recent_work.is_empty() {
            let mut log = v_flex()
                .id("groups-work-log")
                .test_support()
                .w_full()
                .gap(t.space.compact)
                .child(heading("Chats this computer answered for the group", cx));
            for (index, job) in self.groups.recent_work.iter().take(10).enumerate() {
                let line = format!(
                    "{} ({}) · {}",
                    job.asked_by,
                    job.computer,
                    if job.answered {
                        "answered"
                    } else {
                        "could not answer"
                    }
                );
                log = log.child(
                    div()
                        .id(("groups-work-job", index))
                        .aria_label(line.clone())
                        .test_support()
                        .child(caption(line, cx)),
                );
            }
            section = section.child(log);
        }
        section
    }
}

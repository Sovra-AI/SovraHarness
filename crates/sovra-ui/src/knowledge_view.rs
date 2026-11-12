//! Knowledge tab: saved memory controls, file knowledge, and an illustrative explanation.
//!
//! The picture comes from [`sovra_core::memory_explanation`]. Drawing it does
//! not open a memory store or do any other IO.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _,
    button::ButtonVariants as _, checkbox::Checkbox, h_flex, input::Input, v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::connections::{ChannelKind, ConnectionCommand};
use sovra_core::people::Reach;
use sovra_core::{
    BeliefPicture, BeliefStanding, Command, MemoryExplanation, StoryStep, memory_explanation,
};

use crate::RuntimeClient;

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum KnowledgeSection {
    Files,
    #[default]
    Memories,
    People,
    Help,
}

impl Shell {
    pub(crate) fn select_knowledge_section(
        &mut self,
        section: KnowledgeSection,
        cx: &mut Context<Self>,
    ) {
        self.knowledge_section = section;
        if section == KnowledgeSection::Memories {
            self.memory_view
                .update(cx, |view, cx| view.refresh_captured(true, cx));
        }
        self.knowledge_scroll.set_offset(gpui_kit::Point::default());
        cx.notify();
    }

    /// Opens Knowledge on the people list. Tests use this when the sidebar rail covers the tab.
    pub fn open_knowledge_people(&mut self, cx: &mut Context<Self>) {
        self.select_tab(sovra_core::NavTab::Knowledge, cx);
        self.select_knowledge_section(KnowledgeSection::People, cx);
    }
}

fn send_people(command: ConnectionCommand, cx: &mut Context<Shell>) {
    let _ = cx
        .global::<RuntimeClient>()
        .0
        .send(Command::Connections(command));
    cx.notify();
}

pub(crate) fn render_knowledge_view(
    shell: &Shell,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    if shell.memory_view.read(cx).source_only() {
        return v_flex()
            .id("sovra-memory-source-view").test_support()
            .flex_1().min_w_0().min_h_0().h_full().items_center().overflow_y_scroll()
            .p(t.space.medium)
            .child(v_flex().id("knowledge-column").test_support().w_full().min_w_0().flex_shrink_0().max_w(t.layout.reading_width).mx_auto().gap(t.space.small)
                .child(design::button("memory-back-chat", "Back to conversation", cx)
                    .on_click(cx.listener(|shell, _, _, cx| shell.select_tab(sovra_core::NavTab::Chat, cx))))
                .child(div().sovra_text(TextRole::Caption, cx).child("The saved record behind this reference. A reference alone does not prove the reply is correct."))
                .child(shell.memory_view.clone()))
            .into_any_element();
    }
    // The memory section is a card page with a map, so it gets the wider page measure;
    // the other sections are prose and forms.
    let measure = if shell.knowledge_section == KnowledgeSection::Memories {
        t.layout.page_width
    } else {
        t.layout.reading_width
    };
    let column = v_flex()
        .id("knowledge-column")
        .test_support()
        .flex_shrink_0()
        .w_full()
        .min_w_0()
        .max_w(measure)
        .mx_auto()
        .gap(t.space.medium);
    let column = match shell.knowledge_section {
        KnowledgeSection::Files => column.child(crate::file_index_view::render(shell, cx)),
        KnowledgeSection::Memories => {
            let column = if shell.memory_view.read(cx).reading_source() {
                column
            } else {
                column.child(render_memory_status(shell, cx))
            };
            column.child(shell.memory_view.clone())
        }
        KnowledgeSection::People => column.child(render_people(shell, cx)),
        KnowledgeSection::Help => {
            let story = memory_explanation();
            let column = column
                .child(render_intro(&story, cx))
                .child(render_story(&story, cx))
                .child(render_about(&story, cx))
                .child(render_search(&story, cx))
                .child(render_times_toggle(shell.knowledge_times_open, cx));
            if shell.knowledge_times_open {
                column.child(render_times(&story, cx))
            } else {
                column
            }
        }
    };
    let mut navigation = h_flex()
        .id("knowledge-navigation")
        .test_support()
        .w_full()
        .min_w_0()
        .max_w(measure)
        .mx_auto()
        .flex_wrap()
        .gap(t.space.small);
    for (id, label, section) in [
        (
            "knowledge-memories",
            "About you",
            KnowledgeSection::Memories,
        ),
        ("knowledge-people", "People", KnowledgeSection::People),
        ("knowledge-files", "Files", KnowledgeSection::Files),
        ("knowledge-help", "Guide", KnowledgeSection::Help),
    ] {
        navigation = navigation.child(
            design::button(id, label, cx)
                .selected(shell.knowledge_section == section)
                .on_click(
                    cx.listener(move |shell, _, _, cx| shell.select_knowledge_section(section, cx)),
                ),
        );
    }
    v_flex()
        .id("knowledge-page")
        .test_support()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .child(
            v_flex()
                .w_full()
                .flex_shrink_0()
                .items_center()
                .px(t.space.medium)
                .py(t.space.small)
                .child(navigation),
        )
        .child(
            v_flex()
                .id("sovra-knowledge-view")
                .test_support()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .w_full()
                .items_center()
                .overflow_y_scroll()
                .track_scroll(&shell.knowledge_scroll)
                .px(t.space.medium)
                .pt(t.space.small)
                .pb(t.space.medium)
                .child(column),
        )
        .into_any_element()
}

const PEOPLE_INTRO: &str = "Everyone you can message, in one place. The same person can be on a phone number, WhatsApp, Telegram, and email. WhatsApp groups you are in are listed separately and are never answered automatically.";
const PEOPLE_SAFETY: &str = "Nothing is sent until you press Send.";
const PEOPLE_JOIN: &str = "A shared first name does not make two people the same. Join two cards only when you know they are one person.";
const PEOPLE_TELEGRAM: &str =
    "Telegram only knows chats that have already written to your bot, or a chat you add here.";
const GROUPS_INTRO: &str = "WhatsApp groups you are in. Sovra does not answer group chats automatically. Message this group opens a review; nothing is sent until you press Send.";

/// Knowledge → People: a search box, a compact list of rows, and one open row
/// with the message box and actions. Hundreds of synced contacts stay readable
/// because only one person is ever expanded and the list is paged.
fn render_people(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let book = shell.workspace().connections.people.clone();
    let groups = shell.workspace().connections.groups.clone();
    let query = shell.people_search_input.read(cx).value().trim().to_owned();
    let matches = book.search(&query);
    let total = book.people.len();
    let mut page = v_flex()
        .id("people-list")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .child(
            div()
                .id("people-intro")
                .test_support()
                .aria_label(format!("People. {PEOPLE_INTRO}"))
                .child(design::page_header("People", PEOPLE_INTRO, cx)),
        );
    if total == 0 {
        let note = book
            .note
            .clone()
            .unwrap_or_else(|| sovra_core::EMPTY_PEOPLE_NOTE.to_owned());
        page = page.child(
            div()
                .id("people-empty")
                .test_support()
                .aria_label(note.clone())
                .sovra_text(TextRole::Body, cx)
                .child(note),
        );
    } else {
        page = page.child(
            Input::new(&shell.people_search_input)
                .id("people-search")
                .prefix(Icon::new(IconName::Search).small())
                .cleanable(true)
                .aria_label("Find someone by name, number or app"),
        );
        let count = match (query.is_empty(), matches.len()) {
            (true, _) => format!("{total} {}", plural(total, "person", "people")),
            (false, 0) => format!(
                "No one matches “{query}”. Try part of a name, or a few digits of a number."
            ),
            (false, found) => format!("{found} of {total} match “{query}”"),
        };
        page = page.child(
            div()
                .id("people-count")
                .test_support()
                .role(gpui_kit::Role::Status)
                .aria_label(count.clone())
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(count),
        );
        if !matches.is_empty() {
            let shown = matches.len().min(shell.people_shown);
            let mut rows = v_flex()
                .id("people-rows")
                .test_support()
                .w_full()
                .min_w_0()
                .sovra_surface(Surface::Matte, cx)
                .p(t.space.compact)
                .gap(t.space.hair);
            for (index, person) in matches.iter().take(shown).enumerate() {
                rows = rows.child(render_person(shell, person, index, &book, cx));
            }
            page = page.child(rows);
            let more = matches.len() - shown;
            if more > 0 {
                let label = format!("Show {} more", more.min(sovra_core::PEOPLE_PAGE));
                page = page.child(
                    design::button("people-show-more", label.clone(), cx)
                        .accessibility_label(label)
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.people_shown += sovra_core::PEOPLE_PAGE;
                            cx.notify();
                        })),
                );
            }
        }
    }
    let lowered = query.to_lowercase();
    let groups: Vec<(usize, &sovra_core::SyncedGroup)> = groups
        .iter()
        .filter(|group| lowered.is_empty() || group.subject.to_lowercase().contains(&lowered))
        .enumerate()
        .collect();
    if !groups.is_empty() {
        page = page
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("WhatsApp groups"),
            )
            .child(
                div()
                    .id("whatsapp-groups-intro")
                    .test_support()
                    .aria_label(GROUPS_INTRO)
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child(GROUPS_INTRO),
            );
        let mut rows = v_flex()
            .id("whatsapp-group-rows")
            .test_support()
            .w_full()
            .min_w_0()
            .sovra_surface(Surface::Matte, cx)
            .p(t.space.compact)
            .gap(t.space.hair);
        for (index, group) in groups {
            rows = rows.child(render_group(shell, group, index, cx));
        }
        page = page.child(rows);
    }
    page
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

/// The round badge at the start of a row: initials for a person, a group icon.
fn badge(text: Option<String>, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    let primary = theme.primary;
    let badge = h_flex()
        .flex_shrink_0()
        .size(t.layout.profile_avatar)
        .rounded(t.radius.pill)
        .items_center()
        .justify_center()
        .bg(t.subtle)
        .border_1()
        .border_color(t.material_border)
        .text_color(primary);
    match text {
        Some(text) => badge
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(primary)
                    .child(text),
            )
            .into_any_element(),
        None => badge
            .child(Icon::new(IconName::Users).small().text_color(primary))
            .into_any_element(),
    }
}

/// One compact list row: badge, name and how to reach them. Clicking opens it.
fn row_shell(
    id: impl Into<gpui_kit::ElementId>,
    open: bool,
    cx: &Context<Shell>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let t = design::tokens(cx);
    let hover = t.subtle;
    let row = v_flex()
        .id(id)
        .w_full()
        .min_w_0()
        .px(t.space.small)
        .py(t.space.compact)
        .gap(t.space.small)
        .rounded(t.radius.card)
        .cursor_pointer();
    if open {
        row.sovra_surface(Surface::Elevated, cx)
    } else {
        row.hover(move |style| style.bg(hover))
    }
}

fn row_heading(
    badge_text: Option<String>,
    title: String,
    caption: (gpui_kit::ElementId, String),
    open: bool,
    trailing: Option<gpui_kit::AnyElement>,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let (caption_id, caption) = caption;
    h_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .items_center()
        .child(badge(badge_text, cx))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .min_w_0()
                        .sovra_text(TextRole::Body, cx)
                        .text_ellipsis()
                        .child(title),
                )
                .child(
                    div()
                        .id(caption_id)
                        .test_support()
                        .min_w_0()
                        .aria_label(caption.clone())
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .text_ellipsis()
                        .child(caption),
                ),
        )
        .children(trailing)
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .small()
            .text_color(t.text_secondary),
        )
}

/// The message box and its safety note, shared by an open person or group.
fn message_box(shell: &Shell, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.compact)
        .child(
            Input::new(&shell.people_message_input)
                .id("people-message")
                .aria_label("Write a message"),
        )
        .child(
            div()
                .id("people-safety")
                .test_support()
                .aria_label(PEOPLE_SAFETY)
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(PEOPLE_SAFETY),
        )
}

fn toggle_open(shell: &mut Shell, id: &str, cx: &mut Context<Shell>) {
    if shell.people_selected.as_deref() == Some(id) {
        shell.people_selected = None;
    } else {
        shell.people_selected = Some(id.to_owned());
    }
    cx.notify();
}

fn render_group(
    shell: &Shell,
    group: &sovra_core::SyncedGroup,
    index: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let title = if group.subject.is_empty() {
        "WhatsApp group".to_owned()
    } else {
        group.subject.clone()
    };
    let open = shell.people_selected.as_deref() == Some(group.id.as_str());
    let toggle_id = group.id.clone();
    let mut row = row_shell(("whatsapp-group-card", index), open, cx)
        .aria_label(format!("{title}. WhatsApp group."))
        .on_click(cx.listener(move |shell, _, _, cx| toggle_open(shell, &toggle_id, cx)))
        .child(row_heading(
            None,
            title,
            (
                ("whatsapp-group-kind", index).into(),
                "WhatsApp group".to_owned(),
            ),
            open,
            None,
            cx,
        ));
    if open {
        let group_id = group.id.clone();
        row = row.child(
            v_flex()
                .id(("whatsapp-group-open", index))
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                // Typing in the open row must not close it.
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(message_box(shell, cx))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            design::primary_button(
                                ("whatsapp-group-prepare", index),
                                "Message this group",
                                cx,
                            )
                            .accessibility_label("Message this group")
                            .on_click(cx.listener(
                                move |shell, _, _, cx| {
                                    let text =
                                        shell.people_message_input.read(cx).value().to_string();
                                    shell.people_open_review = Some(text.clone());
                                    let session_id = shell.workspace().session_id.clone();
                                    send_people(
                                        ConnectionCommand::PrepareMessage {
                                            session_id,
                                            channel: ChannelKind::WhatsApp,
                                            recipient: group_id.clone(),
                                            text,
                                        },
                                        cx,
                                    );
                                },
                            )),
                        ),
                )
                .child(
                    div()
                        .id(("whatsapp-group-id", index))
                        .test_support()
                        .w_full()
                        .min_w_0()
                        .whitespace_normal()
                        .aria_label(group.id.clone())
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(format!("Group address {}", group.id)),
                ),
        );
    }
    row.test_support()
}

fn render_person(
    shell: &Shell,
    person: &sovra_core::Person,
    index: usize,
    book: &sovra_core::PeopleBook,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let connected = reach_summary(person);
    let open = shell.people_selected.as_deref() == Some(person.id.as_str());
    let toggle_id = person.id.clone();
    let mut row = row_shell(("person-card", index), open, cx)
        .aria_label(format!("{}. {connected}", person.title()))
        .on_click(cx.listener(move |shell, _, _, cx| toggle_open(shell, &toggle_id, cx)))
        .child(row_heading(
            Some(person.initials()),
            person.title(),
            (("person-channels", index).into(), connected),
            open,
            None,
            cx,
        ));
    if !open {
        return row.test_support();
    }
    let mut actions = h_flex().w_full().min_w_0().flex_wrap().gap(t.space.small);
    if person.channel_labels().contains(&"WhatsApp") {
        let person_id = person.id.clone();
        actions = actions.child(
            design::primary_button(("person-prepare", index), "Message on WhatsApp", cx)
                .accessibility_label("Message on WhatsApp")
                .on_click(cx.listener(move |shell, _, _, cx| {
                    let text = shell.people_message_input.read(cx).value().to_string();
                    shell.people_open_review = Some(text.clone());
                    let session_id = shell.workspace().session_id.clone();
                    send_people(
                        ConnectionCommand::PrepareMessage {
                            session_id,
                            channel: ChannelKind::WhatsApp,
                            recipient: format!("person:{person_id}"),
                            text,
                        },
                        cx,
                    );
                })),
        );
    }
    row = row.child(
        v_flex()
            .id(("person-open", index))
            .w_full()
            .min_w_0()
            .gap(t.space.medium)
            // Typing in the open row must not close it.
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(message_box(shell, cx))
            .child(actions)
            .child(render_reach_editor(shell, person, cx))
            .children(render_join(person, book, cx))
            .child(
                div()
                    .id("people-join-hint")
                    .test_support()
                    .aria_label(PEOPLE_JOIN)
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child(PEOPLE_JOIN),
            ),
    );
    row.test_support()
}

/// Inside the open row: cards whose names share a word with this person, each
/// with one button to join them. Hidden when there is no likely duplicate.
fn render_join(
    person: &sovra_core::Person,
    book: &sovra_core::PeopleBook,
    cx: &mut Context<Shell>,
) -> Option<impl IntoElement + use<>> {
    let t = design::tokens(cx);
    let candidates = book.possible_duplicates(&person.id, 5);
    if candidates.is_empty() {
        return None;
    }
    let mut buttons = h_flex().w_full().min_w_0().flex_wrap().gap(t.space.small);
    for (index, other) in candidates.into_iter().enumerate() {
        let keep = person.id.clone();
        let merge = other.id.clone();
        let label = format!("Same person as {}", other.title());
        buttons = buttons.child(
            design::button(("person-link", index), label.clone(), cx)
                .small()
                .accessibility_label(label)
                .on_click(cx.listener(move |_shell, _, _, cx| {
                    send_people(
                        ConnectionCommand::LinkPeople {
                            keep: keep.clone(),
                            merge: merge.clone(),
                        },
                        cx,
                    );
                })),
        );
    }
    Some(
        v_flex()
            .id("people-join")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.compact)
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .child(format!("Is {} also one of these cards?", person.title())),
            )
            .child(buttons),
    )
}

/// "More ways to reach …", inside the open person's row.
fn render_reach_editor(
    shell: &Shell,
    person: &sovra_core::Person,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let prompt = format!("Add a way to reach {}.", person.title());
    let field = |input, id: &'static str, label: &'static str, button| {
        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .gap(t.space.small)
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(input).id(id).aria_label(label)),
            )
            .child(button)
    };
    let email = design::button("people-add-email", "Add email", cx).on_click(cx.listener(
        |shell, _, _, cx| {
            let Some(person_id) = shell.people_selected.clone() else {
                return;
            };
            let value = shell.people_email_input.read(cx).value().to_string();
            send_people(
                ConnectionCommand::AttachHandle {
                    person_id,
                    reach: Reach::Email,
                    value,
                },
                cx,
            );
        },
    ));
    let telegram = design::button("people-add-telegram", "Add Telegram chat", cx).on_click(
        cx.listener(|shell, _, _, cx| {
            let Some(person_id) = shell.people_selected.clone() else {
                return;
            };
            let value = shell.people_telegram_input.read(cx).value().to_string();
            send_people(
                ConnectionCommand::AttachHandle {
                    person_id,
                    reach: Reach::Telegram,
                    value,
                },
                cx,
            );
        }),
    );
    v_flex()
        .id("people-reach")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.compact)
        .child(
            div()
                .id("people-reach-prompt")
                .test_support()
                .aria_label(prompt.clone())
                .sovra_text(TextRole::Caption, cx)
                .child(prompt),
        )
        .child(field(
            &shell.people_email_input,
            "people-email",
            "Email address",
            email.into_any_element(),
        ))
        .child(field(
            &shell.people_telegram_input,
            "people-telegram",
            "Telegram chat you already know",
            telegram.into_any_element(),
        ))
        .child(
            div()
                .id("people-telegram-note")
                .test_support()
                .aria_label(PEOPLE_TELEGRAM)
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(PEOPLE_TELEGRAM),
        )
}

/// `WhatsApp +447700900111 · Email sam@school.example`, most direct first.
fn reach_summary(person: &sovra_core::Person) -> String {
    let parts: Vec<String> = [Reach::Phone, Reach::WhatsApp, Reach::Telegram, Reach::Email]
        .into_iter()
        .flat_map(|reach| {
            person
                .handles
                .iter()
                .filter(move |handle| handle.reach == reach)
                .map(move |handle| format!("{} {}", reach.label(), handle.label))
        })
        .collect();
    if parts.is_empty() {
        "No phone, WhatsApp, Telegram, or email saved yet".to_owned()
    } else {
        parts.join(" · ")
    }
}

fn render_memory_status(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex().id("conversation-memory-status").test_support()
                .aria_label(format!("Conversation memory. {} Messages may be sent to your selected AI service. Saved details are stored on this computer.", shell.workspace().memory_status.label()))
                .w_full().min_w_0().sovra_surface(Surface::Matte, cx)
                .p(t.space.medium).gap(t.space.small)
                .child(design::page_header("What I know about you", "Your preferences, important people, work and plans — with sources you can check.", cx))
                .child(div().id("memory-working-status").role(gpui_kit::Role::Status)
                    .aria_label(shell.workspace().memory_status.label().to_owned())
                    .sovra_text(TextRole::Body, cx).child(shell.workspace().memory_status.label().to_owned()))
                .child(div().sovra_text(TextRole::Caption, cx).child("Sovra learns useful details from what you share in chat using your selected AI service. Saved details stay on this computer. Pause below to stop automatic learning and recall; your saved history stays."))
                .child(Checkbox::new("knowledge-memory-toggle")
                    .cursor_pointer()
                    .disabled(!shell.models().read(cx).state.loaded)
                    .label("Remember across conversations")
                    .checked(shell.models().read(cx).state.preferences.agent_settings.memory_enabled)
                    .on_click(cx.listener(|shell, checked, _, cx| {
                        shell.models().update(cx, |models, cx| {
                            let mut settings = models.state.preferences.agent_settings.clone();
                            settings.memory_enabled = *checked;
                            models.save_agent_settings(settings, cx);
                        });
                        cx.notify();
                    })))
}

fn render_intro(story: &MemoryExplanation, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let example_notice = "Example only — these people and details are not your saved memories. Open About you to see what Sovra has actually saved.";
    v_flex()
        .id("knowledge-intro")
        .test_support()
        .aria_label(format!("{} {}", story.title, story.lead))
        .child(design::page_header(
            "Knowledge",
            "How Sovra remembers, learns, and keeps context.",
            cx,
        ))
        .child(
            div()
                .mt(t.space.medium)
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(story.lead),
        )
        .child(
            div()
                .id("knowledge-example-notice")
                .test_support()
                .aria_label(example_notice)
                .mt(t.space.small)
                .min_w_0()
                .sovra_text(TextRole::Caption, cx)
                .child(example_notice),
        )
}

fn render_story(story: &MemoryExplanation, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("knowledge-story")
        .test_support()
        .sovra_surface(Surface::Matte, cx)
        .p(t.space.large)
        .gap(t.space.large)
        .children(
            story
                .steps
                .iter()
                .enumerate()
                .map(|(index, step)| render_step(index, step, cx)),
        )
}

fn render_step(
    index: usize,
    step: &StoryStep,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let number = index + 1;
    h_flex()
        .id(("knowledge-step", index))
        .test_support()
        .aria_label(format!("Step {number}. {} {}", step.heading, step.body))
        .w_full()
        .items_start()
        .gap(t.space.medium)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(t.layout.logo_small)
                .flex_shrink_0()
                .rounded(t.radius.pill)
                .bg(t.nav_active_background)
                .sovra_text(TextRole::Caption, cx)
                .text_color(cx.theme().link)
                .child(number.to_string()),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(t.space.compact)
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(step.heading),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .text_color(t.text_secondary)
                        .child(step.body),
                ),
        )
}

fn render_about(story: &MemoryExplanation, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut label = String::from("Who or what this is about. ");
    for belief in story.beliefs {
        label.push_str(&belief_sentence(belief));
        label.push(' ');
    }
    v_flex()
        .id("knowledge-about")
        .test_support()
        .aria_label(label)
        .w_full()
        .sovra_surface(Surface::Subtle, cx)
        .p(t.space.medium)
        .gap(t.space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("Who or what this is about"),
        )
        .children(story.beliefs.iter().map(|belief| render_belief(belief, cx)))
}

fn render_belief(belief: &BeliefPicture, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let surface = match belief.standing {
        BeliefStanding::Current => Surface::Matte,
        BeliefStanding::History => Surface::Subtle,
    };
    v_flex()
        .w_full()
        .sovra_surface(surface, cx)
        .p(t.space.small)
        .gap(t.space.compact)
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(cx.theme().link)
                .child(belief.standing.label()),
        )
        .child(div().sovra_text(TextRole::Body, cx).child(belief.statement))
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(format!("About {}", belief.about.join(" and "))),
        )
}

fn belief_sentence(belief: &BeliefPicture) -> String {
    format!(
        "{} {} About {}.",
        belief.standing.label(),
        belief.statement,
        belief.about.join(" and ")
    )
}

fn render_search(story: &MemoryExplanation, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("knowledge-search")
        .test_support()
        .aria_label(format!("{} {}", story.search.heading, story.search.summary))
        .w_full()
        .sovra_surface(Surface::Matte, cx)
        .p(t.space.medium)
        .gap(t.space.compact)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(story.search.heading),
        )
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(story.search.summary),
        )
}

fn render_times_toggle(times_open: bool, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let label = if times_open {
        "Hide the times Sovra keeps"
    } else {
        "Show the times Sovra keeps"
    };
    design::button("knowledge-times-toggle", label, cx)
        .ghost()
        .on_click(cx.listener(|shell, _, _, cx| shell.toggle_knowledge_times(cx)))
}

fn render_times(story: &MemoryExplanation, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut label = String::from("The times Sovra keeps. ");
    for note in story.times {
        label.push_str(note.heading);
        label.push_str(". ");
        label.push_str(note.body);
        label.push(' ');
    }
    v_flex()
        .id("knowledge-times")
        .test_support()
        .aria_label(label)
        .w_full()
        .sovra_surface(Surface::Subtle, cx)
        .p(t.space.medium)
        .gap(t.space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("The times Sovra keeps"),
        )
        .children(story.times.iter().enumerate().map(|(index, note)| {
            v_flex()
                .id(("knowledge-time", index))
                .test_support()
                .aria_label(format!("{} {}", note.heading, note.body))
                .gap(t.space.compact)
                .child(div().sovra_text(TextRole::Body, cx).child(note.heading))
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .text_color(t.text_secondary)
                        .child(note.body),
                )
        }))
}

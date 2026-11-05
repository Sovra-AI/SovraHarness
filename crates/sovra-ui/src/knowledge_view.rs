//! Knowledge tab: saved memory controls, file knowledge, and an illustrative explanation.
//!
//! The picture comes from [`sovra_core::memory_explanation`]. Drawing it does
//! not open a memory store or do any other IO.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, button::ButtonVariants as _,
    checkbox::Checkbox, h_flex, input::Input, v_flex,
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

const PEOPLE_INTRO: &str = "Everyone you can message, in one place. The same person can be on a phone number, WhatsApp, Telegram, and email.";
const PEOPLE_SAFETY: &str = "Nothing is sent until you press Send.";
const PEOPLE_JOIN: &str = "A shared first name does not make two people the same. Join two cards only when you know they are one person.";
const PEOPLE_TELEGRAM: &str =
    "Telegram only knows chats that have already written to your bot, or a chat you add here.";

fn render_people(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let book = shell.workspace().connections.people.clone();
    let mut list = v_flex()
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
        )
        .child(
            div()
                .id("people-join-hint")
                .test_support()
                .aria_label(PEOPLE_JOIN)
                .sovra_text(TextRole::Caption, cx)
                .child(PEOPLE_JOIN),
        )
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
                .child(PEOPLE_SAFETY),
        );
    if book.people.is_empty() {
        let note = book
            .note
            .clone()
            .unwrap_or_else(|| sovra_core::EMPTY_PEOPLE_NOTE.to_owned());
        list = list.child(
            div()
                .id("people-empty")
                .test_support()
                .aria_label(note.clone())
                .sovra_text(TextRole::Body, cx)
                .child(note),
        );
    } else {
        for (index, person) in book.people.iter().enumerate() {
            list = list.child(render_person(shell, person, index, &book, cx));
        }
        list = list.child(render_reach_editor(shell, &book, cx));
    }
    list
}

fn render_person(
    shell: &Shell,
    person: &sovra_core::Person,
    index: usize,
    book: &sovra_core::PeopleBook,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let channels = person.channel_labels();
    let connected = reach_summary(person);
    let summary = format!("{}. {connected}", person.title());
    let selected = shell.people_selected.as_deref() == Some(person.id.as_str());
    let person_id = person.id.clone();
    let mut card = v_flex()
        .id(("person-card", index))
        .test_support()
        .aria_label(summary)
        .w_full()
        .min_w_0()
        .sovra_surface(
            if selected {
                Surface::Elevated
            } else {
                Surface::Matte
            },
            cx,
        )
        .p(t.space.medium)
        .gap(t.space.small)
        .cursor_pointer()
        .on_click(cx.listener(move |shell, _, _, cx| {
            shell.people_selected = Some(person_id.clone());
            cx.notify();
        }))
        .child(
            div()
                .min_w_0()
                .sovra_text(TextRole::Body, cx)
                .child(person.title()),
        )
        .child(
            div()
                .id(("person-channels", index))
                .test_support()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .aria_label(connected.clone())
                .sovra_text(TextRole::Caption, cx)
                .child(connected),
        );
    let mut actions = h_flex().w_full().min_w_0().flex_wrap().gap(t.space.small);
    if channels.contains(&"WhatsApp") {
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
    if let Some(other_id) = shell.people_selected.clone().filter(|id| id != &person.id) {
        let other_title = book
            .people
            .iter()
            .find(|other| other.id == other_id)
            .map_or_else(|| "that person".to_owned(), sovra_core::Person::title);
        let keep = other_id;
        let merge = person.id.clone();
        let label = format!("Same person as {other_title}");
        actions = actions.child(
            design::button(("person-link", index), label.clone(), cx)
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
    card = card.child(actions);
    card
}

fn render_reach_editor(
    shell: &Shell,
    book: &sovra_core::PeopleBook,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let chosen = shell.people_selected.as_ref().and_then(|id| {
        book.people
            .iter()
            .find(|person| person.id == *id)
            .map(sovra_core::Person::title)
    });
    let prompt = chosen.map_or_else(
        || "Choose a person to add an email or a Telegram chat.".to_owned(),
        |name| format!("Add a way to reach {name}."),
    );
    let ready = shell.people_selected.is_some();
    v_flex()
        .id("people-reach")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .child(
            div()
                .id("people-reach-prompt")
                .test_support()
                .aria_label(prompt.clone())
                .sovra_text(TextRole::Body, cx)
                .child(prompt),
        )
        .child(
            div()
                .id("people-telegram-note")
                .test_support()
                .aria_label(PEOPLE_TELEGRAM)
                .sovra_text(TextRole::Caption, cx)
                .child(PEOPLE_TELEGRAM),
        )
        .child(
            Input::new(&shell.people_email_input)
                .id("people-email")
                .aria_label("Email address"),
        )
        .child(
            design::button("people-add-email", "Add email", cx)
                .disabled(!ready)
                .on_click(cx.listener(|shell, _, _, cx| {
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
                })),
        )
        .child(
            Input::new(&shell.people_telegram_input)
                .id("people-telegram")
                .aria_label("Telegram chat you already know"),
        )
        .child(
            design::button("people-add-telegram", "Add Telegram chat", cx)
                .disabled(!ready)
                .on_click(cx.listener(|shell, _, _, cx| {
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
                })),
        )
}

fn reach_summary(person: &sovra_core::Person) -> String {
    let mut parts = Vec::new();
    for reach in [Reach::Phone, Reach::WhatsApp, Reach::Telegram, Reach::Email] {
        for handle in person.handles.iter().filter(|handle| handle.reach == reach) {
            parts.push(format!("{} {}", reach.label(), handle.label));
        }
    }
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    let summary = english_list(&refs);
    if summary.is_empty() {
        "No phone, WhatsApp, Telegram, or email saved yet.".to_owned()
    } else {
        format!("{summary}.")
    }
}

fn english_list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [first, second] => format!("{first} and {second}"),
        [first, rest @ ..] => {
            let mut text = (*first).to_owned();
            let Some((last, middle)) = rest.split_last() else {
                return text;
            };
            for item in middle {
                text.push_str(", ");
                text.push_str(item);
            }
            text.push_str(", and ");
            text.push_str(last);
            text
        }
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

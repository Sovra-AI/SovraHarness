//! Knowledge tab: saved memory controls, file knowledge, and an illustrative explanation.
//!
//! The picture comes from [`sovra_core::memory_explanation`]. Drawing it does
//! not open a memory store or do any other IO.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, button::ButtonVariants as _,
    checkbox::Checkbox, h_flex, v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::{BeliefPicture, BeliefStanding, MemoryExplanation, StoryStep, memory_explanation};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum KnowledgeSection {
    #[default]
    Files,
    Memories,
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
            .child(v_flex().w_full().min_w_0().flex_shrink_0().max_w(t.layout.reading_width).gap(t.space.small)
                .child(design::button("memory-back-chat", "Back to conversation", cx)
                    .on_click(cx.listener(|shell, _, _, cx| shell.select_tab(sovra_core::NavTab::Chat, cx))))
                .child(div().sovra_text(TextRole::Caption, cx).child("The saved record behind this reference. A reference alone does not prove the reply is correct."))
                .child(shell.memory_view.clone()))
            .into_any_element();
    }
    let column = v_flex()
        .flex_shrink_0()
        .w_full()
        .min_w_0()
        .max_w(t.layout.reading_width)
        .gap(t.space.medium);
    let column = match shell.knowledge_section {
        KnowledgeSection::Files => column.child(crate::file_index_view::render(shell, cx)),
        KnowledgeSection::Memories => column
            .child(render_memory_status(shell, cx))
            .child(shell.memory_view.clone()),
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
        .max_w(t.layout.reading_width)
        .flex_wrap()
        .gap(t.space.small);
    for (id, label, section) in [
        ("knowledge-files", "Files", KnowledgeSection::Files),
        ("knowledge-memories", "Memories", KnowledgeSection::Memories),
        ("knowledge-help", "How it works", KnowledgeSection::Help),
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

fn render_memory_status(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex().id("conversation-memory-status").test_support()
                .aria_label(format!("Conversation memory. {} Messages may be sent to your selected AI service. Saved details are stored on this computer.", shell.workspace().memory_status.label()))
                .w_full().min_w_0().sovra_surface(Surface::Matte, cx)
                .p(t.space.medium).gap(t.space.small)
                .child(div().sovra_text(TextRole::SectionTitle, cx).child("Conversation memory"))
                .child(div().sovra_text(TextRole::Body, cx).child(shell.workspace().memory_status.label().to_owned()))
                .child(div().sovra_text(TextRole::Caption, cx).child("Uses the AI selected in Models. Messages may be sent to that service; saved details are stored on this computer."))
                .child(div().sovra_text(TextRole::Caption, cx).child("Matching saved details can inform answers with source references. Turning remembering off stops new remembering and long-term recall; saved history is kept."))
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

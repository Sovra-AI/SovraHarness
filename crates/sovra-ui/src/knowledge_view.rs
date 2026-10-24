//! Knowledge tab: a plain-language picture of how Sovra remembers.
//!
//! The picture comes from [`sovra_core::memory_explanation`]. Drawing it does
//! not open a memory store or do any other IO.

use gpui_kit::component::{ActiveTheme as _, button::ButtonVariants as _, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::{BeliefPicture, BeliefStanding, MemoryExplanation, StoryStep, memory_explanation};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

pub(crate) fn render_knowledge_view(
    times_open: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let story = memory_explanation();
    let t = design::tokens(cx);
    let mut column = v_flex()
        .w_full()
        .max_w(t.layout.reading_width)
        .gap(t.space.medium)
        .child(render_intro(&story, cx))
        .child(render_story(&story, cx))
        .child(render_about(&story, cx))
        .child(render_search(&story, cx))
        .child(render_times_toggle(times_open, cx));
    if times_open {
        column = column.child(render_times(&story, cx));
    }
    v_flex()
        .id("sovra-knowledge-view")
        .test_support()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .p(t.space.large)
        .child(column)
}

fn render_intro(story: &MemoryExplanation, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("knowledge-intro")
        .test_support()
        .aria_label(format!("{} {}", story.title, story.lead))
        .gap(t.space.compact)
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(cx.theme().link)
                .child("How Sovra remembers"),
        )
        .child(div().sovra_text(TextRole::PageTitle, cx).child(story.title))
        .child(
            div()
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
        .gap(t.space.small)
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
    v_flex()
        .id(("knowledge-step", index))
        .test_support()
        .aria_label(format!("Step {number}. {} {}", step.heading, step.body))
        .w_full()
        .sovra_surface(Surface::Matte, cx)
        .p(t.space.medium)
        .gap(t.space.compact)
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(cx.theme().link)
                .child(format!("Step {number}")),
        )
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
        BeliefStanding::Current => Surface::Elevated,
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

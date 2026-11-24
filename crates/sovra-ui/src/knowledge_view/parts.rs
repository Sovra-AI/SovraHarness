//! Small shared pieces of the Relationships & categories section: pills, numbers
//! at a glance, and the plain names of what Sovra finds in files.
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, TestSupportExt as _, div,
};
use sovra_core::knowledge_analysis::FindingKind;

use crate::design::{self, SovraStyled as _, Surface, TextRole};

/// A quiet label with an optional count, for categories that are read, not pressed.
pub(crate) fn pill(text: String, count: Option<u64>, cx: &App) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .flex_shrink_0()
        .max_w_full()
        .min_w_0()
        .items_center()
        .gap(t.space.compact)
        .px(t.space.small)
        .py(t.space.hair)
        .rounded(t.radius.pill)
        .border_1()
        .border_color(t.material_border)
        .bg(t.subtle)
        .child(
            div()
                .min_w_0()
                .truncate()
                .sovra_text(TextRole::Caption, cx)
                .child(text),
        )
        .children(count.map(|n| {
            div()
                .flex_shrink_0()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(sovra_core::file_index::ontology::group_digits(n))
        }))
}

/// A row of pills that wraps; `id` lets tests and assistive tools find the group.
pub(crate) fn pills(
    id: &'static str,
    label: &str,
    items: Vec<(String, Option<u64>)>,
    cx: &App,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let spoken = items
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    v_flex()
        .id(id)
        .aria_label(format!("{label}: {spoken}"))
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.compact)
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(label.to_owned()),
        )
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .gap(t.space.compact)
                .children(items.into_iter().map(|(text, count)| pill(text, count, cx))),
        )
}

/// One number at a glance with what it counts.
pub(crate) fn stat(
    icon: IconName,
    number: u64,
    caption: &str,
    cx: &App,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .flex_1()
        .min_w(t.layout.filter_width)
        .items_center()
        .gap(t.space.small)
        .p(t.space.small)
        .sovra_surface(Surface::Matte, cx)
        .child(Icon::new(icon).flex_shrink_0().text_color(t.text_secondary))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(sovra_core::file_index::ontology::group_digits(number)),
                )
                .child(
                    div()
                        .min_w_0()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(caption.to_owned()),
                ),
        )
}

/// A heading with one plain sentence under it.
pub(crate) fn heading(title: &str, sentence: &str, cx: &App) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.hair)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(title.to_owned()),
        )
        .child(
            div()
                .min_w_0()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(sentence.to_owned()),
        )
}

/// What a finding kind is called on the page, its icon, and what it means.
pub(crate) fn finding_words(kind: FindingKind) -> (&'static str, IconName, &'static str) {
    match kind {
        FindingKind::Relation => (
            "Connections in sentences",
            IconName::Workflow,
            "Sentences and table rows that say how two things relate.",
        ),
        FindingKind::Measurement => (
            "Measurements",
            IconName::Scale,
            "Weights, lengths, temperatures and other amounts, in comparable units.",
        ),
        FindingKind::Chronology => (
            "Dates & times",
            IconName::Calendar,
            "Dated readings grouped by day, hour or minute.",
        ),
        FindingKind::Profile => (
            "Tables",
            IconName::Table,
            "What kind of table a file holds and how complete it is.",
        ),
        FindingKind::Morphology => (
            "Word forms",
            IconName::PenLine,
            "Words matched by their root, so “notes” also finds “note”.",
        ),
        FindingKind::Topology => (
            "Patterns in numbers",
            IconName::TrendingUp,
            "Clusters and loops in columns of coordinates.",
        ),
        FindingKind::Coverage => (
            "Reading limits",
            IconName::Info,
            "Where a file was too large or unclear to read fully.",
        ),
    }
}

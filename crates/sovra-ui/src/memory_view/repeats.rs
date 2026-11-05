//! Local repeat check: exact repeats link as earlier versions, close wording waits.
//! Nothing is deleted, and close wording is joined only after the person confirms.

use gpui_kit::component::{Disableable as _, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::{
    Fact, MemoryAction, MemoryDuplicateReport, MemorySameDetail, MemoryStatementPair,
    check_memory_duplicates, confirm_memory_duplicate, stored_version_link,
};

use super::MemoryView;
use crate::design::{self, SovraStyled as _, Surface, TextRole};

impl MemoryView {
    pub(super) fn duplicate_check(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut panel = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .child("Look through the details on this page for the same words, and for wording that is close. Nothing is deleted."),
            )
            .child(
                design::button("memory-duplicate-check", "Check for repeats", cx)
                    .self_start()
                    .max_w_full()
                    .disabled(self.state.loading)
                    .on_click(cx.listener(|view, _, _, cx| {
                        let report = check_memory_duplicates(&view.state.page.facts);
                        let links = unlinked(&view.state.page.facts, &report.exact);
                        view.duplicates = Some(report);
                        if links.is_empty() {
                            cx.notify();
                        } else {
                            view.send(MemoryAction::KeepSameDetails(links), cx);
                        }
                    })),
            );
        if let Some(report) = &self.duplicates {
            panel = panel
                .child(exact_block(report, cx))
                .child(near_block(report, cx));
        }
        panel
    }
}

fn unlinked(facts: &[Fact], exact: &[MemoryStatementPair]) -> Vec<MemorySameDetail> {
    exact
        .iter()
        .filter(|pair| !stored_version_link(facts, &pair.newer, &pair.older))
        .map(|pair| MemorySameDetail {
            newer: pair.newer.clone(),
            older: pair.older.clone(),
        })
        .collect()
}

fn exact_block(
    report: &MemoryDuplicateReport,
    cx: &mut Context<MemoryView>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut words = if report.exact.is_empty() {
        "No exact repeats in the details shown.".to_owned()
    } else {
        "Same wording, saved more than once. Both stay. The older one is an earlier version."
            .to_owned()
    };
    for pair in &report.exact {
        words.push(' ');
        words.push_str(&pair.older_words);
        words.push(' ');
        words.push_str(&pair.newer_words);
    }
    v_flex()
        .id("memory-duplicate-exact")
        .test_support()
        .aria_label(words.clone())
        .w_full()
        .min_w_0()
        .gap(t.space.hair)
        .p(t.space.small)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            div()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Body, cx)
                .child(words),
        )
}

fn near_block(
    report: &MemoryDuplicateReport,
    cx: &mut Context<MemoryView>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut panel = v_flex().w_full().min_w_0().gap(t.space.small);
    if report.near.is_empty() {
        return panel.child(
            div()
                .min_w_0()
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .child("No close wording is waiting for you."),
        );
    }
    panel = panel.child(
        div()
            .min_w_0()
            .whitespace_normal()
            .sovra_text(TextRole::Caption, cx)
            .child("These are close, but not the same words. Nothing is joined until you confirm."),
    );
    for (index, pair) in report.near.iter().enumerate() {
        let newer = pair.newer.clone();
        let older = pair.older.clone();
        let label = format!(
            "These are close, but not the same words. Nothing is joined until you confirm. {} {}",
            pair.older_words, pair.newer_words
        );
        panel = panel.child(
            v_flex()
                .id(("memory-duplicate-near", index))
                .test_support()
                .aria_label(label)
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .p(t.space.small)
                .sovra_surface(Surface::Matte, cx)
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Body, cx)
                        .child(pair.older_words.clone()),
                )
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Body, cx)
                        .child(pair.newer_words.clone()),
                )
                .child(
                    design::button(
                        ("memory-duplicate-confirm", index),
                        "Treat as the same detail",
                        cx,
                    )
                    .self_start()
                    .max_w_full()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        let confirmed = view
                            .duplicates
                            .as_mut()
                            .is_some_and(|report| confirm_memory_duplicate(report, &newer, &older));
                        if confirmed && !stored_version_link(&view.state.page.facts, &newer, &older)
                        {
                            view.send(
                                MemoryAction::KeepSameDetails(vec![MemorySameDetail {
                                    newer: newer.clone(),
                                    older: older.clone(),
                                }]),
                                cx,
                            );
                        } else {
                            cx.notify();
                        }
                    })),
                ),
        );
    }
    panel
}

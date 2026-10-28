//! Read every saved text part without loading or rendering a whole source file.

use super::*;
use sovra_core::file_index::BrowserState;

pub(super) fn render(
    entry: &CatalogEntry,
    state: &BrowserState,
    scroll: &gpui_kit::ScrollHandle,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut panel = v_flex().w_full().min_w_0().gap(t.space.small);
    let Some(first) = &entry.first_passage else {
        return panel.child(div().sovra_text(TextRole::Body, cx).child("No searchable text was saved. Read the explanation above; after correcting the source, refresh its folder to retry."));
    };
    let reference = state.preview_reference.as_ref().unwrap_or(first);
    let ordinal = reference.ordinal;
    let last = entry
        .passage_count
        .checked_sub(1)
        .and_then(|n| u32::try_from(n).ok());
    let position = format!(
        "Saved text · part {} of {}",
        u64::from(ordinal) + 1,
        entry.passage_count
    );
    panel = panel
        .child(div().id("index-preview-position").test_support().aria_label(position.clone()).sovra_text(TextRole::Body, cx).child(position))
        .child(div().sovra_text(TextRole::Caption, cx).child("Parts divide the saved text; they are not original document pages. Extraction warnings apply to the whole file. Refresh the folder to capture source changes."));
    let mut actions = h_flex().flex_wrap().gap(t.space.small);
    for (id, label, target) in [
        (
            "index-preview-first",
            "First part",
            (ordinal > 0).then_some(0),
        ),
        (
            "index-preview-previous",
            "Previous part",
            ordinal.checked_sub(1),
        ),
        (
            "index-preview-next",
            "Next part",
            ordinal
                .checked_add(1)
                .filter(|n| u64::from(*n) < entry.passage_count),
        ),
        (
            "index-preview-last",
            "Last part",
            last.filter(|n| *n != ordinal),
        ),
    ] {
        actions = actions.child(
            design::button(id, label, cx)
                .disabled(state.previewing || target.is_none())
                .on_click(cx.listener(move |shell, _, _, cx| {
                    if let Some(target) = target {
                        shell.preview_index_part(target, cx);
                    }
                })),
        );
    }
    panel = panel.child(actions);
    if state.previewing {
        panel = panel.child(
            div()
                .sovra_text(TextRole::Body, cx)
                .child("Loading saved text…"),
        );
    }
    if let Some(error) = &state.preview_error {
        panel = panel
            .child(div().sovra_text(TextRole::Body, cx).child(error.clone()))
            .child(
                design::button("index-preview-retry", "Retry this part", cx).on_click(
                    cx.listener(move |shell, _, _, cx| shell.preview_index_part(ordinal, cx)),
                ),
            );
    }
    if let Some(passage) = &state.preview {
        panel = panel
            .child(div().sovra_text(TextRole::Caption, cx).child(format!(
                "Saved text lines {}–{}",
                passage.start_line, passage.end_line
            )))
            .child(
                div()
                    .id("index-preview-text")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .max_h(t.layout.index_preview_height)
                    .overflow_y_scroll()
                    .overflow_x_scroll()
                    .track_scroll(scroll)
                    .sovra_text(TextRole::Body, cx)
                    .child(passage.text.clone()),
            );
    }
    panel
}

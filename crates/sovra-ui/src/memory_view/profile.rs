//! Personal knowledge summary and explicit capture recovery controls.

use super::*;

impl MemoryView {
    pub(super) fn profile(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let busy = self.state.loading;
        let enabled = self
            .models
            .read(cx)
            .state
            .preferences
            .agent_settings
            .memory_enabled;
        let mut panel = v_flex()
            .id("memory-profile")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.medium)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Your saved knowledge"),
            );
        if let Some(overview) = &self.state.page.overview {
            let mut totals = h_flex()
                .id("memory-totals")
                .test_support()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .gap(t.space.small);
            let narrow = self.width_class == design::WidthClass::Compact;
            if narrow {
                totals = totals.flex_col();
            }
            for (id, count, title) in [
                ("memory-total-current", overview.current, "Saved details"),
                (
                    "memory-total-pending",
                    overview.waiting + overview.working,
                    "Being learned",
                ),
                ("memory-total-failed", overview.failed, "Need attention"),
            ] {
                let card = v_flex()
                    .id(id)
                    .test_support()
                    .aria_label(format!("{count} {title}"))
                    .min_w_0()
                    .gap(t.space.compact)
                    .p(t.space.small)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(
                        div()
                            .sovra_text(TextRole::PageTitle, cx)
                            .child(count.to_string()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .sovra_text(TextRole::Caption, cx)
                            .child(title),
                    );
                totals = totals.child(if narrow { card.w_full() } else { card.flex_1() });
            }
            panel = panel.child(totals);
            if !overview.subjects.is_empty() {
                let mut subjects = h_flex()
                    .id("memory-subjects")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .gap(t.space.small);
                for (index, subject) in overview.subjects.iter().enumerate() {
                    let name = subject.name.clone();
                    subjects = subjects.child(
                        design::button(
                            ("memory-subject", index),
                            format!("{} · {}", subject.name, subject.details),
                            cx,
                        )
                        .max_w_full()
                        .disabled(busy)
                        .selected(self.query.subject.as_ref() == Some(&subject.name))
                        .on_click(cx.listener(
                            move |view, _, window, cx| {
                                view.query.subject = Some(name.clone());
                                view.search
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                view.load(cx);
                            },
                        )),
                    );
                }
                panel = panel.child(div().sovra_text(TextRole::SectionTitle, cx).child("Explore by subject"))
                    .child(div().sovra_text(TextRole::Caption, cx).child("People, places and projects mentioned in your saved details. Select one to see what is known."))
                    .child(subjects);
                if overview.more_subjects {
                    panel = panel.child(div().id("memory-more-subjects").test_support()
                        .sovra_text(TextRole::Caption, cx).child("Showing the 12 most-mentioned subjects. Browse all subjects to explore the full list."));
                }
            }
            if overview.failed > 0 {
                panel = panel.child(v_flex().id("memory-capture-help").test_support()
                    .w_full().min_w_0().gap(t.space.small)
                    .child(div().child("Some messages could not be learned from. Choose a working AI in Models, then try again. Recent activity includes the latest checks and their outcomes."))
                    .child(design::button("memory-retry-failed", "Try failed messages again", cx)
                        .self_start().max_w_full()
                        .disabled(busy || !enabled)
                        .on_click(cx.listener(|view, _, _, cx| view.send(MemoryAction::RetryFailed, cx)))));
            }
            if !overview.recent_activity.is_empty() {
                panel = panel.child(
                    design::button("memory-activity-toggle", "Recent activity", cx)
                        .self_start()
                        .max_w_full()
                        .selected(self.capture.expanded)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.capture.expanded = !view.capture.expanded;
                            cx.notify();
                        })),
                );
                if self.capture.expanded {
                    let mut activity = v_flex().id("memory-activity").test_support()
                        .w_full().min_w_0().gap(t.space.small)
                        .child(div().sovra_text(TextRole::Caption, cx).child("Latest 8 message checks, newest first. A completed check may find nothing new to save."));
                    for (index, item) in overview.recent_activity.iter().enumerate() {
                        let label = item.label();
                        let source_id = item.episode_id.clone();
                        activity = activity.child(
                            v_flex()
                                .id(("memory-activity-item", index))
                                .test_support()
                                .aria_label(label.clone())
                                .w_full()
                                .min_w_0()
                                .p(t.space.small)
                                .sovra_surface(Surface::Subtle, cx)
                                .gap(t.space.small)
                                .child(
                                    div()
                                        .sovra_text(TextRole::Caption, cx)
                                        .child(exploration::date(item.created_at)),
                                )
                                .child(label)
                                .child(
                                    design::button(
                                        ("memory-activity-source", index),
                                        "Read original message",
                                        cx,
                                    )
                                    .disabled(busy)
                                    .on_click(cx.listener(
                                        move |view, _, _, cx| {
                                            view.open_source(
                                                MemoryAction::InspectSource(source_id.clone()),
                                                cx,
                                            );
                                            view.source_navigation = SourceNavigation::Memories;
                                        },
                                    )),
                                ),
                        );
                    }
                    panel = panel.child(activity);
                }
            }
        }
        panel = panel.child(
            design::button("memory-browse-subjects", "Browse all subjects", cx)
                .self_start()
                .max_w_full()
                .disabled(busy)
                .on_click(cx.listener(|view, _, window, cx| {
                    view.subject_search
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    view.load_subjects(sovra_core::MemorySubjectQuery::default(), cx);
                })),
        );
        panel = panel.child(
            design::button("memory-browse-sources", "Browse original records", cx)
                .self_start()
                .max_w_full()
                .disabled(busy)
                .on_click(cx.listener(|view, _, window, cx| {
                    view.source_search
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    view.load_sources(sovra_core::MemorySourceQuery::default(), cx);
                })),
        );
        if let Some(notice) = &self.notice {
            panel = panel.child(
                div()
                    .id("memory-action-notice")
                    .test_support()
                    .role(gpui_kit::Role::Status)
                    .aria_label(notice.clone())
                    .min_w_0()
                    .child(notice.clone()),
            );
        }
        if self.adding {
            panel = panel.child(v_flex().id("memory-note-form").test_support().w_full().min_w_0().gap(t.space.small)
                .child(div().sovra_text(TextRole::SectionTitle, cx).child("Tell me something to remember"))
                .child(div().sovra_text(TextRole::Caption, cx).child("For example: ‘I prefer short answers’ or ‘My sister's name is Maya’. Saved on this computer exactly as you write it. Your selected AI can use it in answers while remembering is on."))
                .child(div().id("memory-note-input").test_support().w_full().min_w_0()
                    .child(Textarea::new(&self.note).disabled(busy).aria_label("Detail to remember")))
                .child(h_flex().w_full().flex_wrap().gap(t.space.small)
                    .child(design::primary_button("memory-add-save", "Save detail", cx).disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| {
                            let note = sovra_core::MemoryNote { text: view.note.read(cx).value().to_string() };
                            if let Err(error) = note.validate() {
                                view.state.error = Some(error.to_string()); cx.notify();
                            } else { view.send(MemoryAction::Remember(note), cx); }
                        })))
                    .child(design::button("memory-add-cancel", "Cancel", cx).disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| { view.adding = false; cx.notify(); })))));
        } else {
            panel = panel.child(
                design::button("memory-add", "Add a detail", cx)
                    .self_start()
                    .max_w_full()
                    .disabled(busy)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.note
                            .update(cx, |note, cx| note.set_value("", window, cx));
                        view.adding = true;
                        view.notice = None;
                        cx.notify();
                    })),
            );
        }
        panel
    }
}

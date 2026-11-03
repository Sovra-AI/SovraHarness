//! A complete, bounded subject directory backed by current saved knowledge.

use super::*;
use sovra_core::MemorySubjectQuery;

impl MemoryView {
    pub(super) fn load_subjects(&mut self, query: MemorySubjectQuery, cx: &mut Context<Self>) {
        self.source_query = None;
        self.subject_query = Some(query.clone());
        self.state.subjects = sovra_core::MemorySubjectPage::default();
        self.state.detail = None;
        self.source_only = false;
        self.editing = None;
        self.stopping = None;
        self.send(MemoryAction::ListSubjects(query), cx);
    }

    pub(super) fn search_subjects(&mut self, cx: &mut Context<Self>) {
        self.load_subjects(
            MemorySubjectQuery {
                search: self.subject_search.read(cx).value().to_string(),
                ..Default::default()
            },
            cx,
        );
    }

    pub(super) fn subject_directory(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let busy = self.state.loading;
        let mut panel = v_flex().id("memory-subject-directory").test_support()
            .w_full().min_w_0().gap(t.space.medium)
            .child(div().sovra_text(TextRole::SectionTitle, cx).child("People, places & projects"))
            .child(div().sovra_text(TextRole::Body, cx).child("Explore subjects mentioned in your current saved details. Counts show how many details mention each subject. These labels do not prove a relationship."))
            .child(design::button("memory-subjects-close", "Back to saved knowledge", cx)
                .self_start().max_w_full().disabled(busy)
                .on_click(cx.listener(|view, _, _, cx| view.load(cx))))
            .child(div().id("memory-subjects-input").test_support().w_full().min_w_0().child(Input::new(&self.subject_search).aria_label("Search knowledge subjects")))
            .child(design::button("memory-subjects-search", "Search subjects", cx)
                .self_start().max_w_full().disabled(busy)
                .on_click(cx.listener(|view, _, _, cx| view.search_subjects(cx))));
        if busy {
            panel = panel.child(
                div()
                    .id("memory-subjects-loading")
                    .test_support()
                    .child("Finding subjects…"),
            );
        } else if let Some(error) = &self.state.error {
            panel = panel.child(
                div()
                    .id("memory-subjects-error")
                    .test_support()
                    .child(error.clone()),
            );
        } else if self.state.subjects.subjects.is_empty() {
            let message = if self
                .subject_query
                .as_ref()
                .is_some_and(|query| !query.search.trim().is_empty() || query.after.is_some())
            {
                "No subjects match this page. Try another name or start again from the beginning."
            } else {
                "No subjects have been identified yet. Your saved details are still available in saved knowledge."
            };
            panel = panel.child(
                div()
                    .id("memory-subjects-empty")
                    .test_support()
                    .aria_label(message)
                    .child(message),
            );
        }
        for (index, subject) in self.state.subjects.subjects.iter().enumerate() {
            let name = subject.name.clone();
            let count = format!(
                "{} saved {}",
                subject.details,
                if subject.details == 1 {
                    "detail"
                } else {
                    "details"
                }
            );
            panel = panel.child(
                v_flex()
                    .id(("memory-subject-row", index))
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .gap(t.space.small)
                    .p(t.space.medium)
                    .sovra_surface(Surface::Subtle, cx)
                    .child(
                        div()
                            .id(("memory-subject-name", index))
                            .test_support()
                            .aria_label(subject.name.clone())
                            .min_w_0()
                            .sovra_text(TextRole::Reading, cx)
                            .child(subject.name.clone()),
                    )
                    .child(div().sovra_text(TextRole::Caption, cx).child(count))
                    .child(
                        design::button(("memory-subject-open", index), "View saved details", cx)
                            .self_start()
                            .max_w_full()
                            .disabled(busy)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.query.subject = Some(name.clone());
                                view.search
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                view.load(cx);
                            })),
                    ),
            );
        }
        let mut paging = h_flex().w_full().min_w_0().flex_wrap().gap(t.space.small);
        if let Some(next) = &self.state.subjects.next {
            let next = next.clone();
            paging = paging.child(
                design::button("memory-subjects-next", "More subjects", cx)
                    .disabled(busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(mut query) = view.subject_query.clone() {
                            query.after = Some(next.clone());
                            view.load_subjects(query, cx);
                        }
                    })),
            );
        }
        if self
            .subject_query
            .as_ref()
            .is_some_and(|query| query.after.is_some() || !query.search.is_empty())
        {
            paging = paging.child(
                design::button("memory-subjects-start", "All subjects from the start", cx)
                    .disabled(busy)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.subject_search
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        view.load_subjects(MemorySubjectQuery::default(), cx);
                    })),
            );
        }
        panel.child(paging)
    }
}

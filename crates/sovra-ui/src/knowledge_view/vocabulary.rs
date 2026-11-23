//! The vocabulary beside reviewed evidence. Selecting a predicate uses the saved graph query.
use super::*;
use sovra_core::file_index::{
    graph::{EntityKind, RelationKind},
    ontology::{AREAS, KINDS},
};

pub(super) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let open = shell.knowledge_vocabulary_open;
    let mut panel = v_flex().min_w_0().w_full().gap(t.space.small).child(
        design::button(
            "knowledge-taxonomy",
            "Relationship types & file categories",
            cx,
        )
        .self_start()
        .icon(if open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .selected(open)
        .on_click(cx.listener(|shell, _, _, cx| {
            shell.knowledge_vocabulary_open = !shell.knowledge_vocabulary_open;
            cx.notify();
        })),
    );
    if open {
        let browser = &shell.file_index.state.knowledge_browser;
        let mut predicates = h_flex().min_w_0().flex_wrap().gap(t.space.compact);
        for (index, kind) in RelationKind::BUILT_INS.iter().enumerate() {
            predicates = predicates.child(predicate_button(index, kind, shell, cx));
        }
        panel = panel
            .child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .child("Choose a relationship type to see its saved connections."),
            )
            .child(predicates);
        let mut custom = Vec::new();
        if let Some(page) = &browser.page {
            for edge in &page.relations {
                if matches!(edge.relation, RelationKind::Property(_))
                    && !custom.contains(&&edge.relation)
                {
                    custom.push(&edge.relation);
                }
            }
        }
        if !custom.is_empty() {
            panel = panel.child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Custom types in these results"),
            );
            for (index, kind) in custom.into_iter().enumerate() {
                let RelationKind::Property(property) = kind else {
                    continue;
                };
                let endpoints =
                    format!("{} → {}", property.subject.label(), property.object.label());
                panel = panel.child(
                    h_flex()
                        .min_w_0()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(predicate_button(
                            index + RelationKind::BUILT_INS.len(),
                            kind,
                            shell,
                            cx,
                        ))
                        .child(div().sovra_text(TextRole::Caption, cx).child(endpoints)),
                );
            }
        }
        for (id, title, labels) in [
            (
                "knowledge-entity-types",
                "Kinds of subjects",
                EntityKind::BUILT_INS
                    .iter()
                    .map(EntityKind::label)
                    .collect::<Vec<_>>(),
            ),
            (
                "knowledge-file-types",
                "File kinds",
                KINDS.iter().map(|k| k.pref_label).collect(),
            ),
            (
                "knowledge-life-areas",
                "Areas of life",
                AREAS.iter().map(|a| a.pref_label).collect(),
            ),
        ] {
            panel = panel.child(
                v_flex()
                    .min_w_0()
                    .gap(t.space.compact)
                    .child(div().sovra_text(TextRole::SectionTitle, cx).child(title))
                    .child(
                        div()
                            .id(id)
                            .test_support()
                            .sovra_text(TextRole::Body, cx)
                            .child(labels.join(" · ")),
                    ),
            );
        }
    }
    panel
}

fn predicate_button(
    index: usize,
    kind: &RelationKind,
    shell: &Shell,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let relation = kind.clone();
    design::button(("knowledge-predicate", index), kind.label().to_owned(), cx)
        .small()
        .max_w_full()
        .selected(
            shell
                .file_index
                .state
                .knowledge_browser
                .query
                .relation
                .as_ref()
                == Some(kind),
        )
        .on_click(cx.listener(move |shell, _, window, cx| {
            shell.filter_by_relationship_type(relation.clone(), window, cx);
        }))
}

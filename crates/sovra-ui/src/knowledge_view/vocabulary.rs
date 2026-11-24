//! The words Sovra sorts with, folded away by default. Choosing a relationship
//! type filters the saved relationships through the saved graph query.
use super::parts::pills;
use super::*;
use sovra_core::file_index::{
    graph::{EntityKind, RelationKind},
    ontology::{AREAS, KINDS},
};

pub(super) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let open = shell.knowledge_vocabulary_open;
    let panel = v_flex().min_w_0().w_full().gap(t.space.medium).child(
        design::button("knowledge-taxonomy", "What Sovra can recognise", cx)
            .ghost()
            .small()
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
    if !open {
        return panel;
    }
    let discovered = shell
        .file_index
        .state
        .discoveries
        .as_ref()
        .and_then(|r| r.as_ref().ok());
    // Types a file defined for itself, such as "grown by" between a product and a grower.
    let mut custom: Vec<&RelationKind> = discovered
        .into_iter()
        .flat_map(|r| &r.predicates)
        .map(|(kind, _)| kind)
        .filter(|kind| matches!(kind, RelationKind::Property(_)))
        .collect();
    if let Some(page) = &shell.file_index.state.knowledge_browser.page {
        for edge in &page.relations {
            if matches!(edge.relation, RelationKind::Property(_))
                && !custom.contains(&&edge.relation)
            {
                custom.push(&edge.relation);
            }
        }
    }
    let mut types = h_flex().min_w_0().w_full().flex_wrap().gap(t.space.compact);
    for (index, kind) in RelationKind::BUILT_INS.iter().enumerate() {
        types = types.child(predicate_button(
            index,
            kind,
            kind.display_name(),
            shell,
            cx,
        ));
    }
    for (index, kind) in custom.into_iter().enumerate() {
        let RelationKind::Property(property) = kind else {
            continue;
        };
        let label = format!(
            "{} ({} → {})",
            kind.display_name(),
            property.subject.display_name(),
            property.object.display_name()
        );
        types = types.child(predicate_button(
            index + RelationKind::BUILT_INS.len(),
            kind,
            label,
            shell,
            cx,
        ));
    }
    let names = |labels: Vec<String>| labels.into_iter().map(|l| (l, None)).collect::<Vec<_>>();
    panel
        .child(
            v_flex()
                .w_full()
                .min_w_0()
                .gap(t.space.compact)
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child("Ways things can relate · choose one to show only those"),
                )
                .child(types),
        )
        .child(pills(
            "knowledge-entity-types",
            "Kinds of things",
            names(
                EntityKind::BUILT_INS
                    .iter()
                    .chain(
                        discovered
                            .into_iter()
                            .flat_map(|r| &r.subjects)
                            .map(|(kind, _)| kind)
                            .filter(|kind| matches!(kind, EntityKind::Domain(_))),
                    )
                    .map(EntityKind::display_name)
                    .collect(),
            ),
            cx,
        ))
        .child(pills(
            "knowledge-file-types",
            "Kinds of files",
            names(KINDS.iter().map(|k| k.pref_label.to_owned()).collect()),
            cx,
        ))
        .child(pills(
            "knowledge-life-areas",
            "Parts of life",
            names(AREAS.iter().map(|a| a.pref_label.to_owned()).collect()),
            cx,
        ))
}

pub(super) fn predicate_button(
    index: usize,
    kind: &RelationKind,
    label: String,
    shell: &Shell,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let relation = kind.clone();
    design::button(("knowledge-predicate", index), label, cx)
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

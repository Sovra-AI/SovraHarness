//! Project the granted file index onto one map. Each expandable node carries its source cursor.
use sovra_core::{
    file_index::{
        FileIndex, IndexError, IndexPassage, PassageRef,
        graph::{GraphEndpoint, GraphQuery, RelationKind},
        summary::{SummaryInputRef, SummaryQuery},
    },
    knowledge_map::{Branch, DetailLevel, Layer, MapPage},
};
type Result<T> = std::result::Result<T, IndexError>;

pub(super) fn passage_key(r: &PassageRef) -> String {
    format!("part:{}:{}:{}", r.file_id, r.version_id, r.ordinal)
}
fn read_part(page: &mut MapPage, part: &IndexPassage) -> String {
    let r = &part.reference;
    let file = page.file(&r.root_id, &r.file_id, &part.path);
    let id = page.node(
        Layer::Files,
        passage_key(r),
        format!(
            "{} · lines {}–{}",
            part.path.rsplit('/').next().unwrap_or(&part.path),
            part.start_line,
            part.end_line
        ),
        format!(
            "{}\n{}{}",
            part.path,
            part.text,
            part.note
                .as_deref()
                .map_or(String::new(), |s| format!("\n{s}"))
        ),
    );
    page.link(&file, &id, "contains passage", true);
    id
}
fn predicate_key(predicate: Option<&RelationKind>) -> String {
    predicate.map_or_else(
        || Layer::Relationships.hub(),
        |p| format!("predicate:{p:?}"),
    )
}

pub(super) fn overview(store: &dyn FileIndex, history: bool) -> Result<MapPage> {
    let mut page = MapPage::default();
    for root in store.list()? {
        let path = root.path.display().to_string();
        let id = page.node(
            Layer::Files,
            format!("root:{}", root.id),
            root.path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&path),
            format!(
                "{path}\n{} files found during the last scan",
                root.progress.discovered
            ),
        );
        page.member(Layer::Files, &id);
        for (facet, label, count) in root
            .progress
            .kinds
            .iter()
            .map(|(kind, n)| ("File kind", kind.label(), n))
            .chain(
                root.progress
                    .areas
                    .iter()
                    .map(|(area, n)| ("Life area", area.label(), n)),
            )
        {
            let key = page.node(Layer::Files, format!("{facet}:{label}"), label, facet);
            page.link(&id, &key, &format!("{count} in last scan"), true);
        }
        page.branch(
            &id,
            Some(Branch::Files {
                root: root.id,
                after: String::new(),
            }),
        );
    }
    page.branch(
        &Layer::Relationships.hub(),
        Some(Branch::Relations(None, None)),
    );
    for (key, title, branch) in [
        ("quotes", "Saved quotations", Branch::Quotes(None)),
        ("summaries", "Saved summaries", Branch::Summaries(None)),
    ] {
        page.node(
            Layer::Files,
            key.into(),
            title,
            "Open the saved records and their sources.",
        );
        page.member(Layer::Files, key);
        page.branch(key, Some(branch));
    }
    if let Some(index) = store.automatic()
        && let Some(summary) = index.overview(None)?
    {
        let total = summary.counts.iter().map(|(_, n)| n).sum::<u64>();
        page.node(
            Layer::Findings,
            Layer::Findings.hub(),
            format!(
                "Findings · {}",
                sovra_core::file_index::ontology::group_digits(total)
            ),
            format!("{total} indexed findings across all connected folders"),
        );
        for (kind, count) in summary.counts {
            let key = page.node(
                Layer::Findings,
                format!("finding:{kind:?}"),
                format!(
                    "{} · {}",
                    kind.label(),
                    sovra_core::file_index::ontology::group_digits(count)
                ),
                format!("{count} indexed findings"),
            );
            page.member(Layer::Findings, &key);
            page.branch(&key, Some(Branch::Findings(kind, None)));
        }
        for (kind, count) in summary.predicates {
            let key = page.node(
                Layer::Relationships,
                predicate_key(Some(&kind)),
                kind.label(),
                format!("Named in {count} file records"),
            );
            page.member(Layer::Relationships, &key);
            if let RelationKind::Property(p) = &kind {
                page.link(
                    &key,
                    &format!("entity-kind:{:?}", p.subject),
                    "declared subject type",
                    false,
                );
                page.link(
                    &key,
                    &format!("entity-kind:{:?}", p.object),
                    "declared object type",
                    false,
                );
            }
            page.branch(&key, Some(Branch::Relations(Some(kind), None)));
        }
        for (facet, label, count) in summary
            .kinds
            .iter()
            .map(|(k, n)| ("File kind", k.concept().pref_label, n))
            .chain(
                summary
                    .areas
                    .iter()
                    .map(|(a, n)| ("Life area", a.concept().pref_label, n)),
            )
        {
            let key = page.describe(
                Layer::Files,
                format!("{facet}:{label}"),
                label,
                format!("{facet} · {count} classified files"),
                DetailLevel::Full,
            );
            page.member(Layer::Files, &key);
        }
        for (kind, count) in summary.subjects {
            let key = page.node(
                Layer::Relationships,
                format!("entity-kind:{kind:?}"),
                kind.label(),
                format!("{count} indexed entity occurrences"),
            );
            page.member(Layer::Relationships, &key);
        }
        for item in summary.examples {
            finding(&mut page, item);
        }
        if summary.working {
            page.notices
                .push("Folder analysis is still adding findings.".into());
        }
    }
    if let Some(graph) = store.knowledge() {
        for proposal in graph.pending()? {
            let id = page.node(
                Layer::Relationships,
                format!("proposal:{}", proposal.id),
                format!("Awaiting review · {}", proposal.draft.title),
                "Suggested relationships awaiting a decision",
            );
            page.member(Layer::Relationships, &id);
            for (index, r) in proposal.draft.relations.iter().enumerate() {
                let claim = page.node(
                    Layer::Relationships,
                    format!("{id}:claim:{index}"),
                    &r.statement,
                    format!(
                        "Proposed · {} {} {}",
                        proposal.endpoint_label(&r.subject),
                        r.relation.label(),
                        proposal.endpoint_label(&r.object)
                    ),
                );
                page.link(&id, &claim, "suggests", false);
                for n in &r.evidence {
                    if let Some(p) = proposal.sources.get(usize::from(*n)) {
                        let part = read_part(&mut page, p);
                        page.link(&claim, &part, "cites", false);
                    }
                }
            }
        }
        match expand(store, Branch::Relations(None, None), history) {
            Ok(relations) => page.merge(relations),
            Err(e) => page.notices.push(e.0),
        }
    }
    Ok(page)
}

fn finding(page: &mut MapPage, item: sovra_core::file_index::automatic::DiscoveredFinding) {
    let r = &item.reference;
    let file = page.file(&r.root_id, &r.file_id, &item.path);
    let key = page.node(
        Layer::Findings,
        format!("found:{}", item.id),
        item.detail.clone(),
        format!("{} · line {}\n{}", item.path, item.line, item.detail),
    );
    page.link(&format!("finding:{:?}", item.kind), &key, "contains", true);
    page.link(&key, &file, "found in", false);
    page.link(&key, &passage_key(r), "source wording", false);
    page.branch(&key, Some(Branch::Passage(item.reference)));
}

pub(super) fn expand(store: &dyn FileIndex, branch: Branch, history: bool) -> Result<MapPage> {
    let mut page = MapPage::default();
    match branch {
        Branch::Files { root, after } => return store.map_folder(&root, &after, history),
        Branch::Observations(q) if history => {
            let record = store.history(&q)?;
            let key = format!("history:{}", record.file_id);
            for obs in record.observations {
                let id = page.node(
                    Layer::Files,
                    format!("observation:{}", obs.sequence),
                    format!("{} · observation {}", obs.path, obs.sequence),
                    format!(
                        "{:?} · {:?} · observed at {} (Unix seconds)",
                        obs.presence, obs.extraction, obs.basis.recorded_at
                    ),
                );
                page.link(&key, &id, "includes observation", true);
                if let Some(prior) = obs.replaces {
                    page.link(&id, &format!("observation:{prior}"), "replaces", false);
                }
                if let Some(r) = obs.representation.and_then(|v| v.first_passage) {
                    page.branch(&id, Some(Branch::Historical(r)));
                }
            }
            page.branch(
                &key,
                record.next_before.map(|b| {
                    Branch::Observations(sovra_core::file_index::FileHistoryQuery {
                        before: Some(b),
                        ..q
                    })
                }),
            );
        }
        Branch::Historical(r) if history => {
            let result = store.read_history(&r)?;
            let key = format!("observation:{}", r.observation);
            let part = read_part(&mut page, &result.passage);
            page.link(&key, &part, "observed wording", false);
            page.branch(&key, result.next.map(Branch::Historical));
        }
        Branch::Parts(r, total) => {
            let file = format!("file:{}", r.file_id);
            let end = r.ordinal.saturating_add(8).min(total);
            for ordinal in r.ordinal..end {
                read_part(
                    &mut page,
                    &store.read(&PassageRef {
                        ordinal,
                        ..r.clone()
                    })?,
                );
            }
            page.branch(
                &file,
                (end < total).then_some(Branch::Parts(PassageRef { ordinal: end, ..r }, total)),
            );
        }
        Branch::Passage(r) => {
            read_part(&mut page, &store.read(&r)?);
        }
        Branch::Findings(kind, before) => {
            let items = store
                .automatic()
                .ok_or_else(|| IndexError("Folder analysis is unavailable.".into()))?
                .findings(kind, before)?;
            for item in items.items {
                finding(&mut page, item);
            }
            page.branch(
                &format!("finding:{kind:?}"),
                items.next.map(|n| Branch::Findings(kind, Some(n))),
            );
        }
        Branch::Relations(..) | Branch::Entity(..) => {
            let (predicate, entity, cursor) = match &branch {
                Branch::Relations(p, c) => (p.clone(), None, c.clone()),
                Branch::Entity(id, c) => (None, Some(id.clone()), c.clone()),
                _ => unreachable!(),
            };
            let key = entity.as_ref().map_or_else(
                || predicate_key(predicate.as_ref()),
                |e| format!("entity:{e}"),
            );
            let query = GraphQuery {
                relation: predicate.clone(),
                entity_id: entity.clone(),
                before: cursor,
                include_stale: history,
                include_superseded: history,
                ..Default::default()
            };
            let graph = store
                .knowledge()
                .ok_or_else(|| IndexError("File relationships are unavailable.".into()))?
                .query(&query)?;
            for e in graph.entities {
                let id = page.node(
                    Layer::Relationships,
                    format!("entity:{}", e.id),
                    e.label,
                    e.kind.label(),
                );
                page.link(
                    &format!("entity-kind:{:?}", e.kind),
                    &id,
                    "classifies",
                    true,
                );
                page.branch(&id, Some(Branch::Entity(e.id, None)));
            }
            for s in graph.sources {
                let file = page.file(&s.reference.root_id, &s.reference.file_id, &s.path);
                let id = page.node(
                    Layer::Files,
                    format!("evidence:{}", s.id),
                    format!(
                        "{} · lines {}–{}",
                        s.path.rsplit('/').next().unwrap_or(&s.path),
                        s.start_line,
                        s.end_line
                    ),
                    s.path,
                );
                page.link(&id, &file, "passage from", false);
                page.branch(&id, Some(Branch::Evidence(s.id)));
            }
            let endpoint = |e: GraphEndpoint| match e {
                GraphEndpoint::Entity(id) => format!("entity:{id}"),
                GraphEndpoint::Source(id) => format!("evidence:{id}"),
            };
            for r in graph.relations {
                let from = endpoint(r.subject);
                let to = endpoint(r.object);
                let id = page.node(
                    Layer::Relationships,
                    format!("claim:{}", r.id),
                    r.statement.clone(),
                    format!(
                        "{}\nEvidence · {:?}{}{}",
                        r.statement,
                        r.basis.tier,
                        if r.sources_current {
                            ""
                        } else {
                            " · older file version"
                        },
                        if r.expired_at.is_some() {
                            " · replaced"
                        } else {
                            ""
                        }
                    ),
                );
                page.member(Layer::Relationships, &id);
                page.link(&predicate_key(Some(&r.relation)), &id, "contains", true);
                page.link(&from, &to, r.relation.label(), false);
                page.link(&id, &from, "subject", false);
                page.link(&id, &to, "object", false);
                for e in r.evidence {
                    page.link(&id, &format!("evidence:{e}"), "supported by", false);
                }
            }
            page.branch(
                &key,
                graph.next.map(|c| {
                    if let Some(e) = entity {
                        Branch::Entity(e, Some(c))
                    } else {
                        Branch::Relations(predicate, Some(c))
                    }
                }),
            );
        }
        Branch::Evidence(id) => {
            let s = store
                .knowledge()
                .ok_or_else(|| IndexError("Evidence is unavailable.".into()))?
                .evidence(id)?;
            let part = read_part(&mut page, &s.passage);
            page.link(&format!("evidence:{id}"), &part, "original wording", false);
        }
        Branch::Quotes(before) => {
            let quotes = store.saved_answers(before)?;
            for q in quotes.answers {
                let r = q.reference();
                let file = page.file(&r.root_id, &r.file_id, q.file());
                let id = page.node(
                    Layer::Files,
                    format!("quote:{}", passage_key(r)),
                    q.wording(),
                    q.screen_words(),
                );
                page.link("quotes", &id, "contains", true);
                page.link(&id, &file, "quoted from", false);
            }
            page.branch(
                "quotes",
                quotes.next_before.map(|n| Branch::Quotes(Some(n))),
            );
        }
        Branch::Summaries(before_id) => {
            let list = store
                .summaries()
                .ok_or_else(|| IndexError("Summaries are unavailable.".into()))?
                .query_summaries(&SummaryQuery {
                    before_id,
                    include_stale: history,
                    ..Default::default()
                })?;
            for s in list.summaries {
                let key = page.node(
                    Layer::Files,
                    format!("summary:{}", s.id),
                    s.title,
                    s.preview,
                );
                page.link("summaries", &key, "contains", true);
                page.branch(&key, Some(Branch::Summary(s.id)));
            }
            page.branch(
                "summaries",
                list.next_before.map(|n| Branch::Summaries(Some(n))),
            );
        }
        Branch::Summary(id) => {
            let s = store
                .summaries()
                .ok_or_else(|| IndexError("Summaries are unavailable.".into()))?
                .read_summary(id, history)?;
            let key = page.node(
                Layer::Files,
                format!("summary:{id}"),
                s.content.title.clone(),
                s.content.embedding_text(),
            );
            for input in s.inputs {
                let (child, branch) = match input.reference {
                    SummaryInputRef::Passage(r) => (passage_key(&r), Branch::Passage(r)),
                    SummaryInputRef::Summary(n) => (format!("summary:{n}"), Branch::Summary(n)),
                };
                page.describe(
                    Layer::Files,
                    child.clone(),
                    input.label,
                    input.text,
                    DetailLevel::Reference,
                );
                page.branch(&child, Some(branch));
                page.link(&key, &child, "summarizes", false);
            }
        }
        _ => return Err(IndexError("Choose a file collection.".into())),
    }
    Ok(page)
}

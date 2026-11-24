//! Page the opened spaces of all current memberships without contacting peers.
use super::{GroupKnowledge, VisibleGroup};
use sovra_core::{
    group::{DetailStatus, OpHash, PersonId, SharedArtifact, SharedDetail, SharedTemplate},
    knowledge_map::{Branch, Layer, MapPage},
};
use std::{
    collections::BTreeMap,
    ops::Bound::{Excluded, Unbounded},
};

const PAGE: usize = 32;
enum Shared<'a> {
    Detail(&'a SharedDetail),
    Artifact(&'a SharedArtifact),
    Template(&'a SharedTemplate),
}

impl GroupKnowledge {
    pub(crate) async fn map_page(&self, history: bool, scope: Option<(&str, OpHash)>) -> MapPage {
        let mut page = MapPage::default();
        for g in self
            .0
            .read()
            .await
            .values()
            .filter(|g| scope.is_none_or(|(id, _)| g.id == id))
        {
            let group = page.node(
                Layer::Groups,
                format!("group:{}", g.id),
                &g.name,
                "Shared group",
            );
            page.member(Layer::Groups, &group);
            if scope.is_none() {
                for (person, name) in &g.members {
                    let id = member(&mut page, g, *person, name);
                    page.link(&group, &id, "has member", false);
                }
            }
            let bounds = (
                scope.map_or(Unbounded, |(_, cursor)| Excluded(cursor)),
                Unbounded,
            );
            // Each lane is already sorted by stable operation ID. Only the next page
            // of each lane can contribute to the first page of their union.
            let mut items = BTreeMap::new();
            for view in &g.views {
                items.extend(
                    view.details
                        .range(bounds)
                        .filter(|(_, d)| visible(d.usable(), &d.status, history))
                        .take(PAGE + 1)
                        .map(|(key, d)| (*key, Shared::Detail(d))),
                );
                items.extend(
                    view.artifacts
                        .range(bounds)
                        .filter(|(_, a)| visible(a.is_latest(), &a.status, history))
                        .take(PAGE + 1)
                        .map(|(key, a)| (*key, Shared::Artifact(a))),
                );
                items.extend(
                    view.templates
                        .range(bounds)
                        .filter(|(_, t)| visible(t.is_latest(), &t.status, history))
                        .take(PAGE + 1)
                        .map(|(key, t)| (*key, Shared::Template(t))),
                );
            }
            let more = items.len() > PAGE;
            let mut last = None;
            for (key, item) in items.into_iter().take(PAGE) {
                let id = format!("shared:{}", key.to_hex());
                let (person, author, previous) = match item {
                    Shared::Detail(d) => {
                        page.node(
                            Layer::Groups,
                            id.clone(),
                            &d.text,
                            format!("{} · {}\n{}", d.shared_by, standing(&d.status), d.text),
                        );
                        page.link(&group, &id, "shared detail", true);
                        for topic in &d.about {
                            page.subject(&id, topic, Layer::Groups);
                        }
                        for person in &d.agreed_by {
                            let name = g
                                .members
                                .get(person)
                                .map_or("Former member", String::as_str);
                            let who = member(&mut page, g, *person, name);
                            page.link(&who, &id, "agreed with", false);
                        }
                        (d.person, &d.shared_by, &d.replaces)
                    }
                    Shared::Artifact(a) => {
                        page.node(
                            Layer::Groups,
                            id.clone(),
                            &a.title,
                            format!(
                                "{} · {} · {}\n{}",
                                a.kind.label(),
                                a.shared_by,
                                standing(&a.status),
                                a.content
                            ),
                        );
                        page.link(&group, &id, "shared work", true);
                        (a.person, &a.shared_by, &a.replaces)
                    }
                    Shared::Template(t) => {
                        page.node(
                            Layer::Groups,
                            id.clone(),
                            t.template.name(),
                            format!(
                                "{} · {}\n{}",
                                t.shared_by,
                                standing(&t.status),
                                t.template.body()
                            ),
                        );
                        page.link(&group, &id, "shared template", true);
                        (t.person, &t.shared_by, &t.replaces)
                    }
                };
                if let Some(person) = person {
                    let who = member(&mut page, g, person, author);
                    page.link(&id, &who, "shared by", false);
                }
                for old in previous {
                    page.link(&id, &format!("shared:{}", old.to_hex()), "replaces", false);
                }
                last = Some(key);
            }
            page.branch(
                &group,
                last.filter(|_| more)
                    .map(|after| Branch::GroupRecords(g.id.clone(), after)),
            );
        }
        page
    }
}
fn member(page: &mut MapPage, g: &VisibleGroup, person: PersonId, name: &str) -> String {
    page.node(
        Layer::Groups,
        format!("member:{}:{}", g.id, person.to_hex()),
        name,
        &g.name,
    )
}
fn visible(current: bool, status: &DetailStatus, history: bool) -> bool {
    current || (history && !matches!(status, DetailStatus::Redacted))
}
fn standing(s: &DetailStatus) -> &'static str {
    match s {
        DetailStatus::Current => "Current",
        DetailStatus::Replaced { .. } => "Replaced",
        DetailStatus::Disputed { .. } => "In dispute",
        DetailStatus::Withdrawn => "Withdrawn",
        DetailStatus::Redacted => "Erased",
    }
}

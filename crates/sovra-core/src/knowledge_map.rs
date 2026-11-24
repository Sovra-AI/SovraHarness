//! Local review of all knowledge sources. Keep record identity, evidence and collection
//! membership separate so a shared display name never silently becomes a fact.
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    MemoryCursor, MemorySourceCursor,
    file_index::{
        PassageRef,
        graph::{GraphCursor, RelationKind},
    },
    knowledge_analysis::FindingKind,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Layer {
    Files,
    Findings,
    Relationships,
    Personal,
    Artifacts,
    People,
    Groups,
}
impl Layer {
    pub const ALL: [Self; 7] = [
        Self::Files,
        Self::Findings,
        Self::Relationships,
        Self::Personal,
        Self::Artifacts,
        Self::People,
        Self::Groups,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Findings => "Findings",
            Self::Relationships => "Relationships",
            Self::Personal => "About you",
            Self::Artifacts => "Saved work",
            Self::People => "People",
            Self::Groups => "Group knowledge",
        }
    }
    pub fn hub(self) -> String {
        format!("layer:{self:?}")
    }
}

/// Each continuation is issued by its source. Opening another source adds to the same map.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Branch {
    Overview,
    Files { root: String, after: String },
    Observations(crate::file_index::FileHistoryQuery),
    Historical(crate::file_index::HistoryPassageRef),
    Passage(PassageRef),
    Parts(PassageRef, u32),
    Entity(String, Option<GraphCursor>),
    Findings(FindingKind, Option<i64>),
    Relations(Option<RelationKind>, Option<GraphCursor>),
    Evidence(u64),
    Quotes(Option<u64>),
    Summaries(Option<u64>),
    Summary(u64),
    Facts(Option<MemoryCursor>),
    Sources(Option<MemorySourceCursor>),
    Original(crate::EpisodeId),
    Artifacts(Option<String>),
    GroupRecords(String, crate::group::OpHash),
    Artifact(String, Option<usize>),
    Search(String),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MapRequest {
    pub id: u64,
    pub branch: Branch,
    pub history: bool,
}
/// A later citation must keep the wording already opened in the inspector.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum DetailLevel {
    Reference,
    Record,
    Full,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Node {
    detail_level: DetailLevel,
    pub id: String,
    pub layer: Layer,
    pub label: String,
    pub detail: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Link {
    pub from: String,
    pub to: String,
    pub label: String,
    pub membership: bool,
}

#[derive(Clone, Default, Debug, Eq, PartialEq)]
pub struct MapPage {
    pub nodes: BTreeMap<String, Node>,
    pub links: BTreeSet<Link>,
    /// Some opens a branch; None marks an exhausted branch even when records repeat.
    pub branches: BTreeMap<String, Option<Branch>>,
    pub notices: Vec<String>,
}

impl MapPage {
    pub fn node(
        &mut self,
        layer: Layer,
        id: String,
        label: impl Into<String>,
        detail: impl Into<String>,
    ) -> String {
        self.describe(layer, id, label, detail, DetailLevel::Record)
    }
    pub fn describe(
        &mut self,
        layer: Layer,
        id: String,
        label: impl Into<String>,
        detail: impl Into<String>,
        detail_level: DetailLevel,
    ) -> String {
        self.insert_node(Node {
            id: id.clone(),
            layer,
            label: label.into(),
            detail: detail.into(),
            detail_level,
        });
        id
    }
    fn insert_node(&mut self, node: Node) {
        if self
            .nodes
            .get(&node.id)
            .is_none_or(|old| old.detail_level <= node.detail_level)
        {
            self.nodes.insert(node.id.clone(), node);
        }
    }
    pub fn link(&mut self, from: &str, to: &str, label: &str, membership: bool) {
        self.links.insert(Link {
            from: from.into(),
            to: to.into(),
            label: label.into(),
            membership,
        });
    }
    pub fn member(&mut self, layer: Layer, id: &str) {
        self.link(&layer.hub(), id, "contains", true);
    }
    pub fn branch(&mut self, id: &str, branch: Option<Branch>) {
        self.branches.insert(id.into(), branch);
    }
    pub fn merge(&mut self, mut other: Self) {
        for node in other.nodes.into_values() {
            self.insert_node(node);
        }
        self.links.append(&mut other.links);
        self.branches.append(&mut other.branches);
        self.notices.append(&mut other.notices);
    }
    pub fn hubs(&mut self) {
        for layer in Layer::ALL {
            self.node(layer, layer.hub(), layer.label(), "Knowledge collection");
        }
    }
    pub fn file(&mut self, root: &str, file: &str, path: &str) -> String {
        let id = self.node(
            Layer::Files,
            format!("file:{file}"),
            path.rsplit('/').next().unwrap_or(path),
            path,
        );
        self.link(&format!("root:{root}"), &id, "contains", true);
        id
    }
    pub fn subject(&mut self, record: &str, label: &str, layer: Layer) {
        let id = format!("topic:{}", label.to_lowercase());
        // A topic is a shared label, never a merged person or entity identity.
        self.nodes.entry(id.clone()).or_insert_with(|| Node {
            id: id.clone(),
            layer,
            label: label.into(),
            detail: "A topic named in a source".into(),
            detail_level: DetailLevel::Reference,
        });
        self.link(record, &id, "mentions topic", false);
    }
    pub fn chat(&mut self, record: &str, session: &str, title: &str) {
        if session.is_empty() {
            return;
        }
        let id = format!("chat:{session}");
        if title.is_empty() {
            self.describe(
                Layer::Personal,
                id.clone(),
                "Conversation",
                session,
                DetailLevel::Reference,
            );
        } else {
            self.node(Layer::Personal, id.clone(), title, session);
        }
        self.link(record, &id, "from conversation", false);
    }
    pub fn people(
        &mut self,
        book: &crate::people::PeopleBook,
        groups: &[crate::connections::SyncedGroup],
    ) {
        for person in &book.people {
            let id = self.node(
                Layer::People,
                format!("contact:{}", person.id),
                person.title(),
                person
                    .handles
                    .iter()
                    .map(|h| format!("{} · {}", h.reach.label(), h.label))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            self.member(Layer::People, &id);
        }
        for group in groups {
            let id = self.node(
                Layer::People,
                format!("contact-group:{}", group.id),
                if group.subject.is_empty() {
                    &group.id
                } else {
                    &group.subject
                },
                "WhatsApp group",
            );
            self.member(Layer::People, &id);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn citations_keep_the_full_record_after_another_page_is_added() {
        let mut page = MapPage::default();
        page.describe(
            Layer::Personal,
            "source:1".into(),
            "Source",
            "Full wording",
            DetailLevel::Full,
        );
        let mut next = MapPage::default();
        next.describe(
            Layer::Personal,
            "source:1".into(),
            "Original",
            "Open source",
            DetailLevel::Reference,
        );
        page.merge(next);
        assert_eq!(page.nodes["source:1"].detail, "Full wording");
    }
    #[test]
    fn collections_share_provenance_without_merging_equal_labels() {
        let mut page = MapPage::default();
        page.hubs();
        for (layer, id) in [
            (Layer::Personal, "fact:1"),
            (Layer::Artifacts, "artifact:1"),
        ] {
            page.node(layer, id.into(), "Same words", "");
            page.member(layer, id);
            page.chat(id, "one", "Source chat");
        }
        assert!(page.nodes.contains_key("fact:1") && page.nodes.contains_key("artifact:1"));
        assert_eq!(page.links.iter().filter(|l| l.to == "chat:one").count(), 2);
    }
}

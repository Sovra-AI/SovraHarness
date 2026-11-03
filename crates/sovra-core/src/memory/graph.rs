//! Bounded picture of saved links, and a local check for repeated wording.
//!
//! Layout is a pure function of the nodes it is given. It does not call a
//! model, a browser, or the network. Duplicate checks compare wording only:
//! an exact normalized repeat is linked as an earlier version, and both
//! records stay. Close wording is reported and is not joined here.

use super::{Fact, FactId, Timestamp, exploration::MAX_MEMORY_CONNECTIONS};

/// Session name for one saved detail, when the original chat is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryFactChat {
    pub fact: FactId,
    pub chat: String,
}

/// One saved detail, plus the chat it came from when that chat is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryGraphRecord {
    pub fact: Fact,
    pub chat: Option<String>,
}

/// A point in the picture, in thousandths of the drawing area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MemoryGraphNodeId {
    Detail(FactId),
    Chat(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryGraphNode {
    pub id: MemoryGraphNodeId,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryGraphEdgeKind {
    EarlierVersion,
    SameMessage,
    SharedSubject(Vec<String>),
    SavedFromChat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryGraphEdge {
    pub from: MemoryGraphNodeId,
    pub to: MemoryGraphNodeId,
    pub kind: MemoryGraphEdgeKind,
}

impl MemoryGraphEdgeKind {
    pub fn label(&self) -> String {
        match self {
            Self::EarlierVersion => "Earlier version of this detail".into(),
            Self::SameMessage => "From the same original message".into(),
            Self::SharedSubject(names) => format!("Also mentions {}", names.join(", ")),
            Self::SavedFromChat => "Saved from this chat".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryGraph {
    pub nodes: Vec<MemoryGraphNode>,
    pub edges: Vec<MemoryGraphEdge>,
    pub points: Vec<MemoryPoint>,
    /// The bounded picture left some real links out.
    pub omitted: bool,
}

/// Two saved details whose wording matches or nearly matches.
/// `newer` was saved later. Both records stay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryStatementPair {
    pub newer: FactId,
    pub older: FactId,
    pub newer_words: String,
    pub older_words: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryDuplicateReport {
    /// Wording that normalizes to the same statement. Already linked.
    pub exact: Vec<MemoryStatementPair>,
    /// Close wording. Not joined until the person confirms.
    pub near: Vec<MemoryStatementPair>,
}

/// Places nodes on a ring. The same node list always produces the same points.
pub fn layout_memory_graph(nodes: &[MemoryGraphNode]) -> Vec<MemoryPoint> {
    let count = nodes.len();
    (0..count).map(|index| place_node(index, count)).collect()
}

fn place_node(index: usize, count: usize) -> MemoryPoint {
    if count <= 1 {
        return MemoryPoint { x: 500, y: 500 };
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "a review picture has at most a few dozen nodes"
    )]
    let turns = index as f64 / count as f64;
    let angle = turns * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
    MemoryPoint {
        x: thousand(500.0 + 380.0 * angle.cos()),
        y: thousand(500.0 + 380.0 * angle.sin()),
    }
}

fn thousand(value: f64) -> i32 {
    let rounded = value.round().clamp(0.0, 1000.0);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the value is clamped to 0..=1000 before it becomes a coordinate"
    )]
    {
        rounded as i32
    }
}

/// Builds the bounded picture from stored links only.
/// `versions` are exact repeats already classified, or near-duplicates the
/// person has confirmed. A shared generic "you" label is not a link.
pub fn memory_graph(
    records: &[MemoryGraphRecord],
    versions: &[MemoryStatementPair],
) -> MemoryGraph {
    let mut records = records.to_vec();
    records.sort_by(|a, b| a.fact.id.0.cmp(&b.fact.id.0));
    records.dedup_by(|a, b| a.fact.id == b.fact.id);

    let mut candidates = Vec::new();
    for pair in versions {
        if records.iter().any(|record| record.fact.id == pair.newer)
            && records.iter().any(|record| record.fact.id == pair.older)
        {
            candidates.push(candidate(
                0,
                MemoryGraphNodeId::Detail(pair.newer.clone()),
                MemoryGraphNodeId::Detail(pair.older.clone()),
                MemoryGraphEdgeKind::EarlierVersion,
            ));
        }
    }
    for record in &records {
        if let Some(previous) = &record.fact.replaces
            && records.iter().any(|other| &other.fact.id == previous)
        {
            candidates.push(candidate(
                0,
                MemoryGraphNodeId::Detail(record.fact.id.clone()),
                MemoryGraphNodeId::Detail(previous.clone()),
                MemoryGraphEdgeKind::EarlierVersion,
            ));
        }
        if let Some(chat) = record.chat.as_ref().filter(|chat| !chat.trim().is_empty()) {
            candidates.push(candidate(
                1,
                MemoryGraphNodeId::Detail(record.fact.id.clone()),
                MemoryGraphNodeId::Chat(chat.trim().to_owned()),
                MemoryGraphEdgeKind::SavedFromChat,
            ));
        }
    }
    for left in 0..records.len() {
        for right in (left + 1)..records.len() {
            let a = &records[left];
            let b = &records[right];
            if a.fact.source.is_some() && a.fact.source == b.fact.source {
                candidates.push(candidate(
                    2,
                    MemoryGraphNodeId::Detail(a.fact.id.clone()),
                    MemoryGraphNodeId::Detail(b.fact.id.clone()),
                    MemoryGraphEdgeKind::SameMessage,
                ));
            }
            let mut shared = a
                .fact
                .entities
                .iter()
                .filter(|name| {
                    !generic_subject(name) && b.fact.entities.iter().any(|other| other == *name)
                })
                .cloned()
                .collect::<Vec<_>>();
            shared.sort();
            shared.dedup();
            if !shared.is_empty() {
                candidates.push(candidate(
                    3,
                    MemoryGraphNodeId::Detail(a.fact.id.clone()),
                    MemoryGraphNodeId::Detail(b.fact.id.clone()),
                    MemoryGraphEdgeKind::SharedSubject(shared),
                ));
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then(node_sort_key(&a.edge.from).cmp(&node_sort_key(&b.edge.from)))
            .then(node_sort_key(&a.edge.to).cmp(&node_sort_key(&b.edge.to)))
            .then_with(|| edge_name(&a.edge.kind).cmp(&edge_name(&b.edge.kind)))
    });
    candidates.dedup_by(|a, b| {
        a.edge.from == b.edge.from
            && a.edge.to == b.edge.to
            && edge_name(&a.edge.kind) == edge_name(&b.edge.kind)
    });

    let omitted_edges = candidates.len() > MAX_MEMORY_CONNECTIONS;
    candidates.truncate(MAX_MEMORY_CONNECTIONS);

    let mut nodes = Vec::new();
    let mut seen_nodes = std::collections::HashSet::new();
    for edge in &candidates {
        push_node(
            &mut nodes,
            &mut seen_nodes,
            &records,
            edge.edge.from.clone(),
        );
        push_node(&mut nodes, &mut seen_nodes, &records, edge.edge.to.clone());
    }
    let mut fact_count = nodes
        .iter()
        .filter(|node| matches!(node.id, MemoryGraphNodeId::Detail(_)))
        .count();
    let mut omitted_facts = false;
    for record in &records {
        let id = MemoryGraphNodeId::Detail(record.fact.id.clone());
        if seen_nodes.contains(&id) {
            continue;
        }
        if fact_count >= MAX_MEMORY_CONNECTIONS {
            omitted_facts = true;
            continue;
        }
        push_node(&mut nodes, &mut seen_nodes, &records, id);
        fact_count += 1;
    }
    nodes.sort_by(|a, b| node_sort_key(&a.id).cmp(&node_sort_key(&b.id)));
    let points = layout_memory_graph(&nodes);
    MemoryGraph {
        edges: candidates.into_iter().map(|item| item.edge).collect(),
        nodes,
        points,
        omitted: omitted_edges || omitted_facts,
    }
}

struct RankedEdge {
    rank: u8,
    edge: MemoryGraphEdge,
}

fn candidate(
    rank: u8,
    from: MemoryGraphNodeId,
    to: MemoryGraphNodeId,
    kind: MemoryGraphEdgeKind,
) -> RankedEdge {
    RankedEdge {
        rank,
        edge: MemoryGraphEdge { from, to, kind },
    }
}

fn edge_name(kind: &MemoryGraphEdgeKind) -> String {
    kind.label()
}

fn node_sort_key(id: &MemoryGraphNodeId) -> (u8, &str) {
    match id {
        MemoryGraphNodeId::Chat(name) => (0, name.as_str()),
        MemoryGraphNodeId::Detail(id) => (1, id.0.as_str()),
    }
}

fn push_node(
    nodes: &mut Vec<MemoryGraphNode>,
    seen: &mut std::collections::HashSet<MemoryGraphNodeId>,
    records: &[MemoryGraphRecord],
    id: MemoryGraphNodeId,
) {
    if !seen.insert(id.clone()) {
        return;
    }
    let label = match &id {
        MemoryGraphNodeId::Chat(name) => format!("Chat {name}"),
        MemoryGraphNodeId::Detail(fact_id) => records
            .iter()
            .find(|record| &record.fact.id == fact_id)
            .map_or_else(
                || "Saved detail".into(),
                |record| clip_label(&record.fact.content),
            ),
    };
    nodes.push(MemoryGraphNode { id, label });
}

fn clip_label(text: &str) -> String {
    match text.char_indices().nth(72) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text.to_owned(),
    }
}

fn generic_subject(name: &str) -> bool {
    let name = name.trim();
    name.eq_ignore_ascii_case("user") || name.eq_ignore_ascii_case("you")
}

/// Finds exact repeats and close wording in the details already loaded.
/// Nothing is removed from `facts`.
pub fn check_memory_duplicates(facts: &[Fact]) -> MemoryDuplicateReport {
    let mut ordered = facts.to_vec();
    ordered.sort_by(|a, b| a.fact_order().cmp(&b.fact_order()));
    ordered.dedup_by(|a, b| a.id == b.id);

    let mut exact_groups: Vec<Vec<Fact>> = Vec::new();
    for fact in &ordered {
        let key = statement_key(&fact.content);
        if key.is_empty() {
            continue;
        }
        if let Some(group) = exact_groups
            .iter_mut()
            .find(|group| statement_key(&group[0].content) == key)
        {
            group.push(fact.clone());
        } else {
            exact_groups.push(vec![fact.clone()]);
        }
    }
    let mut exact = Vec::new();
    let mut representative = std::collections::HashMap::new();
    for group in &exact_groups {
        let oldest = &group[0].id;
        for fact in group {
            representative.insert(fact.id.clone(), oldest.clone());
        }
        if group.len() < 2 {
            continue;
        }
        let older = &group[0];
        for newer in group.iter().skip(1) {
            exact.push(MemoryStatementPair {
                newer: newer.id.clone(),
                older: older.id.clone(),
                newer_words: newer.content.clone(),
                older_words: older.content.clone(),
            });
        }
    }

    let mut near = Vec::new();
    for left in 0..ordered.len() {
        for right in (left + 1)..ordered.len() {
            let older = &ordered[left];
            let newer = &ordered[right];
            if representative
                .get(&older.id)
                .is_some_and(|id| id != &older.id)
                || representative
                    .get(&newer.id)
                    .is_some_and(|id| id != &newer.id)
            {
                continue;
            }
            if statements_are_close(&older.content, &newer.content) {
                near.push(MemoryStatementPair {
                    newer: newer.id.clone(),
                    older: older.id.clone(),
                    newer_words: newer.content.clone(),
                    older_words: older.content.clone(),
                });
            }
        }
    }
    near.truncate(16);
    MemoryDuplicateReport { exact, near }
}

/// Moves a reported near-duplicate into the linked list. Returns false when
/// that pair was not waiting for confirmation.
pub fn confirm_memory_duplicate(
    report: &mut MemoryDuplicateReport,
    left: &FactId,
    right: &FactId,
) -> bool {
    let Some(index) = report.near.iter().position(|pair| {
        (&pair.newer == left && &pair.older == right)
            || (&pair.newer == right && &pair.older == left)
    }) else {
        return false;
    };
    let pair = report.near.remove(index);
    report.exact.push(pair);
    true
}

trait FactOrder {
    fn fact_order(&self) -> (Timestamp, &str);
}

impl FactOrder for Fact {
    fn fact_order(&self) -> (Timestamp, &str) {
        (self.created_at, self.id.0.as_str())
    }
}

/// Letters and numbers only, lowered, with single spaces.
pub fn statement_key(text: &str) -> String {
    let mut out = String::new();
    let mut spaced = false;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            spaced = false;
        } else if !out.is_empty() && !spaced {
            out.push(' ');
            spaced = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn statements_are_close(left: &str, right: &str) -> bool {
    if statement_key(left) == statement_key(right) {
        return false;
    }
    let left_tokens = content_tokens(left);
    let right_tokens = content_tokens(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return false;
    }
    let mut shared = 0;
    let mut used = vec![false; right_tokens.len()];
    for token in &left_tokens {
        if let Some(index) = right_tokens
            .iter()
            .enumerate()
            .position(|(index, other)| !used[index] && other == token)
        {
            used[index] = true;
            shared += 1;
        }
    }
    let union = left_tokens.len() + right_tokens.len() - shared;
    shared >= 3 && shared * 5 >= union * 3
}

fn content_tokens(text: &str) -> Vec<String> {
    statement_key(text)
        .split(' ')
        .filter(|word| !word.is_empty() && !is_light_word(word))
        .map(str::to_owned)
        .collect()
}

fn is_light_word(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "the"
            | "and"
            | "or"
            | "of"
            | "to"
            | "for"
            | "in"
            | "on"
            | "at"
            | "is"
            | "it"
            | "you"
            | "user"
            | "i"
            | "my"
            | "your"
            | "with"
            | "from"
            | "that"
            | "this"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EpisodeId;

    fn fact(id: &str, content: &str, names: &[&str], at: i64) -> Fact {
        Fact {
            id: FactId(id.into()),
            content: content.into(),
            entities: names.iter().map(|name| (*name).into()).collect(),
            source: None,
            replaces: None,
            extraction: None,
            created_at: Timestamp(at),
            valid_at: None,
            invalid_at: None,
            expired_at: None,
            stopped_at: None,
        }
    }

    fn record(fact: Fact, chat: Option<&str>) -> MemoryGraphRecord {
        MemoryGraphRecord {
            fact,
            chat: chat.map(str::to_owned),
        }
    }

    #[test]
    fn cross_chat_links_are_real_and_a_generic_you_is_not() {
        let mut school = fact(
            "fact:school",
            "Sam likes mint tea at school.",
            &["Sam", "you"],
            1,
        );
        school.source = Some(EpisodeId("episode:school".into()));
        let mut home = fact("fact:home", "Sam keeps mint tea at home.", &["Sam"], 2);
        home.source = Some(EpisodeId("episode:home".into()));
        let mut only_you = fact("fact:you", "You prefer quiet mornings.", &["you"], 3);
        only_you.source = Some(EpisodeId("episode:you".into()));
        let mut also_you = fact("fact:user", "You prefer quiet evenings.", &["user"], 4);
        also_you.source = Some(EpisodeId("episode:user".into()));
        let graph = memory_graph(
            &[
                record(home.clone(), Some("home")),
                record(only_you.clone(), Some("evenings")),
                record(school.clone(), Some("school")),
                record(also_you.clone(), None),
            ],
            &[],
        );
        assert!(graph.edges.iter().any(|edge| {
            let facts = matches!(
                (&edge.from, &edge.to),
                (
                    MemoryGraphNodeId::Detail(left),
                    MemoryGraphNodeId::Detail(right)
                ) if (left == &school.id && right == &home.id)
                    || (left == &home.id && right == &school.id)
            );
            facts
                && matches!(edge.kind, MemoryGraphEdgeKind::SharedSubject(ref names) if names.as_slice() == ["Sam".to_owned()])
        }));
        assert!(graph.nodes.iter().any(|node| node.label == "Chat school"));
        assert!(graph.nodes.iter().any(|node| node.label == "Chat home"));
        assert!(!graph.edges.iter().any(|edge| {
            matches!(
                (&edge.from, &edge.to),
                (
                    MemoryGraphNodeId::Detail(left),
                    MemoryGraphNodeId::Detail(right)
                ) if (left == &only_you.id && right == &also_you.id)
                    || (left == &also_you.id && right == &only_you.id)
            )
        }));
        let again = memory_graph(
            &[
                record(also_you, None),
                record(school, Some("school")),
                record(only_you, Some("evenings")),
                record(home, Some("home")),
            ],
            &[],
        );
        assert_eq!(again, graph);
        assert!(!graph.omitted);
    }

    #[test]
    fn exact_repeats_link_the_older_detail_and_near_wording_waits() {
        let older = fact("fact:old", "Sam likes mint tea.", &["Sam"], 1);
        let newer = fact("fact:new", "sam   likes mint tea!", &["Sam"], 5);
        let close = fact("fact:close", "Sam enjoys mint tea.", &["Sam"], 6);
        let other = fact("fact:other", "The school gate opens at three.", &[], 7);
        let before = vec![older.clone(), newer.clone(), close.clone(), other.clone()];
        let mut report = check_memory_duplicates(&before);
        assert_eq!(before.len(), 4);
        assert_eq!(report.exact.len(), 1);
        assert_eq!(report.exact[0].older, older.id);
        assert_eq!(report.exact[0].newer, newer.id);
        assert_eq!(report.near.len(), 1);
        assert_eq!(report.near[0].older, older.id);
        assert_eq!(report.near[0].newer, close.id);
        assert!(
            !report
                .exact
                .iter()
                .chain(&report.near)
                .any(|pair| { pair.newer == other.id || pair.older == other.id })
        );
        assert!(!confirm_memory_duplicate(&mut report, &other.id, &older.id));
        assert!(confirm_memory_duplicate(&mut report, &close.id, &older.id));
        assert_eq!(report.near.len(), 0);
        assert_eq!(report.exact.len(), 2);
        let graph = memory_graph(
            &[
                record(older.clone(), Some("school")),
                record(newer, Some("home")),
            ],
            &report.exact,
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| { matches!(edge.kind, MemoryGraphEdgeKind::EarlierVersion) })
        );
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| matches!(node.id, MemoryGraphNodeId::Detail(_)))
                .count(),
            2
        );
    }

    #[test]
    fn the_largest_picture_has_a_stable_layout_and_says_when_links_were_left_out() {
        let mut records = Vec::new();
        for index in 0..40 {
            let mut item = fact(
                &format!("fact:{index:02}"),
                &format!("Project note {index}"),
                &["Project"],
                index,
            );
            if index > 0 {
                item.replaces = Some(FactId(format!("fact:{:02}", index - 1)));
            }
            records.push(record(item, None));
        }
        let graph = memory_graph(&records, &[]);
        assert!(graph.omitted);
        assert_eq!(graph.edges.len(), MAX_MEMORY_CONNECTIONS);
        assert_eq!(graph.points.len(), graph.nodes.len());
        assert!(
            graph
                .points
                .iter()
                .all(|point| (0..=1000).contains(&point.x) && (0..=1000).contains(&point.y))
        );
        let laid_out = layout_memory_graph(&graph.nodes);
        assert_eq!(laid_out, graph.points);
        assert_eq!(layout_memory_graph(&graph.nodes), laid_out);
        let empty = memory_graph(&[], &[]);
        assert_eq!(empty.nodes.len(), 0);
        assert!(!empty.omitted);
        let alone = memory_graph(
            &[record(fact("fact:one", "Only this detail.", &[], 1), None)],
            &[],
        );
        assert_eq!(alone.nodes.len(), 1);
        assert_eq!(alone.edges.len(), 0);
        assert_eq!(alone.points, vec![MemoryPoint { x: 500, y: 500 }]);
    }
}

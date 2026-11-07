//! Bounded picture of saved links, and a local check for repeated wording.
//!
//! Layout is a pure function of the nodes it is given. It does not call a
//! model, a browser, or the network. Duplicate checks compare wording only:
//! an exact normalized repeat is linked as an earlier version, and both
//! records stay. Close wording is reported and is not joined here.

use super::{Fact, FactId, Timestamp, exploration::MAX_MEMORY_CONNECTIONS};

/// Session id for a note typed on the memory screen. It is not a conversation.
pub const MEMORY_REVIEW_SESSION: &str = "memory-review";

/// One saved detail and the conversation it came from.
///
/// `session` is the conversation id. `title` is the name already stored for
/// that conversation. Two conversations can share a title and stay two nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryFactChat {
    pub fact: FactId,
    pub session: String,
    pub title: String,
}

/// One saved detail, plus the conversation it came from when that chat is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryGraphRecord {
    pub fact: Fact,
    pub session: Option<String>,
    pub title: Option<String>,
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

impl MemoryGraph {
    /// A reading view of one detail and its direct connections. No inferred links
    /// are added, and the source graph remains available to return to the overview.
    #[must_use]
    pub fn around(&self, selected: &MemoryGraphNodeId) -> Self {
        let edges: Vec<_> = self
            .edges
            .iter()
            .filter(|edge| &edge.from == selected || &edge.to == selected)
            .cloned()
            .collect();
        let nodes: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| {
                &node.id == selected
                    || edges
                        .iter()
                        .any(|edge| edge.from == node.id || edge.to == node.id)
            })
            .cloned()
            .collect();
        let points = layout_memory_graph(&nodes, &edges, MEMORY_GRAPH_SQUARE);
        Self {
            nodes,
            edges,
            points,
            omitted: self.omitted,
        }
    }
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

/// Square picture: `memory_graph` lays out for this until the view measures its pane.
pub const MEMORY_GRAPH_SQUARE: u16 = 100;
const ASPECT_RANGE: std::ops::RangeInclusive<u16> = 50..=400;
const LAYOUT_STEPS: u16 = 360;
/// Thousandths kept clear at each edge so a node and its ring stay inside the drawing.
const LAYOUT_MARGIN: f64 = 70.0;
const MAX_AXIS_STRETCH: f64 = 1.8;
/// Pairs further apart than this many spacings stop pushing.
const REPULSION_REACH: f64 = 2.5;
const GRAVITY: f64 = 2.0;

/// Force-directed placement: links pull their ends together, every pair pushes apart,
/// and a weak pull to the middle keeps unlinked details beside the cluster.
///
/// `aspect_percent` is the drawing's width over its height, times 100. The result is in
/// thousandths of each axis, so a wide pane gets a wide layout instead of a stretched
/// circle. Deterministic: the same nodes, links and aspect always give the same points.
pub fn layout_memory_graph(
    nodes: &[MemoryGraphNode],
    edges: &[MemoryGraphEdge],
    aspect_percent: u16,
) -> Vec<MemoryPoint> {
    let count = nodes.len();
    if count <= 1 {
        return vec![MemoryPoint { x: 500, y: 500 }; count];
    }
    let aspect =
        f64::from(aspect_percent.clamp(*ASPECT_RANGE.start(), *ASPECT_RANGE.end())) / 100.0;
    let (width, height) = (aspect.sqrt(), 1.0 / aspect.sqrt());
    let links: Vec<(usize, usize)> = edges
        .iter()
        .filter_map(|edge| {
            let from = nodes.iter().position(|node| node.id == edge.from)?;
            let to = nodes.iter().position(|node| node.id == edge.to)?;
            (from != to).then_some((from, to))
        })
        .collect();
    #[expect(
        clippy::cast_precision_loss,
        reason = "a picture has at most a few dozen nodes"
    )]
    let n = count as f64;
    let spacing = (width * height / n).sqrt() * 0.9;
    // Golden-angle spiral: evenly spread, and a pure function of the node order.
    let mut spots: Vec<(f64, f64)> = (0..count)
        .map(|index| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "a picture has at most a few dozen nodes"
            )]
            let i = index as f64 + 0.5;
            let radius = (i / n).sqrt() * 0.45;
            let angle = i * 2.399_963_229_728_653;
            (radius * width * angle.cos(), radius * height * angle.sin())
        })
        .collect();
    let mut heat = width.max(height) * 0.1;
    let cooling = heat / f64::from(LAYOUT_STEPS);
    for _ in 0..LAYOUT_STEPS {
        let mut push = vec![(0.0_f64, 0.0_f64); count];
        for a in 0..count {
            for b in (a + 1)..count {
                let (dx, dy) = separation(spots[a], spots[b], a, b);
                let distance = dx.hypot(dy);
                // Short-range push: separate groups pack beside each other instead of
                // flying to the corners and shrinking every group when the picture fits.
                if distance > spacing * REPULSION_REACH {
                    continue;
                }
                let force = spacing * spacing / distance;
                let (fx, fy) = (dx / distance * force, dy / distance * force);
                push[a].0 += fx;
                push[a].1 += fy;
                push[b].0 -= fx;
                push[b].1 -= fy;
            }
        }
        for &(a, b) in &links {
            let (dx, dy) = separation(spots[a], spots[b], a, b);
            let distance = dx.hypot(dy);
            let force = distance * distance / spacing;
            let (fx, fy) = (dx / distance * force, dy / distance * force);
            push[a].0 -= fx;
            push[a].1 -= fy;
            push[b].0 += fx;
            push[b].1 += fy;
        }
        for (spot, force) in spots.iter_mut().zip(&mut push) {
            force.0 -= spot.0 * spacing * GRAVITY;
            force.1 -= spot.1 * spacing * GRAVITY * aspect;
            let length = force.0.hypot(force.1);
            if length > f64::EPSILON {
                let step = length.min(heat);
                spot.0 += force.0 / length * step;
                spot.1 += force.1 / length * step;
            }
        }
        heat = (heat - cooling).max(heat * 0.02);
    }
    fit(&spots, width, height)
}

/// Offset between two spots; coincident spots get a fixed, order-based nudge.
fn separation(a: (f64, f64), b: (f64, f64), first: usize, second: usize) -> (f64, f64) {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    if dx.hypot(dy) > 1e-9 {
        return (dx, dy);
    }
    #[expect(clippy::cast_precision_loss, reason = "node indices are small")]
    let angle = (first * 7 + second * 13) as f64;
    (angle.cos() * 1e-6, angle.sin() * 1e-6)
}

/// Scales the settled layout into the drawing's thousandths, keeping its proportions.
fn fit(spots: &[(f64, f64)], width: f64, height: f64) -> Vec<MemoryPoint> {
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &(x, y) in spots {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let usable = 1000.0 - 2.0 * LAYOUT_MARGIN;
    // Fill both axes, but never squash one more than this against the other, so a
    // long chain stays a line instead of becoming a smeared blob.
    let mut scale_x = width / (max_x - min_x).max(1e-9);
    let mut scale_y = height / (max_y - min_y).max(1e-9);
    scale_x = scale_x.min(scale_y * MAX_AXIS_STRETCH);
    scale_y = scale_y.min(scale_x * MAX_AXIS_STRETCH);
    let (mid_x, mid_y) = (f64::midpoint(min_x, max_x), f64::midpoint(min_y, max_y));
    spots
        .iter()
        .map(|&(x, y)| MemoryPoint {
            x: thousand(500.0 + (x - mid_x) * scale_x / width * usable),
            y: thousand(500.0 + (y - mid_y) * scale_y / height * usable),
        })
        .collect()
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

impl MemoryGraph {
    /// Positions of the nodes linked to `index`, each once, in node order.
    pub fn neighbors(&self, index: usize) -> Vec<usize> {
        let Some(node) = self.nodes.get(index) else {
            return Vec::new();
        };
        let mut found: Vec<usize> = self
            .edges
            .iter()
            .filter_map(|edge| {
                let other = if edge.from == node.id {
                    &edge.to
                } else if edge.to == node.id {
                    &edge.from
                } else {
                    return None;
                };
                self.nodes
                    .iter()
                    .position(|candidate| &candidate.id == other)
            })
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// Links touching each node, in node order: how central a detail is.
    pub fn degrees(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .map(|node| {
                self.edges
                    .iter()
                    .filter(|edge| edge.from == node.id || edge.to == node.id)
                    .count()
            })
            .collect()
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
        if let Some(session) = chat_session(record) {
            candidates.push(candidate(
                1,
                MemoryGraphNodeId::Detail(record.fact.id.clone()),
                MemoryGraphNodeId::Chat(session),
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
    let edges: Vec<_> = candidates.into_iter().map(|item| item.edge).collect();
    let points = layout_memory_graph(&nodes, &edges, MEMORY_GRAPH_SQUARE);
    MemoryGraph {
        edges,
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
        MemoryGraphNodeId::Chat(session) => chat_label(records, session),
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

fn review_bucket(session: &str) -> bool {
    session == MEMORY_REVIEW_SESSION
}

/// Conversation id for the picture. A note typed on the memory screen is not a chat.
fn chat_session(record: &MemoryGraphRecord) -> Option<String> {
    let session = record.session.as_deref()?.trim();
    if session.is_empty() || review_bucket(session) {
        return None;
    }
    Some(session.to_owned())
}

fn chat_label(records: &[MemoryGraphRecord], session: &str) -> String {
    let title = records.iter().find_map(|record| {
        let same = record
            .session
            .as_deref()
            .is_some_and(|id| id.trim() == session);
        let title = record.title.as_deref().unwrap_or("").trim();
        (same && !title.is_empty() && !review_bucket(title)).then(|| title.to_owned())
    });
    match title {
        Some(title) => format!("Chat {title}"),
        None => "Chat".to_owned(),
    }
}

/// True when one of these details already points at the other as an earlier version.
pub fn stored_version_link(facts: &[Fact], left: &FactId, right: &FactId) -> bool {
    facts.iter().any(|fact| {
        (fact.id == *left && fact.replaces.as_ref() == Some(right))
            || (fact.id == *right && fact.replaces.as_ref() == Some(left))
    })
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
            if stored_version_link(&ordered, &older.id, &newer.id) {
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
            session: chat.map(str::to_owned),
            title: chat.map(str::to_owned),
        }
    }

    #[test]
    fn focused_map_keeps_only_direct_evidence_and_preserves_the_overview() {
        let graph = memory_graph(
            &[
                record(fact("a", "Maya lives in Oslo", &["Maya", "Oslo"], 1), None),
                record(fact("b", "Maya likes tea", &["Maya"], 2), None),
                record(fact("c", "Oslo trip in May", &["Oslo"], 3), None),
                record(fact("d", "Garden needs water", &["Garden"], 4), None),
            ],
            &[],
        );
        let id = MemoryGraphNodeId::Detail(FactId("b".into()));
        let focused = graph.around(&id);
        assert_eq!(focused.nodes.len(), 2);
        assert_eq!(focused.edges.len(), 1);
        assert_eq!(focused.points.len(), focused.nodes.len());
        assert!(
            focused
                .edges
                .iter()
                .all(|edge| edge.from == id || edge.to == id)
        );
        assert_eq!(graph.nodes.len(), 4);
        let isolated = graph.around(&MemoryGraphNodeId::Detail(FactId("d".into())));
        assert_eq!(isolated.nodes.len(), 1);
        assert_eq!(isolated.edges.len(), 0);
        assert_eq!(
            graph
                .around(&MemoryGraphNodeId::Detail(FactId("missing".into())))
                .nodes
                .len(),
            0
        );
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
        assert_eq!(report.near, vec![]);
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
        let laid_out = layout_memory_graph(&graph.nodes, &graph.edges, MEMORY_GRAPH_SQUARE);
        assert_eq!(laid_out, graph.points);
        assert_eq!(
            layout_memory_graph(&graph.nodes, &graph.edges, MEMORY_GRAPH_SQUARE),
            laid_out
        );
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

    fn distance(a: MemoryPoint, b: MemoryPoint) -> f64 {
        f64::from(a.x - b.x).hypot(f64::from(a.y - b.y))
    }

    /// Two topics joined only through a chat, plus two details nobody links to.
    fn two_topics() -> MemoryGraph {
        let mut records = Vec::new();
        for (index, (topic, names)) in [
            ("Sam", &["Sam"][..]),
            ("Sam", &["Sam"]),
            ("Sam", &["Sam"]),
            ("Garden", &["Garden"]),
            ("Garden", &["Garden"]),
            ("Garden", &["Garden"]),
        ]
        .into_iter()
        .enumerate()
        {
            let chat = (index == 0 || index == 3).then_some("chat-bridge");
            records.push(record(
                fact(
                    &format!("fact:{index}"),
                    &format!("{topic} note {index}"),
                    names,
                    i64::try_from(index).unwrap(),
                ),
                chat,
            ));
        }
        records.push(record(fact("fact:lone-a", "Lone detail.", &[], 9), None));
        records.push(record(
            fact("fact:lone-b", "Another lone detail.", &[], 10),
            None,
        ));
        memory_graph(&records, &[])
    }

    #[test]
    fn linked_details_settle_together_and_nothing_overlaps() {
        let graph = two_topics();
        let id = |name: &str| {
            graph
                .nodes
                .iter()
                .position(|node| node.id == MemoryGraphNodeId::Detail(FactId(name.into())))
                .unwrap()
        };
        let at = |name: &str| graph.points[id(name)];
        let within = f64::midpoint(
            distance(at("fact:1"), at("fact:2")),
            distance(at("fact:4"), at("fact:5")),
        );
        let across = f64::midpoint(
            distance(at("fact:1"), at("fact:4")),
            distance(at("fact:2"), at("fact:5")),
        );
        assert!(within < across, "within {within} across {across}");
        for (a, first) in graph.points.iter().enumerate() {
            assert!((0..=1000).contains(&first.x) && (0..=1000).contains(&first.y));
            for second in &graph.points[a + 1..] {
                assert!(distance(*first, *second) > 40.0, "{first:?} {second:?}");
            }
        }
        let degrees = graph.degrees();
        assert_eq!(degrees[id("fact:lone-a")], 0);
        assert!(degrees[id("fact:0")] >= 3);
        let neighbors = graph.neighbors(id("fact:0"));
        assert!(neighbors.contains(&id("fact:1")));
        assert!(!neighbors.contains(&id("fact:4")));
        assert_eq!(graph.neighbors(graph.nodes.len()), Vec::<usize>::new());
    }

    #[test]
    fn a_wide_pane_gets_a_wide_layout_not_a_stretched_circle() {
        let graph = two_topics();
        let spread = |points: &[MemoryPoint], axis: fn(&MemoryPoint) -> i32| {
            let values: Vec<i32> = points.iter().map(axis).collect();
            values.iter().max().unwrap() - values.iter().min().unwrap()
        };
        let wide = layout_memory_graph(&graph.nodes, &graph.edges, 250);
        assert_eq!(wide, layout_memory_graph(&graph.nodes, &graph.edges, 250));
        // In thousandths of each axis a wide layout fills both; in real proportions it is wider.
        assert!(
            spread(&wide, |p| p.x) > 700 && spread(&wide, |p| p.y) > 500,
            "{wide:?}"
        );
        assert_eq!(
            layout_memory_graph(&graph.nodes, &graph.edges, 9_999),
            layout_memory_graph(&graph.nodes, &graph.edges, 400)
        );
    }

    #[test]
    fn matching_chat_titles_stay_apart_and_a_review_note_is_not_a_chat() {
        let school = fact("fact:school", "Sam likes mint tea at school.", &["Sam"], 1);
        let home = fact("fact:home", "Sam keeps mint tea at home.", &["Sam"], 2);
        let note = fact("fact:note", "I typed this on the memory screen.", &[], 3);
        let graph = memory_graph(
            &[
                MemoryGraphRecord {
                    fact: school,
                    session: Some("chat-school-a".into()),
                    title: Some("School pickup".into()),
                },
                MemoryGraphRecord {
                    fact: home,
                    session: Some("chat-school-b".into()),
                    title: Some("School pickup".into()),
                },
                MemoryGraphRecord {
                    fact: note,
                    session: Some(MEMORY_REVIEW_SESSION.into()),
                    title: Some(MEMORY_REVIEW_SESSION.into()),
                },
            ],
            &[],
        );
        let chats: Vec<_> = graph
            .nodes
            .iter()
            .filter(|node| matches!(node.id, MemoryGraphNodeId::Chat(_)))
            .collect();
        assert_eq!(chats.len(), 2);
        assert!(chats.iter().all(|node| node.label == "Chat School pickup"));
        assert!(chats.iter().any(|node| {
            matches!(&node.id, MemoryGraphNodeId::Chat(session) if session == "chat-school-a")
        }));
        assert!(chats.iter().any(|node| {
            matches!(&node.id, MemoryGraphNodeId::Chat(session) if session == "chat-school-b")
        }));
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| !node.label.contains(MEMORY_REVIEW_SESSION))
        );
        assert!(
            graph
                .edges
                .iter()
                .all(|edge| !matches!(&edge.to, MemoryGraphNodeId::Chat(session) if session == MEMORY_REVIEW_SESSION))
        );
    }

    #[test]
    fn a_stored_version_link_is_not_listed_as_waiting() {
        let older = fact("fact:old", "Sam likes mint tea.", &["Sam"], 1);
        let mut close = fact("fact:close", "Sam enjoys mint tea.", &["Sam"], 6);
        close.replaces = Some(older.id.clone());
        let report = check_memory_duplicates(&[older.clone(), close.clone()]);
        assert_eq!(report.near, vec![]);
        assert!(stored_version_link(
            &[older.clone(), close],
            &FactId("fact:close".into()),
            &older.id
        ));
        let graph = memory_graph(
            &[record(older, None), {
                let mut linked = record(
                    fact("fact:close", "Sam enjoys mint tea.", &["Sam"], 6),
                    None,
                );
                linked.fact.replaces = Some(FactId("fact:old".into()));
                linked
            }],
            &[],
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| matches!(edge.kind, MemoryGraphEdgeKind::EarlierVersion))
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
}

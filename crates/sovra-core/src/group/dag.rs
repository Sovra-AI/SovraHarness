//! Causal ordering for signed, hash-linked logs (shared knowledge uses it).
//!
//! Each node names its own previous operation (`prev`, same author, `seq - 1`) and any
//! other operations it had seen (`deps`). Given any set of nodes this computes, the
//! same way on every computer: which are forked, which wait for a missing parent,
//! a deterministic causal order, and each node's full ancestry (to tell "saw it"
//! from "concurrent").

use std::collections::{BTreeMap, BTreeSet};

use super::keys::{DeviceId, OpHash};
use crate::Timestamp;

/// What the ordering needs to know about one node.
#[derive(Debug, Clone)]
pub(crate) struct DagNode {
    pub(crate) author: DeviceId,
    pub(crate) seq: u64,
    pub(crate) prev: Option<OpHash>,
    pub(crate) deps: Vec<OpHash>,
    pub(crate) at: Timestamp,
}

impl DagNode {
    fn parents(&self) -> impl Iterator<Item = OpHash> + '_ {
        self.prev.iter().copied().chain(self.deps.iter().copied())
    }
}

/// Why a node was set aside by the ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DagReject {
    BrokenChain,
    Forked,
    ParentRejected,
}

#[derive(Debug, Default)]
pub(crate) struct DagOrder {
    /// Ready nodes, parents before children; ties by author clock then hash.
    pub(crate) order: Vec<OpHash>,
    pub(crate) ancestors: BTreeMap<OpHash, BTreeSet<OpHash>>,
    pub(crate) pending: BTreeSet<OpHash>,
    pub(crate) rejected: BTreeMap<OpHash, DagReject>,
    pub(crate) forked: BTreeSet<DeviceId>,
    /// Latest ordered node per author, to chain the next one.
    pub(crate) tips: BTreeMap<DeviceId, (u64, OpHash)>,
}

pub(crate) fn order(nodes: &BTreeMap<OpHash, DagNode>) -> DagOrder {
    let mut out = DagOrder::default();

    for (hash, node) in nodes {
        if node.seq == 0 || (node.seq == 1) != node.prev.is_none() {
            out.rejected.insert(*hash, DagReject::BrokenChain);
        }
    }

    let mut seen: BTreeMap<(DeviceId, u64), OpHash> = BTreeMap::new();
    let mut fork_from: BTreeMap<DeviceId, u64> = BTreeMap::new();
    for (hash, node) in nodes {
        if out.rejected.contains_key(hash) {
            continue;
        }
        if seen.insert((node.author, node.seq), *hash).is_some() {
            let first = fork_from.entry(node.author).or_insert(node.seq);
            *first = (*first).min(node.seq);
        }
    }
    for (hash, node) in nodes {
        if fork_from
            .get(&node.author)
            .is_some_and(|&from| node.seq >= from)
        {
            out.rejected.entry(*hash).or_insert(DagReject::Forked);
        }
    }
    out.forked = fork_from.into_keys().collect();

    let mut waiting: BTreeMap<OpHash, usize> = BTreeMap::new();
    let mut children: BTreeMap<OpHash, Vec<OpHash>> = BTreeMap::new();
    let mut ready: BTreeSet<(Timestamp, OpHash)> = BTreeSet::new();
    for (hash, node) in nodes {
        if out.rejected.contains_key(hash) {
            continue;
        }
        let parents: BTreeSet<OpHash> = node.parents().collect();
        if parents
            .iter()
            .any(|parent| out.rejected.contains_key(parent))
        {
            out.rejected.insert(*hash, DagReject::ParentRejected);
            continue;
        }
        for parent in &parents {
            if nodes.contains_key(parent) {
                children.entry(*parent).or_default().push(*hash);
            }
        }
        if parents.is_empty() {
            ready.insert((node.at, *hash));
        } else {
            waiting.insert(*hash, parents.len());
        }
    }

    while let Some((_, hash)) = ready.pop_first() {
        let node = &nodes[&hash];
        if let Some(prev) = node.prev {
            let parent = &nodes[&prev];
            if parent.author != node.author || parent.seq + 1 != node.seq {
                out.rejected.insert(hash, DagReject::BrokenChain);
                let mut stack = vec![hash];
                while let Some(gone) = stack.pop() {
                    for child in children.get(&gone).into_iter().flatten() {
                        if waiting.remove(child).is_some() {
                            out.rejected.insert(*child, DagReject::ParentRejected);
                            stack.push(*child);
                        }
                    }
                }
                continue;
            }
        }
        let mut ancestry = BTreeSet::new();
        for parent in node.parents() {
            ancestry.insert(parent);
            if let Some(grand) = out.ancestors.get(&parent) {
                ancestry.extend(grand.iter().copied());
            }
        }
        out.ancestors.insert(hash, ancestry);
        out.order.push(hash);
        let tip = out.tips.entry(node.author).or_insert((node.seq, hash));
        if node.seq >= tip.0 {
            *tip = (node.seq, hash);
        }
        for child in children.get(&hash).into_iter().flatten() {
            if let Some(count) = waiting.get_mut(child) {
                *count -= 1;
                if *count == 0 {
                    waiting.remove(child);
                    ready.insert((nodes[child].at, *child));
                }
            }
        }
    }
    out.pending = waiting.into_keys().collect();
    out
}

impl DagOrder {
    /// True when neither node is an ancestor of the other.
    pub(crate) fn concurrent(&self, a: &OpHash, b: &OpHash) -> bool {
        let a_saw_b = self.ancestors.get(a).is_some_and(|set| set.contains(b));
        let b_saw_a = self.ancestors.get(b).is_some_and(|set| set.contains(a));
        a != b && !a_saw_b && !b_saw_a
    }

    /// Ordered nodes nothing else in `keep` builds on.
    pub(crate) fn heads(
        &self,
        nodes: &BTreeMap<OpHash, DagNode>,
        keep: &BTreeSet<OpHash>,
    ) -> Vec<OpHash> {
        let parents: BTreeSet<OpHash> =
            keep.iter().flat_map(|hash| nodes[hash].parents()).collect();
        self.order
            .iter()
            .copied()
            .filter(|hash| keep.contains(hash) && !parents.contains(hash))
            .collect()
    }
}

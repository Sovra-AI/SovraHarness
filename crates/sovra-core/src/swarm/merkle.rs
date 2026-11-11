//! A Merkle tree over the audit ledger, exactly as RFC 9162 §2.1 defines it, with
//! BLAKE3 in place of SHA-256.
//!
//! Leaves and inner nodes get different prefixes (`0x00`, `0x01`) so a leaf can never
//! be passed off as a subtree. An **inclusion proof** shows one entry is in a tree of a
//! given size; a **consistency proof** shows a bigger tree is an extension of a smaller
//! one, which is how a witness proves nobody rewrote history between two checkpoints.

use crate::group::ContentHash;

const LEAF: u8 = 0x00;
const NODE: u8 = 0x01;

/// The hash of one leaf's bytes.
pub fn leaf_hash(data: &[u8]) -> ContentHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[LEAF]);
    hasher.update(data);
    ContentHash(*hasher.finalize().as_bytes())
}

fn node_hash(left: &ContentHash, right: &ContentHash) -> ContentHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[NODE]);
    hasher.update(left.as_bytes());
    hasher.update(right.as_bytes());
    ContentHash(*hasher.finalize().as_bytes())
}

/// The largest power of two strictly below `n` (`n >= 2`).
fn split(n: usize) -> usize {
    debug_assert!(n >= 2, "only trees of two or more leaves split");
    let mut k = 1;
    while k << 1 < n {
        k <<= 1;
    }
    k
}

/// The root of the tree whose leaves have these hashes. The empty tree's root is the
/// hash of no bytes.
pub fn root(leaves: &[ContentHash]) -> ContentHash {
    match leaves.len() {
        0 => ContentHash(*blake3::hash(&[]).as_bytes()),
        1 => leaves[0],
        n => {
            let k = split(n);
            node_hash(&root(&leaves[..k]), &root(&leaves[k..]))
        }
    }
}

/// The audit path for leaf `index`, or `None` when it is outside the tree.
pub fn inclusion_proof(index: usize, leaves: &[ContentHash]) -> Option<Vec<ContentHash>> {
    (index < leaves.len()).then(|| path(index, leaves))
}

fn path(index: usize, leaves: &[ContentHash]) -> Vec<ContentHash> {
    if leaves.len() <= 1 {
        return Vec::new();
    }
    let k = split(leaves.len());
    if index < k {
        let mut proof = path(index, &leaves[..k]);
        proof.push(root(&leaves[k..]));
        proof
    } else {
        let mut proof = path(index - k, &leaves[k..]);
        proof.push(root(&leaves[..k]));
        proof
    }
}

/// Checks that `leaf` sits at `index` in a tree of `size` leaves with this `root`
/// (RFC 9162 §2.1.3.2).
pub fn verify_inclusion(
    leaf: &ContentHash,
    index: u64,
    size: u64,
    proof: &[ContentHash],
    root: &ContentHash,
) -> bool {
    if index >= size {
        return false;
    }
    let (mut fn_, mut sn) = (index, size - 1);
    let mut r = *leaf;
    for p in proof {
        if sn == 0 {
            return false;
        }
        if fn_ & 1 == 1 || fn_ == sn {
            r = node_hash(p, &r);
            while fn_ & 1 == 0 && fn_ != 0 {
                fn_ >>= 1;
                sn >>= 1;
            }
        } else {
            r = node_hash(&r, p);
        }
        fn_ >>= 1;
        sn >>= 1;
    }
    sn == 0 && r == *root
}

/// Proof that the tree's first `old_size` leaves are unchanged in the whole tree, or
/// `None` when `old_size` is larger than the tree.
pub fn consistency_proof(old_size: usize, leaves: &[ContentHash]) -> Option<Vec<ContentHash>> {
    if old_size > leaves.len() {
        return None;
    }
    if old_size == 0 || old_size == leaves.len() {
        return Some(Vec::new());
    }
    Some(subproof(old_size, leaves, true))
}

fn subproof(m: usize, leaves: &[ContentHash], whole: bool) -> Vec<ContentHash> {
    let n = leaves.len();
    if m == n {
        return if whole {
            Vec::new()
        } else {
            vec![root(leaves)]
        };
    }
    let k = split(n);
    if m <= k {
        let mut proof = subproof(m, &leaves[..k], whole);
        proof.push(root(&leaves[k..]));
        proof
    } else {
        let mut proof = subproof(m - k, &leaves[k..], false);
        proof.push(root(&leaves[..k]));
        proof
    }
}

/// Checks that a tree of `new_size` with `new_root` extends one of `old_size` with
/// `old_root` (RFC 9162 §2.1.4.2).
pub fn verify_consistency(
    old_size: u64,
    new_size: u64,
    old_root: &ContentHash,
    new_root: &ContentHash,
    proof: &[ContentHash],
) -> bool {
    if old_size > new_size {
        return false;
    }
    if old_size == new_size {
        return proof.is_empty() && old_root == new_root;
    }
    if old_size == 0 {
        return proof.is_empty();
    }
    let mut proof: Vec<ContentHash> = proof.to_vec();
    if proof.is_empty() {
        return false;
    }
    if old_size.is_power_of_two() {
        proof.insert(0, *old_root);
    }
    let (mut fn_, mut sn) = (old_size - 1, new_size - 1);
    while fn_ & 1 == 1 {
        fn_ >>= 1;
        sn >>= 1;
    }
    let (mut fr, mut sr) = (proof[0], proof[0]);
    for c in &proof[1..] {
        if sn == 0 {
            return false;
        }
        if fn_ & 1 == 1 || fn_ == sn {
            fr = node_hash(c, &fr);
            sr = node_hash(c, &sr);
            while fn_ & 1 == 0 && fn_ != 0 {
                fn_ >>= 1;
                sn >>= 1;
            }
        } else {
            sr = node_hash(&sr, c);
        }
        fn_ >>= 1;
        sn >>= 1;
    }
    fr == *old_root && sr == *new_root && sn == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(n: usize) -> Vec<ContentHash> {
        (0..n).map(|i| leaf_hash(&i.to_le_bytes())).collect()
    }

    #[test]
    fn every_leaf_of_every_small_tree_proves_inclusion() {
        for n in 1..=33 {
            let tree = leaves(n);
            let top = root(&tree);
            for (i, leaf) in tree.iter().enumerate() {
                let proof = inclusion_proof(i, &tree).expect("index is inside the tree");
                assert!(
                    verify_inclusion(leaf, i as u64, n as u64, &proof, &top),
                    "{i}/{n}"
                );
                // The same proof never proves a different leaf or position.
                assert!(
                    !verify_inclusion(&tree[(i + 1) % n], i as u64, n as u64, &proof, &top)
                        || n == 1
                );
                if n > 1 {
                    assert!(!verify_inclusion(
                        leaf,
                        ((i + 1) % n) as u64,
                        n as u64,
                        &proof,
                        &top
                    ));
                }
            }
        }
    }

    #[test]
    fn every_prefix_of_every_small_tree_is_consistent() {
        for n in 1..=33 {
            let tree = leaves(n);
            let new_root = root(&tree);
            for m in 0..=n {
                let old_root = root(&tree[..m]);
                let proof = consistency_proof(m, &tree).expect("prefix fits");
                assert!(
                    verify_consistency(m as u64, n as u64, &old_root, &new_root, &proof),
                    "{m}->{n}"
                );
            }
        }
    }

    #[test]
    fn a_rewritten_history_fails_consistency() {
        let tree = leaves(10);
        let mut forged = tree.clone();
        forged[3] = leaf_hash(b"rewritten");
        let proof = consistency_proof(6, &forged).expect("prefix fits");
        assert!(!verify_consistency(
            6,
            10,
            &root(&tree[..6]),
            &root(&forged),
            &proof
        ));
    }

    #[test]
    fn leaves_and_nodes_cannot_be_confused() {
        let tree = leaves(2);
        let inner = node_hash(&tree[0], &tree[1]);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(tree[0].as_bytes());
        bytes.extend_from_slice(tree[1].as_bytes());
        assert_ne!(leaf_hash(&bytes), inner);
    }
}

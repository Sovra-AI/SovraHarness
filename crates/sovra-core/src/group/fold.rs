//! Folding membership operations into a roster.
//!
//! Any set of operations, in any order and with duplicates, folds to the same
//! [`GroupState`] on every computer. The rules, in the order they are applied:
//!
//! 1. **Shape.** Wrong group, bad signature, or a broken own-chain (`seq`/`prev`) is
//!    rejected outright.
//! 2. **Forks.** A device that signs two different operations at the same `seq` is
//!    forked: everything it signed from that point on is rejected.
//! 3. **Causality.** An operation waits (pending) until every parent is present, and
//!    is rejected if a parent was rejected. Ready operations are ordered causally,
//!    ties broken by the author's clock and then the hash, so the order is fixed.
//! 4. **Authority.** Operations apply in that order. Each must come from a current
//!    device whose person's role allows it, and descend from genesis.
//! 5. **Strong removal.** Once someone is removed, anything they did concurrently (the
//!    remover had not seen it, and it had not seen the removal) is void. Removals are
//!    never voided this way; when two admins remove each other concurrently, the first
//!    in the fixed order applies and the second fails because its author is gone, so
//!    the group always keeps an admin.
//! 6. **Remove wins.** An add that is concurrent with a removal of the same person or
//!    device is void. Adding someone back requires having seen the removal.
//! 7. **Never zero admins.** Removing or demoting the last admin is rejected.
//!
//! Steps 4–6 depend on each other, so they iterate to a fixed point. The void set only
//! grows, which keeps the loop finite and the result deterministic.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::keys::{DeviceId, GroupId, OpHash, PersonId, SignatureVerifier};
use super::membership::{MembershipBody, MembershipOp, Role, UnsignedOp};
use crate::Timestamp;

/// Longest group, person or device name, in characters.
pub const MAX_NAME_CHARS: usize = 80;

/// Why an operation is not part of the roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    WrongGroup,
    BadSignature,
    BrokenChain,
    Forked,
    ParentRejected,
    InvalidGenesis,
    NotFromGenesis,
    UnknownAuthor,
    NotAllowed,
    AlreadyPresent,
    UnknownTarget,
    InvalidName,
    WouldLeaveNoAdmin,
    /// The author was removed by an operation that had not seen this one.
    AuthorRemoved,
    /// A removal of the same person or device happened concurrently.
    RemovedConcurrently,
}

/// One computer in the roster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEntry {
    pub name: String,
    pub added_at: Timestamp,
}

/// One person in the roster, with their current computers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub person: PersonId,
    pub name: String,
    pub role: Role,
    pub devices: BTreeMap<DeviceId, DeviceEntry>,
}

/// The roster every computer agrees on, plus what did not make it in and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupState {
    pub group: GroupId,
    pub name: String,
    pub founder: Option<PersonId>,
    pub people: BTreeMap<PersonId, Member>,
    pub removed_people: BTreeSet<PersonId>,
    pub removed_devices: BTreeSet<DeviceId>,
    /// Computers that were in the group and are not now, with whose they were and
    /// that person's name, so their past contributions stay attributable.
    pub former_devices: BTreeMap<DeviceId, (PersonId, String)>,
    /// Applied operations, in fold order.
    pub applied: Vec<OpHash>,
    pub rejected: BTreeMap<OpHash, Rejection>,
    /// Operations waiting for a parent this computer has not received yet.
    pub pending: BTreeSet<OpHash>,
    pub forked: BTreeSet<DeviceId>,
    /// Applied operations nothing else builds on: the `deps` of the next operation.
    pub heads: Vec<OpHash>,
    /// The latest structurally valid operation per device, for chaining the next one.
    chain_tips: BTreeMap<DeviceId, (u64, OpHash)>,
}

impl GroupState {
    fn empty(group: GroupId) -> Self {
        Self {
            group,
            name: String::new(),
            founder: None,
            people: BTreeMap::new(),
            removed_people: BTreeSet::new(),
            removed_devices: BTreeSet::new(),
            former_devices: BTreeMap::new(),
            applied: Vec::new(),
            rejected: BTreeMap::new(),
            pending: BTreeSet::new(),
            forked: BTreeSet::new(),
            heads: Vec::new(),
            chain_tips: BTreeMap::new(),
        }
    }

    /// The person a current device belongs to.
    pub fn owner_of(&self, device: &DeviceId) -> Option<&Member> {
        self.people
            .values()
            .find(|member| member.devices.contains_key(device))
    }

    pub fn role_of(&self, device: &DeviceId) -> Option<Role> {
        self.owner_of(device).map(|member| member.role)
    }

    /// True when the device is in the roster now: the transport admits only these.
    pub fn admits(&self, device: &DeviceId) -> bool {
        self.owner_of(device).is_some()
    }

    pub fn admin_count(&self) -> usize {
        self.people
            .values()
            .filter(|member| member.role.manages())
            .count()
    }

    /// The UI asks for a second admin so nobody is ever locked out.
    pub fn needs_second_admin(&self) -> bool {
        self.founder.is_some() && self.admin_count() < 2
    }

    /// The next operation this device should sign: chained to its own last
    /// operation and depending on everything else it has applied.
    pub fn draft(&self, author: DeviceId, at: Timestamp, body: MembershipBody) -> UnsignedOp {
        let (seq, prev) = match self.chain_tips.get(&author) {
            Some(&(seq, hash)) => (seq + 1, Some(hash)),
            None => (1, None),
        };
        let deps = self
            .heads
            .iter()
            .copied()
            .filter(|head| Some(*head) != prev)
            .collect();
        UnsignedOp {
            group: self.group,
            author,
            seq,
            prev,
            deps,
            at,
            body,
        }
    }
}

/// Folds any set of operations into the roster for `group`.
pub fn fold<'a>(
    group: GroupId,
    ops: impl IntoIterator<Item = &'a MembershipOp>,
    verifier: &dyn SignatureVerifier,
) -> GroupState {
    let mut state = GroupState::empty(group);
    let mut candidates: BTreeMap<OpHash, &MembershipOp> = BTreeMap::new();
    for op in ops {
        candidates.entry(op.hash()).or_insert(op);
    }

    // 1. Shape.
    for (hash, op) in &candidates {
        let unsigned = &op.unsigned;
        let rejection = if unsigned.group != group {
            Some(Rejection::WrongGroup)
        } else if !verifier.verify(&unsigned.author, &op.signing_bytes(), &op.signature) {
            Some(Rejection::BadSignature)
        } else if unsigned.seq == 0 || (unsigned.seq == 1) != unsigned.prev.is_none() {
            Some(Rejection::BrokenChain)
        } else {
            None
        };
        if let Some(rejection) = rejection {
            state.rejected.insert(*hash, rejection);
        }
    }

    // 2. Forks: two distinct operations at one (author, seq).
    let mut seen: BTreeMap<(DeviceId, u64), OpHash> = BTreeMap::new();
    let mut fork_from: BTreeMap<DeviceId, u64> = BTreeMap::new();
    for (hash, op) in &candidates {
        if state.rejected.contains_key(hash) {
            continue;
        }
        let key = (op.author(), op.unsigned.seq);
        if seen.insert(key, *hash).is_some() {
            let first = fork_from.entry(key.0).or_insert(key.1);
            *first = (*first).min(key.1);
        }
    }
    for (hash, op) in &candidates {
        if fork_from
            .get(&op.author())
            .is_some_and(|&from| op.unsigned.seq >= from)
        {
            state.rejected.entry(*hash).or_insert(Rejection::Forked);
        }
    }
    state.forked = fork_from.into_keys().collect();

    // 3. Causal order (Kahn), deterministic ties.
    let order = causal_order(&candidates, &mut state);
    let ancestors = ancestor_sets(&order, &candidates);
    for hash in &order {
        let op = candidates[hash];
        let tip = state.chain_tips.entry(op.author()).or_insert((0, *hash));
        if op.unsigned.seq >= tip.0 {
            *tip = (op.unsigned.seq, *hash);
        }
    }

    // 4–7. Apply, then void by strong removal / remove-wins, to a fixed point.
    let mut void: BTreeMap<OpHash, Rejection> = BTreeMap::new();
    loop {
        // Authority rejections are recomputed every pass (they depend on what is void);
        // only removal-based voids accumulate, so the loop is monotonic and finite.
        let applied = apply_in_order(group, &order, &candidates, &ancestors, &void);
        let mut grew = false;
        for removal in &applied.removals {
            for hash in &order {
                if *hash == removal.op || void.contains_key(hash) {
                    continue;
                }
                let op = candidates[hash];
                let seen_by_removal = ancestors[&removal.op].contains(hash);
                let verdict = removal_verdict(
                    op,
                    removal,
                    &applied.device_owner,
                    seen_by_removal,
                    &ancestors[hash],
                );
                if let Some(rejection) = verdict {
                    void.insert(*hash, rejection);
                    grew = true;
                }
            }
        }
        if !grew {
            let mut finished = applied.state;
            finished.rejected.extend(state.rejected);
            finished.rejected.extend(void);
            finished.rejected.extend(applied.rejected);
            finished.pending = state.pending;
            finished.forked = state.forked;
            finished.chain_tips = state.chain_tips;
            finished.heads = heads(&finished.applied, &candidates);
            return finished;
        }
    }
}

/// Topologically orders every operation whose parents are all present and valid.
fn causal_order(
    candidates: &BTreeMap<OpHash, &MembershipOp>,
    state: &mut GroupState,
) -> Vec<OpHash> {
    let mut waiting: BTreeMap<OpHash, usize> = BTreeMap::new();
    let mut children: BTreeMap<OpHash, Vec<OpHash>> = BTreeMap::new();
    let mut ready: BTreeSet<(Timestamp, OpHash)> = BTreeSet::new();

    for (hash, op) in candidates {
        if state.rejected.contains_key(hash) {
            continue;
        }
        let parents: BTreeSet<OpHash> = op.parents().collect();
        let mut outstanding = 0;
        for parent in &parents {
            if state.rejected.contains_key(parent) {
                outstanding = usize::MAX;
                break;
            }
            if !candidates.contains_key(parent) {
                // A missing parent keeps the op pending; count it so it never becomes ready.
                outstanding += 1;
                continue;
            }
            outstanding += 1;
            children.entry(*parent).or_default().push(*hash);
        }
        if outstanding == usize::MAX {
            state.rejected.insert(*hash, Rejection::ParentRejected);
        } else if outstanding == 0 {
            ready.insert((op.unsigned.at, *hash));
        } else {
            waiting.insert(*hash, outstanding);
        }
    }

    let mut order = Vec::new();
    while let Some(next) = ready.pop_first() {
        let hash = next.1;
        let op = candidates[&hash];
        if let Some(prev) = op.unsigned.prev {
            let parent = candidates[&prev];
            if parent.author() != op.author() || parent.unsigned.seq + 1 != op.unsigned.seq {
                state.rejected.insert(hash, Rejection::BrokenChain);
                reject_descendants(hash, &children, &mut waiting, state);
                continue;
            }
        }
        order.push(hash);
        for child in children.get(&hash).into_iter().flatten() {
            if let Some(count) = waiting.get_mut(child) {
                *count -= 1;
                if *count == 0 {
                    waiting.remove(child);
                    ready.insert((candidates[child].unsigned.at, *child));
                }
            }
        }
    }
    state.pending = waiting.into_keys().collect();
    order
}

fn reject_descendants(
    root: OpHash,
    children: &BTreeMap<OpHash, Vec<OpHash>>,
    waiting: &mut BTreeMap<OpHash, usize>,
    state: &mut GroupState,
) {
    let mut stack = vec![root];
    while let Some(hash) = stack.pop() {
        for child in children.get(&hash).into_iter().flatten() {
            if waiting.remove(child).is_some() {
                state.rejected.insert(*child, Rejection::ParentRejected);
                stack.push(*child);
            }
        }
    }
}

fn ancestor_sets(
    order: &[OpHash],
    candidates: &BTreeMap<OpHash, &MembershipOp>,
) -> BTreeMap<OpHash, BTreeSet<OpHash>> {
    let mut sets: BTreeMap<OpHash, BTreeSet<OpHash>> = BTreeMap::new();
    for hash in order {
        let mut set = BTreeSet::new();
        for parent in candidates[hash].parents() {
            set.insert(parent);
            if let Some(grand) = sets.get(&parent) {
                set.extend(grand.iter().copied());
            }
        }
        sets.insert(*hash, set);
    }
    sets
}

fn heads(applied: &[OpHash], candidates: &BTreeMap<OpHash, &MembershipOp>) -> Vec<OpHash> {
    let parents: BTreeSet<OpHash> = applied
        .iter()
        .flat_map(|hash| candidates[hash].parents())
        .collect();
    applied
        .iter()
        .copied()
        .filter(|hash| !parents.contains(hash))
        .collect()
}

#[derive(Debug, Clone, Copy)]
enum Target {
    Person(PersonId),
    Device(DeviceId),
}

#[derive(Debug, Clone, Copy)]
struct Removal {
    op: OpHash,
    target: Target,
}

struct Applied {
    state: GroupState,
    rejected: BTreeMap<OpHash, Rejection>,
    removals: Vec<Removal>,
    /// Every device ever added, to its person (removed ones included).
    device_owner: BTreeMap<DeviceId, PersonId>,
}

/// Decides whether a removal voids `op` (strong removal, then remove-wins).
fn removal_verdict(
    op: &MembershipOp,
    removal: &Removal,
    device_owner: &BTreeMap<DeviceId, PersonId>,
    seen_by_removal: bool,
    op_ancestors: &BTreeSet<OpHash>,
) -> Option<Rejection> {
    let author_person = device_owner.get(&op.author()).copied();
    let authored_by_target = match removal.target {
        Target::Device(device) => op.author() == device,
        Target::Person(person) => author_person == Some(person),
    };
    // Neither saw the other. Operations causally *after* a removal are left to the
    // authority check, so someone added back can act again.
    let concurrent = !seen_by_removal && !op_ancestors.contains(&removal.op);
    if !concurrent {
        return None;
    }
    let is_removal = matches!(
        op.body(),
        MembershipBody::RemoveDevice { .. } | MembershipBody::RemovePerson { .. }
    );
    if authored_by_target && !is_removal {
        return Some(Rejection::AuthorRemoved);
    }
    let adds_target = match (op.body(), removal.target) {
        (
            MembershipBody::AddPerson { person, .. } | MembershipBody::AddDevice { person, .. },
            Target::Person(removed),
        ) => *person == removed,
        (MembershipBody::AddDevice { device, .. }, Target::Device(removed)) => *device == removed,
        _ => false,
    };
    adds_target.then_some(Rejection::RemovedConcurrently)
}

fn valid_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= MAX_NAME_CHARS
}

/// One pass of rules 4 and 7 over the causal order, skipping already-void operations.
fn apply_in_order(
    group: GroupId,
    order: &[OpHash],
    candidates: &BTreeMap<OpHash, &MembershipOp>,
    ancestors: &BTreeMap<OpHash, BTreeSet<OpHash>>,
    void: &BTreeMap<OpHash, Rejection>,
) -> Applied {
    let mut out = Applied {
        state: GroupState::empty(group),
        rejected: BTreeMap::new(),
        removals: Vec::new(),
        device_owner: BTreeMap::new(),
    };
    let mut genesis: Option<OpHash> = None;

    for hash in order {
        if void.contains_key(hash) {
            continue;
        }
        let op = candidates[hash];
        let result = match genesis {
            None => apply_genesis(group, op, &mut out),
            Some(root) if !ancestors[hash].contains(&root) => Err(Rejection::NotFromGenesis),
            Some(_) => apply_change(op, *hash, &mut out),
        };
        match result {
            Ok(()) => {
                if genesis.is_none() {
                    genesis = Some(*hash);
                }
                out.state.applied.push(*hash);
            }
            Err(rejection) => {
                out.rejected.insert(*hash, rejection);
            }
        }
    }
    out
}

fn apply_genesis(group: GroupId, op: &MembershipOp, out: &mut Applied) -> Result<(), Rejection> {
    let MembershipBody::Genesis {
        name,
        nonce,
        founder,
        founder_name,
        device_name,
    } = op.body()
    else {
        return Err(Rejection::NotFromGenesis);
    };
    if op.unsigned.seq != 1 || GroupId::derive(&op.author(), nonce) != group {
        return Err(Rejection::InvalidGenesis);
    }
    if ![name, founder_name, device_name]
        .iter()
        .all(|text| valid_name(text))
    {
        return Err(Rejection::InvalidName);
    }
    let state = &mut out.state;
    name.trim().clone_into(&mut state.name);
    state.founder = Some(*founder);
    state.people.insert(
        *founder,
        Member {
            person: *founder,
            name: founder_name.trim().to_owned(),
            role: Role::Admin,
            devices: BTreeMap::from([(
                op.author(),
                DeviceEntry {
                    name: device_name.trim().to_owned(),
                    added_at: op.unsigned.at,
                },
            )]),
        },
    );
    out.device_owner.insert(op.author(), *founder);
    Ok(())
}

fn apply_change(op: &MembershipOp, hash: OpHash, out: &mut Applied) -> Result<(), Rejection> {
    let state = &mut out.state;
    let author = state
        .owner_of(&op.author())
        .ok_or(Rejection::UnknownAuthor)?;
    let (author_person, author_role) = (author.person, author.role);
    let manages = author_role.manages();

    match op.body() {
        MembershipBody::Genesis { .. } => Err(Rejection::InvalidGenesis),
        MembershipBody::AddPerson { person, name, role } => {
            if !manages {
                return Err(Rejection::NotAllowed);
            }
            if state.people.contains_key(person) {
                return Err(Rejection::AlreadyPresent);
            }
            if !valid_name(name) {
                return Err(Rejection::InvalidName);
            }
            state.removed_people.remove(person);
            state.people.insert(
                *person,
                Member {
                    person: *person,
                    name: name.trim().to_owned(),
                    role: *role,
                    devices: BTreeMap::new(),
                },
            );
            Ok(())
        }
        MembershipBody::AddDevice {
            person,
            device,
            name,
        } => {
            if !manages && author_person != *person {
                return Err(Rejection::NotAllowed);
            }
            if state.owner_of(device).is_some() {
                return Err(Rejection::AlreadyPresent);
            }
            if !valid_name(name) {
                return Err(Rejection::InvalidName);
            }
            let member = state
                .people
                .get_mut(person)
                .ok_or(Rejection::UnknownTarget)?;
            member.devices.insert(
                *device,
                DeviceEntry {
                    name: name.trim().to_owned(),
                    added_at: op.unsigned.at,
                },
            );
            state.removed_devices.remove(device);
            state.former_devices.remove(device);
            out.device_owner.insert(*device, *person);
            Ok(())
        }
        MembershipBody::RemoveDevice { device } => {
            let owner = state
                .owner_of(device)
                .ok_or(Rejection::UnknownTarget)?
                .person;
            if !manages && owner != author_person {
                return Err(Rejection::NotAllowed);
            }
            if let Some(member) = state.people.get_mut(&owner) {
                member.devices.remove(device);
                state
                    .former_devices
                    .insert(*device, (owner, member.name.clone()));
            }
            state.removed_devices.insert(*device);
            out.removals.push(Removal {
                op: hash,
                target: Target::Device(*device),
            });
            Ok(())
        }
        MembershipBody::RemovePerson { person } => {
            let leaving = author_person == *person;
            if !manages && !leaving {
                return Err(Rejection::NotAllowed);
            }
            let target = state.people.get(person).ok_or(Rejection::UnknownTarget)?;
            if target.role.manages() && state.admin_count() == 1 {
                return Err(Rejection::WouldLeaveNoAdmin);
            }
            if let Some(member) = state.people.remove(person) {
                for device in member.devices.into_keys() {
                    state
                        .former_devices
                        .insert(device, (member.person, member.name.clone()));
                    state.removed_devices.insert(device);
                }
            }
            state.removed_people.insert(*person);
            out.removals.push(Removal {
                op: hash,
                target: Target::Person(*person),
            });
            Ok(())
        }
        MembershipBody::ChangeRole { person, role } => {
            if !manages {
                return Err(Rejection::NotAllowed);
            }
            let admins = state.admin_count();
            let target = state
                .people
                .get_mut(person)
                .ok_or(Rejection::UnknownTarget)?;
            if target.role.manages() && !role.manages() && admins == 1 {
                return Err(Rejection::WouldLeaveNoAdmin);
            }
            target.role = *role;
            Ok(())
        }
        MembershipBody::Rename { name } => {
            if !manages {
                return Err(Rejection::NotAllowed);
            }
            if !valid_name(name) {
                return Err(Rejection::InvalidName);
            }
            name.trim().clone_into(&mut state.name);
            Ok(())
        }
    }
}

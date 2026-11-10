//! Spaces: who a shared thing is for. Either the **whole group**, or **two people**
//! ("Me & Sam"). Each space is its own lane: its own signed history, its own lane
//! keys (only that space's computers ever get them), and its own audience when
//! computers sync. A pair's history never reaches anyone else's computer.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::fold::GroupState;
use super::keys::{DeviceId, GroupId, OpHash, PersonId};
use super::knowledge::LaneId;

/// Who a shared thing is for.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(tag = "space", rename_all = "snake_case")]
pub enum Space {
    /// Everyone in the group.
    #[default]
    Group,
    /// Exactly two people, in a fixed order so both name the same space.
    Pair { people: [PersonId; 2] },
}

impl Space {
    /// The space for two people, whichever order they are named in.
    pub fn pair(a: PersonId, b: PersonId) -> Self {
        let people = if a <= b { [a, b] } else { [b, a] };
        Self::Pair { people }
    }

    /// Its lane in `group`.
    pub fn lane(&self, group: &GroupId) -> LaneId {
        match self {
            Self::Group => LaneId::whole_group(group),
            Self::Pair { people } => {
                let mut hasher = Sha256::new();
                hasher.update(b"sovra-lane-v1/pair");
                hasher.update(group.as_bytes());
                hasher.update(people[0].as_bytes());
                hasher.update(people[1].as_bytes());
                LaneId(OpHash(hasher.finalize().into()))
            }
        }
    }

    /// Whether `person` belongs to it.
    pub fn includes(&self, person: &PersonId) -> bool {
        match self {
            Self::Group => true,
            Self::Pair { people } => people.contains(person),
        }
    }

    /// For a pair, the person who is not `me`.
    pub fn other(&self, me: &PersonId) -> Option<PersonId> {
        match self {
            Self::Group => None,
            Self::Pair { people } => people.iter().copied().find(|person| person != me),
        }
    }

    /// The computers that may hold its keys and receive its history now.
    pub fn audience(&self, roster: &GroupState) -> BTreeSet<DeviceId> {
        roster
            .people
            .values()
            .filter(|member| self.includes(&member.person))
            .flat_map(|member| member.devices.keys().copied())
            .collect()
    }

    /// Whether `device`'s person (current or former) belongs to it.
    pub fn reaches(&self, roster: &GroupState, device: &DeviceId) -> bool {
        match self {
            Self::Group => true,
            Self::Pair { .. } => roster
                .owner_of(device)
                .map(|member| member.person)
                .or_else(|| roster.former_devices.get(device).map(|(person, _)| *person))
                .is_some_and(|person| self.includes(&person)),
        }
    }
}

/// The spaces `me` belongs to: the whole group, and a pair with every other person
/// who is or was in it (so history with someone who left stays readable).
pub fn spaces_of(roster: &GroupState, me: &PersonId) -> Vec<Space> {
    let mut others: BTreeSet<PersonId> = roster.people.keys().copied().collect();
    others.extend(roster.former_devices.values().map(|(person, _)| *person));
    others.remove(me);
    std::iter::once(Space::Group)
        .chain(others.into_iter().map(|other| Space::pair(*me, other)))
        .collect()
}

/// Which space a lane belongs to, among every pair of people the group has known.
pub fn space_of_lane(roster: &GroupState, lane: &LaneId) -> Option<Space> {
    lanes_of(roster).remove(lane)
}

/// Every space the group has had (the whole group and each pair of people, current or
/// former), by lane, for routing many operations at once.
pub fn lanes_of(roster: &GroupState) -> std::collections::BTreeMap<LaneId, Space> {
    let mut people: BTreeSet<PersonId> = roster.people.keys().copied().collect();
    people.extend(roster.former_devices.values().map(|(person, _)| *person));
    let people: Vec<PersonId> = people.into_iter().collect();
    let mut lanes =
        std::collections::BTreeMap::from([(LaneId::whole_group(&roster.group), Space::Group)]);
    for (index, a) in people.iter().enumerate() {
        for b in &people[index + 1..] {
            let space = Space::pair(*a, *b);
            lanes.insert(space.lane(&roster.group), space);
        }
    }
    lanes
}

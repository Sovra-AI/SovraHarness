//! What the UI asks for and hears back about groups, and the state the Groups
//! screens render. Copy here is plain words (Skills-page rule): "computer", not
//! "device"; "group", not "mesh".

use super::fold::GroupState;
use super::keys::{DeviceId, OpHash, PersonId};
use super::knowledge::SharedDetail;
use super::membership::Role;
use super::pairing::{PairingStage, PairingStep};
use crate::Timestamp;

/// A request from the UI about groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupCommand {
    /// Load this computer's group (if any) and its identity.
    Load,
    /// Start a new group with this computer as its first admin.
    Create {
        name: String,
        your_name: String,
        computer_name: String,
    },
    /// Show a QR code and link for another of *my* computers.
    InviteComputer,
    /// Show a QR code and link for a new person.
    InvitePerson {
        name: String,
        role: Role,
    },
    /// Same room, no camera: show a six-digit code instead.
    ShowShortCode,
    /// On the joining computer: a pasted or scanned link.
    Join {
        link: String,
        computer_name: String,
    },
    /// On the joining computer: the six digits shown on the other screen.
    JoinWithCode {
        code: String,
        computer_name: String,
    },
    /// "They match" / "They don't" on the emoji check.
    ConfirmCode {
        matches: bool,
    },
    CancelPairing,
    Rename {
        name: String,
    },
    ChangeRole {
        person: PersonId,
        role: Role,
    },
    RemoveComputer {
        device: DeviceId,
    },
    RemovePerson {
        person: PersonId,
    },
    Leave,
    /// Share a detail with the whole group.
    Share {
        text: String,
        about: Vec<String>,
    },
    /// Correct one or more shared details (settles a dispute when it names several).
    Correct {
        details: Vec<OpHash>,
        text: String,
    },
    /// "I agree" with someone else's detail.
    Agree {
        detail: OpHash,
    },
    /// Stop helpers using a detail; it stays in history.
    Withdraw {
        detail: OpHash,
    },
    /// Erase a detail's words on every computer.
    Erase {
        detail: OpHash,
    },
}

/// Something went wrong, in words a person can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupProblem {
    /// The OS keychain is unavailable, so this computer has no safe identity.
    NoKeychain,
    /// The other computer could not be reached (offline, or the network blocks it).
    Unreachable,
    InviteExpired,
    InviteUsed,
    /// The link or code is not valid.
    BadInvite,
    NotAllowed,
    LastAdmin,
    /// Anything else, already phrased for people.
    Other(String),
}

impl GroupProblem {
    pub fn message(&self) -> &str {
        match self {
            Self::NoKeychain => {
                "This computer can't keep a safe key yet. Turn on the system keychain and try again."
            }
            Self::Unreachable => {
                "Couldn't reach the other computer. Check both are online and try again."
            }
            Self::InviteExpired => "That invite has expired. Make a new one.",
            Self::InviteUsed => "That invite was already used. Make a new one.",
            Self::BadInvite => "That link or code isn't right. Check it, or make a new one.",
            Self::NotAllowed => "Only an admin can do that.",
            Self::LastAdmin => {
                "A group needs at least one admin. Make someone else an admin first."
            }
            Self::Other(text) => text,
        }
    }
}

/// A notification from the runtime about groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupEvent {
    Loaded {
        this_computer: DeviceId,
        group: Option<GroupState>,
    },
    /// The roster changed (here or on another computer).
    Changed(GroupState),
    InviteReady {
        link: String,
        expires_at: Timestamp,
    },
    ShortCodeReady {
        code: String,
        expires_at: Timestamp,
    },
    Pairing(PairingStep),
    /// Everything shared in the group, newest first (usable and historical).
    Knowledge(Vec<SharedDetail>),
    /// This computer was removed from, or left, its group.
    LeftGroup,
    Problem(GroupProblem),
}

/// One row of the roster screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonRow {
    pub person: PersonId,
    pub name: String,
    pub role: Role,
    pub role_label: &'static str,
    pub computers: Vec<ComputerRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerRow {
    pub device: DeviceId,
    pub name: String,
    pub fingerprint: String,
    pub is_this_computer: bool,
}

/// Plain-word role names.
pub const fn role_label(role: Role) -> &'static str {
    match role {
        Role::Admin => "Admin — can add and remove people",
        Role::Member => "Member — can read and add",
        Role::Viewer => "Viewer — can read",
        Role::Contributor => "Contributor — can add, can't read",
    }
}

/// The honest message before removing a computer.
pub fn removal_warning(computer_name: &str) -> String {
    format!(
        "{computer_name} will stop getting new memories and work. Things it already has stay on it. \
         Change anything sensitive it knew."
    )
}

/// Everything the Groups screens show. Built only by [`GroupsView::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupsView {
    pub this_computer: Option<DeviceId>,
    pub group: Option<GroupState>,
    pub pairing: PairingStage,
    pub invite: Option<(String, Timestamp)>,
    pub short_code: Option<(String, Timestamp)>,
    pub problem: Option<GroupProblem>,
    pub loaded: bool,
    /// Shared details, newest first.
    pub knowledge: Vec<SharedDetail>,
}

impl Default for GroupsView {
    fn default() -> Self {
        Self {
            this_computer: None,
            group: None,
            pairing: PairingStage::Idle,
            invite: None,
            short_code: None,
            problem: None,
            loaded: false,
            knowledge: Vec::new(),
        }
    }
}

impl GroupsView {
    /// Pure reducer: the UI calls this for every [`GroupEvent`] and re-renders.
    pub fn apply(&mut self, event: GroupEvent) {
        match event {
            GroupEvent::Loaded {
                this_computer,
                group,
            } => {
                self.this_computer = Some(this_computer);
                self.group = group;
                self.loaded = true;
                self.problem = None;
            }
            GroupEvent::Changed(state) => {
                let removed = self
                    .this_computer
                    .is_some_and(|me| self.group.is_some() && !state.admits(&me));
                if removed {
                    self.leave();
                } else {
                    self.group = Some(state);
                }
            }
            GroupEvent::InviteReady { link, expires_at } => {
                self.invite = Some((link, expires_at));
                self.short_code = None;
                self.pairing = std::mem::replace(&mut self.pairing, PairingStage::Idle)
                    .advance(PairingStep::InviteShown { expires_at });
            }
            GroupEvent::ShortCodeReady { code, expires_at } => {
                self.short_code = Some((code, expires_at));
                self.pairing = std::mem::replace(&mut self.pairing, PairingStage::Idle)
                    .advance(PairingStep::InviteShown { expires_at });
            }
            GroupEvent::Pairing(step) => {
                let finished = matches!(
                    step,
                    PairingStep::Admitted { .. } | PairingStep::Failed(_) | PairingStep::Cancel
                );
                self.pairing =
                    std::mem::replace(&mut self.pairing, PairingStage::Idle).advance(step);
                if finished {
                    self.invite = None;
                    self.short_code = None;
                }
            }
            GroupEvent::Knowledge(details) => self.knowledge = details,
            GroupEvent::LeftGroup => self.leave(),
            GroupEvent::Problem(problem) => self.problem = Some(problem),
        }
    }

    fn leave(&mut self) {
        self.group = None;
        self.knowledge.clear();
        self.invite = None;
        self.short_code = None;
        self.pairing = PairingStage::Idle;
    }

    /// The role of this computer's person, if it is in a group.
    pub fn my_role(&self) -> Option<Role> {
        let me = self.this_computer?;
        self.group.as_ref()?.role_of(&me)
    }

    pub fn can_manage(&self) -> bool {
        self.my_role().is_some_and(Role::manages)
    }

    /// Show "Add a second admin so you're never locked out."
    pub fn show_second_admin_banner(&self) -> bool {
        self.can_manage()
            && self
                .group
                .as_ref()
                .is_some_and(GroupState::needs_second_admin)
    }

    /// Roster rows, admins first, then by name; this computer's person first among equals.
    pub fn rows(&self) -> Vec<PersonRow> {
        let Some(group) = &self.group else {
            return Vec::new();
        };
        let me = self.this_computer;
        let mut rows: Vec<PersonRow> = group
            .people
            .values()
            .map(|member| PersonRow {
                person: member.person,
                name: member.name.clone(),
                role: member.role,
                role_label: role_label(member.role),
                computers: member
                    .devices
                    .iter()
                    .map(|(device, entry)| ComputerRow {
                        device: *device,
                        name: entry.name.clone(),
                        fingerprint: device.fingerprint(),
                        is_this_computer: Some(*device) == me,
                    })
                    .collect(),
            })
            .collect();
        rows.sort_by(|a, b| {
            let mine = |row: &PersonRow| row.computers.iter().any(|c| c.is_this_computer);
            b.role
                .cmp(&a.role)
                .then_with(|| mine(b).cmp(&mine(a)))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        rows
    }
}

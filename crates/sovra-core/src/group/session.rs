//! What the UI asks for and hears back about groups, and the state the Groups
//! screens render. Copy here is plain words (Skills-page rule): "computer", not
//! "device"; "group", not "mesh".

use super::federated::{AskRecord, FederatedSettings, GroupAnswer};
use super::fold::GroupState;
use super::keys::{DeviceId, OpHash, PersonId};
use super::knowledge::SharedDetail;
use super::membership::Role;
use super::pairing::{InviteKind, PairingStage, PairingStep};
use super::shared_artifact::SharedArtifact;
use crate::Timestamp;
use crate::artifacts::ArtifactKind;

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
    /// Same room, no camera: show a six-digit code instead (for this kind of invite).
    ShowShortCode {
        kind: InviteKind,
    },
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
    /// Remove this person and all their computers; the sole person ends the group.
    Leave,
    /// Share a detail with the whole group, or (`with`) just one other person.
    Share {
        text: String,
        about: Vec<String>,
        with: Option<PersonId>,
    },
    /// Share a detail with one department only (its own lane and keys).
    ShareInDepartment {
        department: OpHash,
        text: String,
        about: Vec<String>,
    },
    /// Admins: create department `slug` or set its name and members.
    SetDepartment {
        slug: crate::swarm::department::DepartmentId,
        name: String,
        members: std::collections::BTreeSet<PersonId>,
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
    /// Erase a detail's (or shared artifact's) words on every computer.
    Erase {
        detail: OpHash,
    },
    /// Turn federated recall on or off for this computer: answering others' helpers
    /// from this memory, and asking the group's computers when recalling.
    SetFederated(FederatedSettings),
    /// Turn work handoff on or off for this computer.
    SetWork(super::work::WorkSettings),
    /// "Help the group train": let this computer help, on its own, with training
    /// that someone in the group started with the group. Starts off and stays as
    /// set. Answering questions or lending an AI does not turn it on.
    SetTrainingHelp(bool),
    /// "Ask my group": search the memories of computers that answer, once, now.
    AskGroup {
        question: String,
    },
    /// Share a helper or schedule with the whole group, or (`with`) one person.
    ShareTemplate {
        template: super::template::Template,
        with: Option<PersonId>,
    },
    /// Share a saved artifact with the whole group. `new_version_of` names the shared
    /// artifact it was copied from, so it becomes that artifact's newest version.
    ShareArtifact {
        title: String,
        kind: ArtifactKind,
        language: String,
        content: String,
        new_version_of: Option<OpHash>,
        /// This computer's library id for it, so later edits find their way back.
        local_id: String,
        /// Just one other person instead of the whole group. A new version always
        /// stays in its original's space.
        with: Option<PersonId>,
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
        /// The link as a scannable code (rows of dark modules); empty when unavailable.
        qr: Vec<Vec<bool>>,
    },
    ShortCodeReady {
        code: String,
        expires_at: Timestamp,
    },
    Pairing(PairingStep),
    /// Everything shared in the group, newest first (usable and historical).
    Knowledge(Vec<SharedDetail>),
    /// Every shared artifact version, latest versions first.
    Artifacts(Vec<SharedArtifact>),
    /// Every shared helper and schedule, latest versions first.
    Templates(Vec<super::template::SharedTemplate>),
    /// This computer's federated recall choices and the questions it answered lately.
    Federated {
        settings: FederatedSettings,
        recent: Vec<AskRecord>,
    },
    /// When this computer last heard from each other computer in the group.
    Presence(std::collections::BTreeMap<DeviceId, Timestamp>),
    /// This computer's work-handoff choices and the jobs it ran for others lately.
    Work {
        settings: super::work::WorkSettings,
        recent: Vec<super::work::WorkRecord>,
    },
    /// Whether this computer helps the group's training runs. Starts off.
    TrainingHelp(bool),
    /// What the group's computers answered to an "Ask my group" question.
    Answers {
        question: String,
        answers: Vec<GroupAnswer>,
    },
    /// This computer was removed from, or left, its group.
    LeftGroup,
    Problem(GroupProblem),
    /// This computer's audit history: what it shared and gave out, re-checked.
    Audit(crate::swarm::audit::AuditSummary),
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
    /// When this computer last heard from it (`None`: not since it started).
    pub last_seen: Option<Timestamp>,
}

/// A computer counts as online if it was heard from within this long (three sync
/// rounds, so one missed round does not flicker).
pub const ONLINE_WINDOW_MS: i64 = 60_000;

/// "Online now", "Seen 5 min ago" or "Not seen yet", for one computer.
pub fn presence_label(last_seen: Option<Timestamp>, now: Timestamp) -> String {
    let Some(seen) = last_seen else {
        return "Not seen since this computer started".to_owned();
    };
    let ago = now.0.saturating_sub(seen.0).max(0);
    if ago < ONLINE_WINDOW_MS {
        return "Online now".to_owned();
    }
    let minutes = ago / 60_000;
    if minutes < 60 {
        format!("Seen {minutes} min ago")
    } else if minutes < 48 * 60 {
        format!("Seen {} h ago", minutes / 60)
    } else {
        format!("Seen {} days ago", minutes / (24 * 60))
    }
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
    /// The invite link as a scannable code.
    pub invite_qr: Vec<Vec<bool>>,
    pub short_code: Option<(String, Timestamp)>,
    pub problem: Option<GroupProblem>,
    pub loaded: bool,
    /// Shared details, newest first.
    pub knowledge: Vec<SharedDetail>,
    /// Shared artifact versions, latest versions first.
    pub artifacts: Vec<SharedArtifact>,
    /// Shared helpers and schedules, latest versions first.
    pub templates: Vec<super::template::SharedTemplate>,
    pub federated: FederatedSettings,
    /// Questions other helpers asked this computer, newest first.
    pub recent_asks: Vec<AskRecord>,
    /// The last "Ask my group" question and what came back.
    pub answers: Option<(String, Vec<GroupAnswer>)>,
    /// When this computer last heard from each other computer.
    pub last_seen: std::collections::BTreeMap<DeviceId, Timestamp>,
    pub work: super::work::WorkSettings,
    /// Chat jobs this computer answered for others, newest first.
    pub recent_work: Vec<super::work::WorkRecord>,
    /// "Help the group train": this computer helps the group's training runs on
    /// its own while it is idle. Starts off.
    pub training_help: bool,
    /// This computer's audit history. Kept when leaving a group: history is never lost.
    pub audit: crate::swarm::audit::AuditSummary,
}

impl Default for GroupsView {
    fn default() -> Self {
        Self {
            this_computer: None,
            group: None,
            pairing: PairingStage::Idle,
            invite: None,
            invite_qr: Vec::new(),
            short_code: None,
            problem: None,
            loaded: false,
            knowledge: Vec::new(),
            artifacts: Vec::new(),
            templates: Vec::new(),
            federated: FederatedSettings::default(),
            recent_asks: Vec::new(),
            answers: None,
            last_seen: std::collections::BTreeMap::new(),
            work: super::work::WorkSettings::default(),
            recent_work: Vec::new(),
            training_help: false,
            audit: crate::swarm::audit::AuditSummary::default(),
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
            GroupEvent::InviteReady {
                link,
                expires_at,
                qr,
            } => {
                self.invite = Some((link, expires_at));
                self.invite_qr = qr;
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
            GroupEvent::Artifacts(artifacts) => self.artifacts = artifacts,
            GroupEvent::Templates(templates) => self.templates = templates,
            GroupEvent::Federated { settings, recent } => {
                self.federated = settings;
                self.recent_asks = recent;
            }
            GroupEvent::Answers { question, answers } => self.answers = Some((question, answers)),
            GroupEvent::Presence(last_seen) => self.last_seen = last_seen,
            GroupEvent::Work { settings, recent } => {
                self.work = settings;
                self.recent_work = recent;
            }
            GroupEvent::TrainingHelp(on) => self.training_help = on,
            GroupEvent::LeftGroup => self.leave(),
            GroupEvent::Problem(problem) => self.problem = Some(problem),
            GroupEvent::Audit(summary) => self.audit = summary,
        }
    }

    fn leave(&mut self) {
        self.group = None;
        self.knowledge.clear();
        self.artifacts.clear();
        self.templates.clear();
        self.recent_asks.clear();
        self.answers = None;
        self.last_seen.clear();
        self.recent_work.clear();
        self.invite = None;
        self.short_code = None;
        self.pairing = PairingStage::Idle;
    }

    /// This computer's person in the group, if any.
    pub fn my_person(&self) -> Option<PersonId> {
        let me = self.this_computer?;
        self.group
            .as_ref()?
            .owner_of(&me)
            .map(|member| member.person)
    }

    /// Everyone else in the group, by name, for "Just Dad" choices.
    pub fn others(&self) -> Vec<(PersonId, String)> {
        let me = self.my_person();
        let mut others: Vec<(PersonId, String)> = self
            .group
            .iter()
            .flat_map(|group| group.people.values())
            .filter(|member| Some(member.person) != me)
            .map(|member| (member.person, member.name.clone()))
            .collect();
        others.sort_by_key(|a| a.1.to_lowercase());
        others
    }

    /// "Just you and Dad" for something in a two-person space; `None` for the group.
    pub fn space_label(&self, space: &super::space::Space) -> Option<String> {
        if let super::space::Space::Department { id } = space {
            let name = self
                .group
                .as_ref()
                .and_then(|group| group.departments.get(id))
                .map_or_else(|| "a department".to_owned(), |d| d.name.clone());
            return Some(format!("Just {name}"));
        }
        let me = self.my_person()?;
        let other = space.other(&me)?;
        let name = self
            .group
            .as_ref()
            .and_then(|group| group.people.get(&other))
            .map_or_else(
                || "someone who left".to_owned(),
                |member| member.name.clone(),
            );
        Some(format!("Just you and {name}"))
    }

    /// The departments this computer's person is in now, by name.
    pub fn my_departments(&self) -> Vec<(OpHash, String)> {
        match (self.my_person(), self.group.as_ref()) {
            (Some(me), Some(group)) => group
                .departments_of(&me)
                .into_iter()
                .map(|(id, d)| (id, d.name.clone()))
                .collect(),
            _ => Vec::new(),
        }
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
                        last_seen: self.last_seen.get(device).copied(),
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

/// A personal memory reworded for the group: "the user" / "User" becomes the
/// sharer's name, so "User's dentist is Dr Patel" reaches others as
/// "Mum's dentist is Dr Patel". People still check the words before sharing.
pub fn shareable_text(statement: &str, your_name: &str) -> String {
    let name = your_name.trim();
    if name.is_empty() {
        return statement.trim().to_owned();
    }
    let mut out = String::with_capacity(statement.len());
    let mut rest = statement.trim();
    while !rest.is_empty() {
        let word_end = rest
            .find(|c: char| !c.is_alphanumeric())
            .unwrap_or(rest.len());
        if word_end == 0 {
            let c = rest.chars().next().expect("rest is not empty");
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let word = &rest[..word_end];
        let after = &rest[word_end..];
        // `get` never splits a character, so non-English words pass through safely.
        let the_user = word.eq_ignore_ascii_case("the")
            && after.starts_with(' ')
            && after
                .get(1..5)
                .is_some_and(|next| next.eq_ignore_ascii_case("user"))
            && after
                .get(5..)
                .is_some_and(|tail| !tail.starts_with(|c: char| c.is_alphanumeric()));
        if the_user {
            out.push_str(name);
            rest = &after[5..];
        } else if word.eq_ignore_ascii_case("user") {
            out.push_str(name);
            rest = after;
        } else {
            out.push_str(word);
            rest = after;
        }
    }
    out
}

/// Topics for a shared memory: its subjects, with "user" named, at most the limit.
pub fn shareable_topics(entities: &[String], your_name: &str) -> Vec<String> {
    let mut topics: Vec<String> = Vec::new();
    for entity in entities {
        let topic = if entity.trim().eq_ignore_ascii_case("user") {
            your_name.trim().to_owned()
        } else {
            entity.trim().to_owned()
        };
        if !topic.is_empty() && !topics.contains(&topic) {
            topics.push(topic);
        }
    }
    topics.truncate(super::knowledge::MAX_SHARED_TOPICS);
    topics
}

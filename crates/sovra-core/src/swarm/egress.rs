//! Human sign-off before anything leaves a computer.
//!
//! Every draft a helper wants to send out (a message, a copy for another computer, a
//! call to a cloud model, this computer's part of a training round) is held here
//! until a person who may approve for its department says yes. The approval is bound
//! to the BLAKE3 hash of the exact content: changing one byte afterwards voids it,
//! and each approval releases exactly once. Department rules are checked again at
//! approval, so a person cannot approve what the department forbids.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::department::{CustodyError, DepartmentId, Directory};
use super::ledger::AuditEvent;
use crate::Timestamp;
use crate::group::{ContentHash, DeviceId, PersonId};

/// Where a draft would go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "snake_case")]
pub enum Destination {
    /// Another enrolled computer.
    Computer { device: DeviceId },
    /// A model run by someone outside the company.
    CloudModel { provider: String },
    /// A person or service outside the company: an email address, a chat.
    Outside { address: String },
    /// This computer's masked, noised update for a training round.
    TrainingRound { round: u64 },
}

impl Destination {
    /// Words for the ledger and the approval card.
    pub fn describe(&self) -> String {
        match self {
            Self::Computer { device } => format!("computer {}", device.fingerprint()),
            Self::CloudModel { provider } => format!("cloud model {provider}"),
            Self::Outside { address } => address.clone(),
            Self::TrainingRound { round } => format!("training round {round}"),
        }
    }
}

/// Something a helper wants to send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub content: ContentHash,
    pub department: DepartmentId,
    pub destination: Destination,
    /// Which helper wrote it, shown on the card.
    pub drafted_by: String,
    /// One line for the card ("Reply to Acme about the invoice").
    pub summary: String,
    pub created_at: Timestamp,
}

/// A person's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Approved { by: PersonId, at: Timestamp },
    Declined { by: PersonId, at: Timestamp },
}

/// A draft and where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    pub draft: Draft,
    pub decision: Option<Decision>,
    pub released: bool,
}

/// Why the gate refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EgressError {
    #[error("there's no draft with that number")]
    Unknown,
    #[error("this draft was already answered")]
    AlreadyDecided,
    #[error("that person can't approve for this department")]
    NotAnApprover,
    #[error("nobody has approved this yet")]
    NotApproved,
    #[error("this was declined")]
    Declined,
    #[error("the content changed after it was approved")]
    ContentChanged,
    #[error("this was already sent")]
    AlreadySent,
    #[error(transparent)]
    Custody(#[from] CustodyError),
}

/// Proof that one approved draft may now go out. Spent by the adapter that sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub id: u64,
    pub content: ContentHash,
    pub destination: Destination,
}

/// The waiting room for everything leaving this computer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressGate {
    held: BTreeMap<u64, Held>,
    next: u64,
}

impl EgressGate {
    /// Holds a draft and returns its number. Nothing is sent.
    pub fn hold(&mut self, draft: Draft) -> u64 {
        self.next += 1;
        self.held.insert(
            self.next,
            Held {
                draft,
                decision: None,
                released: false,
            },
        );
        self.next
    }

    pub fn get(&self, id: u64) -> Option<&Held> {
        self.held.get(&id)
    }

    /// Drafts still waiting for a person, oldest first.
    pub fn waiting(&self) -> impl Iterator<Item = (u64, &Held)> {
        self.held
            .iter()
            .filter(|(_, held)| held.decision.is_none())
            .map(|(id, held)| (*id, held))
    }

    /// A person approves draft `id`. Re-checks department rules and returns the
    /// ledger event to record.
    pub fn approve(
        &mut self,
        id: u64,
        by: PersonId,
        at: Timestamp,
        directory: &Directory,
    ) -> Result<AuditEvent, EgressError> {
        let held = self.held.get_mut(&id).ok_or(EgressError::Unknown)?;
        if held.decision.is_some() {
            return Err(EgressError::AlreadyDecided);
        }
        let draft = &held.draft;
        if !directory.department(&draft.department)?.may_approve(&by) {
            return Err(EgressError::NotAnApprover);
        }
        match &draft.destination {
            Destination::Computer { device } => {
                directory.may_replicate(&draft.department, device)?;
            }
            Destination::CloudModel { .. } => directory.may_use_cloud(&draft.department)?,
            Destination::TrainingRound { .. } => directory.may_train(&draft.department)?,
            Destination::Outside { .. } => {}
        }
        held.decision = Some(Decision::Approved { by, at });
        Ok(AuditEvent::EgressApproved {
            content: draft.content,
            department: draft.department.clone(),
            approver: by,
            destination: draft.destination.describe(),
        })
    }

    /// A person says no. The draft never leaves.
    pub fn decline(&mut self, id: u64, by: PersonId, at: Timestamp) -> Result<(), EgressError> {
        let held = self.held.get_mut(&id).ok_or(EgressError::Unknown)?;
        if held.decision.is_some() {
            return Err(EgressError::AlreadyDecided);
        }
        held.decision = Some(Decision::Declined { by, at });
        Ok(())
    }

    /// Releases draft `id` once, if it was approved and `content` is still exactly
    /// what was approved. The sender hashes the bytes it is about to send.
    pub fn release(&mut self, id: u64, content: &ContentHash) -> Result<Release, EgressError> {
        let held = self.held.get_mut(&id).ok_or(EgressError::Unknown)?;
        match held.decision {
            None => return Err(EgressError::NotApproved),
            Some(Decision::Declined { .. }) => return Err(EgressError::Declined),
            Some(Decision::Approved { .. }) => {}
        }
        if held.released {
            return Err(EgressError::AlreadySent);
        }
        if held.draft.content != *content {
            return Err(EgressError::ContentChanged);
        }
        held.released = true;
        Ok(Release {
            id,
            content: *content,
            destination: held.draft.destination.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::department::{Department, TrainingConsent};

    fn person(n: u8) -> PersonId {
        PersonId([n; 32])
    }

    fn setup() -> (Directory, DepartmentId) {
        let id = DepartmentId::new("finance").expect("valid slug");
        let mut finance = Department::new(id.clone(), "Finance");
        finance.members.extend([person(1), person(2)]);
        finance.approvers.insert(person(1));
        let mut directory = Directory::default();
        directory.add(finance);
        directory.devices.insert(DeviceId([2; 32]), person(2));
        (directory, id)
    }

    fn draft(department: &DepartmentId, destination: Destination, body: &[u8]) -> Draft {
        Draft {
            content: ContentHash::of(body),
            department: department.clone(),
            destination,
            drafted_by: "Writer".into(),
            summary: "Reply to Acme".into(),
            created_at: Timestamp(1),
        }
    }

    #[test]
    fn nothing_leaves_without_a_named_person_saying_yes() {
        let (directory, finance) = setup();
        let mut gate = EgressGate::default();
        let body = b"Dear Acme, invoice attached.";
        let outside = Destination::Outside {
            address: "ap@acme.example".into(),
        };
        let id = gate.hold(draft(&finance, outside, body));
        assert_eq!(gate.waiting().count(), 1);
        assert_eq!(
            gate.release(id, &ContentHash::of(body)),
            Err(EgressError::NotApproved)
        );
        // A member who is not an approver cannot sign off.
        assert_eq!(
            gate.approve(id, person(2), Timestamp(2), &directory),
            Err(EgressError::NotAnApprover)
        );
        let event = gate
            .approve(id, person(1), Timestamp(2), &directory)
            .expect("the approver approves");
        assert!(
            matches!(event, AuditEvent::EgressApproved { approver, .. } if approver == person(1))
        );
        // Edited after approval: refused.
        assert_eq!(
            gate.release(id, &ContentHash::of(b"Dear Acme, send money here.")),
            Err(EgressError::ContentChanged)
        );
        assert!(gate.release(id, &ContentHash::of(body)).is_ok());
        assert_eq!(
            gate.release(id, &ContentHash::of(body)),
            Err(EgressError::AlreadySent)
        );
    }

    #[test]
    fn approval_cannot_override_department_rules() {
        let (directory, finance) = setup();
        let mut gate = EgressGate::default();
        let cloud = gate.hold(draft(
            &finance,
            Destination::CloudModel {
                provider: "someone".into(),
            },
            b"q",
        ));
        assert_eq!(
            gate.approve(cloud, person(1), Timestamp(2), &directory),
            Err(EgressError::Custody(CustodyError::CloudBlocked(
                finance.clone()
            )))
        );
        let round = gate.hold(draft(
            &finance,
            Destination::TrainingRound { round: 1 },
            b"u",
        ));
        assert!(matches!(
            gate.approve(round, person(1), Timestamp(2), &directory),
            Err(EgressError::Custody(CustodyError::TrainingOff(_)))
        ));
        let stranger = gate.hold(draft(
            &finance,
            Destination::Computer {
                device: DeviceId([9; 32]),
            },
            b"c",
        ));
        assert!(
            gate.approve(stranger, person(1), Timestamp(2), &directory)
                .is_err()
        );
        let colleague = gate.hold(draft(
            &finance,
            Destination::Computer {
                device: DeviceId([2; 32]),
            },
            b"c",
        ));
        assert!(
            gate.approve(colleague, person(1), Timestamp(2), &directory)
                .is_ok()
        );
    }

    #[test]
    fn declined_drafts_never_go() {
        let (mut directory, finance) = setup();
        if let Some(d) = directory.departments.get_mut(&finance) {
            d.training = TrainingConsent::ApprovedExamples;
        }
        let mut gate = EgressGate::default();
        let id = gate.hold(draft(
            &finance,
            Destination::TrainingRound { round: 3 },
            b"u",
        ));
        gate.decline(id, person(1), Timestamp(2)).expect("waiting");
        assert_eq!(
            gate.approve(id, person(1), Timestamp(3), &directory),
            Err(EgressError::AlreadyDecided)
        );
        assert_eq!(
            gate.release(id, &ContentHash::of(b"u")),
            Err(EgressError::Declined)
        );
    }
}

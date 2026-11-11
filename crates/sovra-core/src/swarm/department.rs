//! Departments: each business unit (legal, finance, engineering) keeps custody of
//! its own memory.
//!
//! A department is a set of people and a policy. The rules are enforced **before**
//! retrieval and before replication (principle 4, "filter before retrieval"): a
//! person outside a department never gets its items in a search, and its items are
//! only ever copied to computers that belong to its members. Sending a department's
//! words to a cloud model, or letting them train a shared model, are two more
//! switches that start off.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::group::{DeviceId, PersonId};

/// The longest department id: a short slug such as `legal` or `finance-emea`.
pub const MAX_DEPARTMENT_ID_CHARS: usize = 48;

/// A department's stable slug.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DepartmentId(String);

/// Why a department id was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DepartmentIdError {
    #[error(
        "a department id needs 1 to {MAX_DEPARTMENT_ID_CHARS} lowercase letters, digits or dashes"
    )]
    Invalid,
}

impl DepartmentId {
    pub fn new(slug: impl Into<String>) -> Result<Self, DepartmentIdError> {
        let slug = slug.into();
        let valid = !slug.is_empty()
            && slug.len() <= MAX_DEPARTMENT_ID_CHARS
            && !slug.starts_with('-')
            && !slug.ends_with('-')
            && slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if valid {
            Ok(Self(slug))
        } else {
            Err(DepartmentIdError::Invalid)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DepartmentId {
    type Error = DepartmentIdError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<DepartmentId> for String {
    fn from(value: DepartmentId) -> Self {
        value.0
    }
}

impl std::fmt::Display for DepartmentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether a department's knowledge may train a shared model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingConsent {
    /// Never. The default, and the only sensible setting for matter lanes in law.
    #[default]
    Off,
    /// Only examples a person approved, only with privacy noise and secure sums.
    ApprovedExamples,
}

/// One business unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Department {
    pub id: DepartmentId,
    /// The name people see ("Legal").
    pub name: String,
    pub members: BTreeSet<PersonId>,
    /// Who may approve something leaving this department. Empty means any member.
    #[serde(default)]
    pub approvers: BTreeSet<PersonId>,
    /// Whether its words may be sent to a model outside the company's computers.
    #[serde(default)]
    pub cloud_allowed: bool,
    #[serde(default)]
    pub training: TrainingConsent,
}

impl Department {
    /// A department with no cloud and no training, the safe default.
    pub fn new(id: DepartmentId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            members: BTreeSet::new(),
            approvers: BTreeSet::new(),
            cloud_allowed: false,
            training: TrainingConsent::Off,
        }
    }

    /// Whether `person` may sign off on something leaving this department.
    pub fn may_approve(&self, person: &PersonId) -> bool {
        if self.approvers.is_empty() {
            self.members.contains(person)
        } else {
            self.approvers.contains(person)
        }
    }
}

/// Every department in the company and which person owns each computer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Directory {
    pub departments: BTreeMap<DepartmentId, Department>,
    /// The person each enrolled computer belongs to.
    pub devices: BTreeMap<DeviceId, PersonId>,
}

/// Why a department rule refused something.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CustodyError {
    #[error("there is no department called {0}")]
    UnknownDepartment(DepartmentId),
    #[error("that computer isn't enrolled")]
    UnknownDevice,
    #[error("that person isn't in {0}")]
    NotAMember(DepartmentId),
    #[error("{0} keeps its words on the company's own computers")]
    CloudBlocked(DepartmentId),
    #[error("{0} hasn't agreed to train shared models")]
    TrainingOff(DepartmentId),
}

impl Directory {
    pub fn add(&mut self, department: Department) {
        self.departments.insert(department.id.clone(), department);
    }

    pub fn department(&self, id: &DepartmentId) -> Result<&Department, CustodyError> {
        self.departments
            .get(id)
            .ok_or_else(|| CustodyError::UnknownDepartment(id.clone()))
    }

    /// The departments `person` belongs to.
    pub fn departments_of(&self, person: &PersonId) -> BTreeSet<DepartmentId> {
        self.departments
            .values()
            .filter(|department| department.members.contains(person))
            .map(|department| department.id.clone())
            .collect()
    }

    /// Whether `person` may read items of `department`.
    pub fn may_read(&self, person: &PersonId, department: &DepartmentId) -> bool {
        self.departments
            .get(department)
            .is_some_and(|d| d.members.contains(person))
    }

    /// Search hits `person` may see, in their original order. Applied to the
    /// candidates before ranking or any model sees them.
    pub fn filter_for<'a, T>(
        &self,
        person: &PersonId,
        hits: impl IntoIterator<Item = (&'a DepartmentId, T)>,
    ) -> Vec<T> {
        let mine = self.departments_of(person);
        hits.into_iter()
            .filter(|(department, _)| mine.contains(*department))
            .map(|(_, hit)| hit)
            .collect()
    }

    /// Whether an item of `department` may be copied onto computer `to`: only to a
    /// computer owned by one of the department's members.
    pub fn may_replicate(
        &self,
        department: &DepartmentId,
        to: &DeviceId,
    ) -> Result<(), CustodyError> {
        let owner = self.devices.get(to).ok_or(CustodyError::UnknownDevice)?;
        if self.department(department)?.members.contains(owner) {
            Ok(())
        } else {
            Err(CustodyError::NotAMember(department.clone()))
        }
    }

    /// Whether `department`'s words may reach a cloud model.
    pub fn may_use_cloud(&self, department: &DepartmentId) -> Result<(), CustodyError> {
        if self.department(department)?.cloud_allowed {
            Ok(())
        } else {
            Err(CustodyError::CloudBlocked(department.clone()))
        }
    }

    /// Whether `department` contributes to shared training.
    pub fn may_train(&self, department: &DepartmentId) -> Result<(), CustodyError> {
        match self.department(department)?.training {
            TrainingConsent::ApprovedExamples => Ok(()),
            TrainingConsent::Off => Err(CustodyError::TrainingOff(department.clone())),
        }
    }

    /// The enrolled computers that may hold `department`'s items.
    pub fn custodians(&self, department: &DepartmentId) -> BTreeSet<DeviceId> {
        let Some(department) = self.departments.get(department) else {
            return BTreeSet::new();
        };
        self.devices
            .iter()
            .filter(|(_, owner)| department.members.contains(owner))
            .map(|(device, _)| *device)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(n: u8) -> PersonId {
        PersonId([n; 32])
    }
    fn device(n: u8) -> DeviceId {
        DeviceId([n; 32])
    }
    fn id(slug: &str) -> DepartmentId {
        DepartmentId::new(slug).expect("valid slug")
    }

    fn company() -> Directory {
        let mut directory = Directory::default();
        let mut legal = Department::new(id("legal"), "Legal");
        legal.members.insert(person(1));
        let mut finance = Department::new(id("finance"), "Finance");
        finance.members.extend([person(2), person(3)]);
        finance.training = TrainingConsent::ApprovedExamples;
        directory.add(legal);
        directory.add(finance);
        directory.devices.extend([
            (device(1), person(1)),
            (device(2), person(2)),
            (device(3), person(3)),
        ]);
        directory
    }

    #[test]
    fn slugs_are_short_lowercase_words() {
        assert!(DepartmentId::new("finance-emea").is_ok());
        for bad in ["", "Legal", "-x", "x-", "a b", &"x".repeat(49)] {
            assert!(DepartmentId::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn search_hits_are_filtered_by_department_before_anyone_sees_them() {
        let directory = company();
        let (legal, finance) = (id("legal"), id("finance"));
        let hits = [(&legal, "contract"), (&finance, "budget"), (&legal, "nda")];
        assert_eq!(directory.filter_for(&person(1), hits), ["contract", "nda"]);
        assert_eq!(directory.filter_for(&person(2), hits), ["budget"]);
        assert_eq!(directory.filter_for(&person(9), hits), Vec::<&str>::new());
    }

    #[test]
    fn items_only_replicate_to_their_departments_computers() {
        let directory = company();
        assert!(directory.may_replicate(&id("finance"), &device(3)).is_ok());
        assert_eq!(
            directory.may_replicate(&id("finance"), &device(1)),
            Err(CustodyError::NotAMember(id("finance")))
        );
        assert_eq!(
            directory.may_replicate(&id("finance"), &device(7)),
            Err(CustodyError::UnknownDevice)
        );
        assert_eq!(
            directory.custodians(&id("finance")),
            BTreeSet::from([device(2), device(3)])
        );
    }

    #[test]
    fn cloud_and_training_start_off() {
        let directory = company();
        assert_eq!(
            directory.may_use_cloud(&id("legal")),
            Err(CustodyError::CloudBlocked(id("legal")))
        );
        assert_eq!(
            directory.may_train(&id("legal")),
            Err(CustodyError::TrainingOff(id("legal")))
        );
        assert!(directory.may_train(&id("finance")).is_ok());
    }
}

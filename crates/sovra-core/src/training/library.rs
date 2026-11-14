//! Finished models: the card that describes one, who has it, and trying it out.
//!
//! A card that arrives from another computer is untrusted until `validate` passes
//! and the fetched file agrees with it. A model is never run as a program: it is
//! only ever used to continue text and as a starting point for more training.

use serde::{Deserialize, Serialize};

use crate::group::{DeviceId, GroupId};

use super::{
    Architecture, BaseModel, MAX_MODEL_BYTES, ModelFacts, ModelId, TrainingError, check_name,
};

/// Longest "Person · Computer" text on a card, in characters.
pub const MAX_TRAINED_BY_CHARS: usize = 120;
/// Most models listed in the state at once.
pub const MAX_LIBRARY_MODELS: usize = 128;
/// Most models received from the group that this computer keeps. When full, new
/// ones are skipped with a notice; nothing is ever removed to make room.
pub const MAX_RECEIVED_MODELS: usize = 64;
/// Most bytes of received models this computer keeps.
pub const MAX_RECEIVED_BYTES: u64 = 512 * 1024 * 1024;
/// Most computers tracked for one model's delivery.
pub const MAX_DELIVERIES: usize = 64;
/// Longest beginning a person can type to try a model.
pub const MAX_TRIAL_PROMPT_BYTES: usize = 512;

/// Where this computer's copy of a model came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelOrigin {
    TrainedHere,
    /// Fetched from this computer of the group.
    Received {
        from: DeviceId,
    },
}

/// What a model is, apart from its values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCard {
    pub id: ModelId,
    pub name: String,
    pub base: BaseModel,
    pub architecture: Architecture,
    pub parameters: u64,
    /// The exact length of the model file.
    pub bytes: u64,
    pub rounds: u32,
    pub tokens: u64,
    pub loss_milli: Option<u32>,
    pub check_loss_milli: Option<u32>,
    /// "Person · Computer" as the computer that trained it wrote it. For a received
    /// model, show the sender's name from this computer's own group list instead:
    /// this text is the sender's word.
    pub trained_by: String,
    pub origin: ModelOrigin,
    /// The group it is shared with, if any.
    pub group: Option<GroupId>,
    pub created_unix_ms: u64,
}

impl ModelCard {
    /// The card for a model file this computer has just checked.
    pub fn from_facts(
        facts: &ModelFacts,
        name: String,
        trained_by: String,
        origin: ModelOrigin,
        group: Option<GroupId>,
        created_unix_ms: u64,
    ) -> Self {
        Self {
            id: facts.id,
            name,
            base: facts.header.base,
            architecture: facts.header.architecture,
            parameters: facts.parameters,
            bytes: facts.bytes,
            rounds: facts.header.rounds,
            tokens: facts.header.tokens,
            loss_milli: facts.header.loss_milli,
            check_loss_milli: facts.header.check_loss_milli,
            trained_by,
            origin,
            group,
            created_unix_ms,
        }
    }

    /// Checks a card before anything is fetched or stored for it: bounded plain
    /// names, a supported shape, and exactly the size that shape implies.
    pub fn validate(&self) -> Result<(), TrainingError> {
        check_name(&self.name).map_err(|_| TrainingError::InvalidModel)?;
        self.architecture.validate()?;
        let by = &self.trained_by;
        if by.chars().count() > MAX_TRAINED_BY_CHARS
            || by.chars().any(char::is_control)
            || (self.base == BaseModel::Starter) != self.architecture.is_table()
            || self.parameters != self.architecture.parameters() as u64
            || self.bytes != self.architecture.file_len() as u64
            || self.bytes > MAX_MODEL_BYTES as u64
            || self.loss_milli == Some(u32::MAX)
            || self.check_loss_milli == Some(u32::MAX)
        {
            return Err(TrainingError::InvalidModel);
        }
        Ok(())
    }

    /// Whether a fetched and checked file is the model this card describes.
    pub fn agrees_with(&self, facts: &ModelFacts) -> bool {
        self.id == facts.id
            && self.base == facts.header.base
            && self.architecture == facts.header.architecture
            && self.parameters == facts.parameters
            && self.bytes == facts.bytes
            && self.rounds == facts.header.rounds
            && self.tokens == facts.header.tokens
            && self.loss_milli == facts.header.loss_milli
            && self.check_loss_milli == facts.header.check_loss_milli
    }

    /// The card as it travels to the group: it says the model was trained by the
    /// sender, whatever the receiver later records as its own origin.
    #[must_use]
    pub fn offered_to(&self, group: GroupId) -> Self {
        Self {
            origin: ModelOrigin::TrainedHere,
            group: Some(group),
            ..self.clone()
        }
    }

    /// The card a receiver stores for a model fetched from `from`.
    #[must_use]
    pub fn received_from(&self, from: DeviceId) -> Self {
        Self {
            origin: ModelOrigin::Received { from },
            ..self.clone()
        }
    }
}

/// Whether one of the group's computers has a shared model yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub device: DeviceId,
    pub name: String,
    pub delivered: bool,
}

/// One model in "Your models".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryModel {
    pub card: ModelCard,
    /// Where the model file is on this computer.
    pub saved_at: String,
    /// The group's other computers, for a model shared from here. Empty otherwise.
    pub delivery: Vec<Delivery>,
}

impl LibraryModel {
    /// "On N of M computers": how many of the group's other computers have it.
    pub fn delivered(&self) -> (usize, usize) {
        (
            self.delivery.iter().filter(|d| d.delivered).count(),
            self.delivery.len(),
        )
    }

    /// Whether this computer offers the model to `group`: only a model trained
    /// here and shared with exactly that group. Received models are not passed on.
    pub fn offered_to(&self, group: GroupId) -> bool {
        self.card.origin == ModelOrigin::TrainedHere && self.card.group == Some(group)
    }

    /// Marks `device` as having the model, if it is one of the tracked computers.
    /// Returns whether anything changed.
    pub fn mark_delivered(&mut self, device: DeviceId) -> bool {
        self.delivery
            .iter_mut()
            .find(|d| d.device == device && !d.delivered)
            .is_some_and(|d| {
                d.delivered = true;
                true
            })
    }

    /// Brings the tracked computers in line with the group's current list, keeping
    /// what is already known. Computers that left are dropped; new ones start as
    /// not delivered. At most [`MAX_DELIVERIES`] are tracked.
    pub fn sync_delivery(&mut self, devices: &[(DeviceId, String)]) {
        let known = std::mem::take(&mut self.delivery);
        self.delivery = devices
            .iter()
            .take(MAX_DELIVERIES)
            .map(|(device, name)| Delivery {
                device: *device,
                name: name.clone(),
                delivered: known.iter().any(|d| d.device == *device && d.delivered),
            })
            .collect();
    }
}

/// Whether one more received model of `bytes` fits beside those already kept.
pub fn room_for_received(models: &[LibraryModel], bytes: u64) -> bool {
    let received: Vec<&LibraryModel> = models
        .iter()
        .filter(|model| matches!(model.card.origin, ModelOrigin::Received { .. }))
        .collect();
    let kept: u64 = received.iter().map(|model| model.card.bytes).sum();
    received.len() < MAX_RECEIVED_MODELS && kept.saturating_add(bytes) <= MAX_RECEIVED_BYTES
}

/// Trying a model: the person types a beginning and the model continues it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trial {
    pub model: ModelId,
    pub prompt: String,
    pub output: Option<String>,
    pub running: bool,
}

/// Checks the beginning typed for a trial. It may be empty: the model then starts
/// on its own.
pub fn check_trial_prompt(prompt: &str) -> Result<(), TrainingError> {
    if prompt.len() > MAX_TRIAL_PROMPT_BYTES {
        return Err(TrainingError::InvalidSetting(format!(
            "Type a beginning of at most {MAX_TRIAL_PROMPT_BYTES} bytes."
        )));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::training::{ModelHeader, encode_model, inspect_model};

    pub(crate) const MICRO: Architecture = Architecture {
        context: 8,
        width: 8,
        layers: 1,
        heads: 2,
    };

    pub(crate) fn micro_file(fill: f32) -> Vec<u8> {
        encode_model(
            &ModelHeader {
                base: BaseModel::Small,
                architecture: MICRO,
                rounds: 40,
                tokens: 5120,
                loss_milli: Some(2100),
                check_loss_milli: Some(2300),
            },
            &vec![fill; MICRO.parameters()],
        )
        .unwrap()
    }

    pub(crate) fn card_for(file: &[u8]) -> ModelCard {
        ModelCard::from_facts(
            &inspect_model(file).unwrap(),
            "Garden notes".into(),
            "Ada · Kitchen laptop".into(),
            ModelOrigin::TrainedHere,
            Some(GroupId([7; 32])),
            1_700_000_000_000,
        )
    }

    fn library_model(card: ModelCard) -> LibraryModel {
        LibraryModel {
            card,
            saved_at: "/models/x.model".into(),
            delivery: Vec::new(),
        }
    }

    #[test]
    fn a_card_built_from_a_checked_file_is_valid_and_agrees_with_it() {
        let file = micro_file(0.25);
        let facts = inspect_model(&file).unwrap();
        let card = card_for(&file);
        assert_eq!(card.validate(), Ok(()));
        assert!(card.agrees_with(&facts));
        assert_eq!(card.bytes, file.len() as u64);
        assert_eq!(card.parameters, 5304);
        assert_eq!((card.rounds, card.tokens), (40, 5120));
        let json = serde_json::to_string(&card).unwrap();
        assert_eq!(serde_json::from_str::<ModelCard>(&json).unwrap(), card);
        let other = inspect_model(&micro_file(0.5)).unwrap();
        assert!(!card.agrees_with(&other));
    }

    #[test]
    fn a_peer_card_with_any_wrong_or_oversized_field_is_refused() {
        let card = card_for(&micro_file(0.25));
        let changes: Vec<Change> = vec![
            Box::new(|c| c.name.clear()),
            Box::new(|c| c.name = "x".repeat(MAX_NAME + 1)),
            Box::new(|c| c.name = "two\nlines".into()),
            Box::new(|c| c.name = " padded ".into()),
            Box::new(|c| c.trained_by = "x".repeat(MAX_TRAINED_BY_CHARS + 1)),
            Box::new(|c| c.trained_by = "bell\u{7}".into()),
            Box::new(|c| c.base = BaseModel::Starter),
            Box::new(|c| c.architecture = Architecture::STARTER),
            Box::new(|c| c.architecture.width = 9),
            Box::new(|c| c.architecture.context = 16),
            Box::new(|c| c.parameters += 1),
            Box::new(|c| c.bytes += 1),
            Box::new(|c| c.bytes = u64::MAX),
            Box::new(|c| c.loss_milli = Some(u32::MAX)),
            Box::new(|c| c.check_loss_milli = Some(u32::MAX)),
        ];
        for (index, change) in changes.iter().enumerate() {
            let mut altered = card.clone();
            change(&mut altered);
            assert_eq!(
                altered.validate(),
                Err(TrainingError::InvalidModel),
                "change {index}"
            );
        }
        let mut json = serde_json::to_value(&card).unwrap();
        json["run"] = "a command".into();
        assert!(serde_json::from_value::<ModelCard>(json).is_err());
        // An empty "trained by" and no losses are fine.
        let mut plain = card;
        plain.trained_by.clear();
        plain.loss_milli = None;
        assert_eq!(plain.validate(), Ok(()));
    }

    const MAX_NAME: usize = crate::training::MAX_NAME_CHARS;
    type Change = Box<dyn Fn(&mut ModelCard)>;

    #[test]
    fn offered_and_received_cards_record_whose_word_they_are() {
        let card = ModelCard {
            group: None,
            ..card_for(&micro_file(0.25))
        };
        let group = GroupId([9; 32]);
        let offered = card.offered_to(group);
        assert_eq!(offered.group, Some(group));
        assert_eq!(offered.origin, ModelOrigin::TrainedHere);
        let received = offered.received_from(DeviceId([3; 32]));
        assert_eq!(
            received.origin,
            ModelOrigin::Received {
                from: DeviceId([3; 32])
            }
        );
        assert_eq!(received.id, card.id);
        // Only a model trained here and shared with that very group is offered.
        assert!(library_model(offered.clone()).offered_to(group));
        assert!(!library_model(offered).offered_to(GroupId([8; 32])));
        assert!(!library_model(card).offered_to(group));
        assert!(!library_model(received).offered_to(group));
    }

    #[test]
    fn delivery_follows_the_group_list_and_never_forgets_a_delivery() {
        let mut model = library_model(card_for(&micro_file(0.25)));
        assert_eq!(model.delivered(), (0, 0));
        let a = (DeviceId([1; 32]), "Ada · Laptop".to_owned());
        let b = (DeviceId([2; 32]), "Ben · Desk".to_owned());
        let c = (DeviceId([3; 32]), "Cy · Tablet".to_owned());
        model.sync_delivery(&[a, b.clone()]);
        assert_eq!(model.delivered(), (0, 2));
        assert!(model.mark_delivered(b.0));
        assert!(!model.mark_delivered(b.0));
        assert!(!model.mark_delivered(c.0));
        assert_eq!(model.delivered(), (1, 2));
        model.sync_delivery(&[b, c]);
        assert_eq!(model.delivered(), (1, 2));
        assert!(model.delivery[0].delivered && !model.delivery[1].delivered);
        assert_eq!(model.delivery[1].name, "Cy · Tablet");
        let many: Vec<_> = (0..100u8)
            .map(|n| (DeviceId([n; 32]), format!("Computer {n}")))
            .collect();
        model.sync_delivery(&many);
        assert_eq!(model.delivery.len(), MAX_DELIVERIES);
        assert_eq!(model.delivered(), (1, MAX_DELIVERIES));
    }

    #[test]
    fn received_models_have_a_quota_and_nothing_is_evicted() {
        let file = micro_file(0.25);
        let received = card_for(&file).received_from(DeviceId([3; 32]));
        let mine = library_model(card_for(&file));
        // Models trained here do not count against the quota.
        let mut models = vec![mine; 200];
        assert!(room_for_received(&models, MAX_RECEIVED_BYTES));
        assert!(!room_for_received(&models, MAX_RECEIVED_BYTES + 1));
        models.extend(vec![
            library_model(received.clone());
            MAX_RECEIVED_MODELS - 1
        ]);
        assert!(room_for_received(&models, 1));
        models.push(library_model(received.clone()));
        assert!(!room_for_received(&models, 1));
        let big = library_model(ModelCard {
            bytes: MAX_RECEIVED_BYTES - 10,
            ..received
        });
        assert!(room_for_received(std::slice::from_ref(&big), 10));
        assert!(!room_for_received(&[big], 11));
    }

    #[test]
    fn a_trial_beginning_is_bounded_and_may_be_empty() {
        assert_eq!(check_trial_prompt(""), Ok(()));
        assert_eq!(
            check_trial_prompt(&"x".repeat(MAX_TRIAL_PROMPT_BYTES)),
            Ok(())
        );
        assert!(check_trial_prompt(&"x".repeat(MAX_TRIAL_PROMPT_BYTES + 1)).is_err());
    }
}

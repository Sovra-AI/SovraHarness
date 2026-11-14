//! Giving a finished model to the group, one bounded part at a time.
//!
//! Delivery is pull-only: a computer learns of a model from a card (an offer or
//! an announcement, both only hints) and then asks for the bytes itself. Every
//! part is checked against the card before it is kept, and the assembled file must
//! hash to the card's id and pass the model-file checks before it is stored.

use crate::group::{Bytes, GroupId};

use super::{
    LibraryModel, MAX_MODEL_PART_BYTES, MAX_OFFER_HAVE, MAX_OFFERED_MODELS, ModelCard, ModelId,
    TrainingError, TrainingPeerRequest, TrainingPeerResponse, inspect_model, model_id,
};

/// The cards to offer a member of `group` who already holds `have`: models
/// trained here and shared with exactly that group, at most
/// [`MAX_OFFERED_MODELS`]. Anything left over goes out on the next ask.
pub fn offers(models: &[LibraryModel], group: GroupId, have: &[ModelId]) -> Vec<ModelCard> {
    models
        .iter()
        .filter(|model| model.offered_to(group) && !have.contains(&model.card.id))
        .take(MAX_OFFERED_MODELS)
        .map(|model| model.card.offered_to(group))
        .collect()
}

/// The ids to send with an `Offers` request: the models already here, newest
/// first as listed, at most [`MAX_OFFER_HAVE`].
pub fn held(models: &[LibraryModel]) -> Vec<ModelId> {
    models
        .iter()
        .map(|model| model.card.id)
        .take(MAX_OFFER_HAVE)
        .collect()
}

/// The reply to `FetchModel { offset }` for a model file being served: the next
/// part of at most [`MAX_MODEL_PART_BYTES`]. `Unavailable` when the offset is not
/// inside the file.
pub fn model_part(file: &[u8], offset: u64) -> TrainingPeerResponse {
    let Some(rest) = usize::try_from(offset)
        .ok()
        .and_then(|offset| file.get(offset..))
        .filter(|rest| !rest.is_empty())
    else {
        return TrainingPeerResponse::Unavailable;
    };
    TrainingPeerResponse::ModelPart {
        total: file.len() as u64,
        bytes: Bytes(rest[..rest.len().min(MAX_MODEL_PART_BYTES)].to_vec()),
    }
}

/// One model being fetched. Drop it on any error and start again later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAssembly {
    card: ModelCard,
    bytes: Vec<u8>,
}

impl ModelAssembly {
    /// Starts a fetch for a card from a member of `group`. The card must be valid
    /// and shared with that very group.
    pub fn new(card: ModelCard, group: GroupId) -> Result<Self, TrainingError> {
        card.validate()?;
        if card.group != Some(group) {
            return Err(TrainingError::InvalidModel);
        }
        Ok(Self {
            card,
            bytes: Vec::new(),
        })
    }

    pub fn card(&self) -> &ModelCard {
        &self.card
    }

    /// Bytes received so far, which is also the next offset to ask for.
    pub fn received(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// The next request to send.
    pub fn request(&self) -> TrainingPeerRequest {
        TrainingPeerRequest::FetchModel {
            model: self.card.id,
            offset: self.received(),
        }
    }

    /// Takes the reply to [`Self::request`]. The total must be the card's size on
    /// every part, and a part must be non-empty, within the part limit, and not
    /// run past the end; all of that is checked before the part is kept. Returns
    /// the whole file once the last part is in and the file hashes to the card's
    /// id, passes the model-file checks and agrees with the card.
    pub fn accept(&mut self, total: u64, part: &Bytes) -> Result<Option<Vec<u8>>, TrainingError> {
        let part = &part.0;
        let after = self.received() + part.len() as u64;
        if total != self.card.bytes
            || part.is_empty()
            || part.len() > MAX_MODEL_PART_BYTES
            || after > total
        {
            return Err(TrainingError::InvalidModel);
        }
        self.bytes.extend_from_slice(part);
        if after < total {
            return Ok(None);
        }
        let file = std::mem::take(&mut self.bytes);
        if model_id(&file) != self.card.id || !self.card.agrees_with(&inspect_model(&file)?) {
            return Err(TrainingError::InvalidModel);
        }
        Ok(Some(file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::DeviceId;
    use crate::training::library::tests::{card_for, micro_file};
    use crate::training::{BaseModel, MAX_MODEL_BYTES, ModelHeader, ModelOrigin, encode_model};

    const GROUP: GroupId = GroupId([7; 32]);

    fn library(card: ModelCard) -> LibraryModel {
        LibraryModel {
            card,
            saved_at: String::new(),
            delivery: Vec::new(),
        }
    }

    fn fetch(file: &[u8], assembly: &mut ModelAssembly) -> Result<Vec<u8>, TrainingError> {
        loop {
            let TrainingPeerRequest::FetchModel { model, offset } = assembly.request() else {
                panic!("a fetch asks for model bytes");
            };
            assert_eq!(model, assembly.card().id);
            let TrainingPeerResponse::ModelPart { total, bytes } = model_part(file, offset) else {
                panic!("the offset is inside the file");
            };
            if let Some(done) = assembly.accept(total, &bytes)? {
                return Ok(done);
            }
        }
    }

    #[test]
    fn only_models_trained_here_and_shared_with_that_group_are_offered() {
        let shared = card_for(&micro_file(0.1));
        let other_group = ModelCard {
            group: Some(GroupId([8; 32])),
            ..card_for(&micro_file(0.2))
        };
        let private = ModelCard {
            group: None,
            ..card_for(&micro_file(0.3))
        };
        let received = card_for(&micro_file(0.4)).received_from(DeviceId([3; 32]));
        let models: Vec<LibraryModel> = [shared.clone(), other_group, private, received]
            .into_iter()
            .map(library)
            .collect();
        assert_eq!(offers(&models, GROUP, &[]), std::slice::from_ref(&shared));
        assert_eq!(offers(&models, GROUP, &[shared.id]), []);
        assert_eq!(offers(&models, GroupId([1; 32]), &[]), []);
        assert_eq!(held(&models).len(), 4);

        let many: Vec<LibraryModel> = (0..100u8)
            .map(|n| library(card_for(&micro_file(f32::from(n)))))
            .collect();
        let offered = offers(&many, GROUP, &[]);
        assert_eq!(offered.len(), MAX_OFFERED_MODELS);
        assert!(
            TrainingPeerResponse::Offered(offered.clone())
                .validate()
                .is_ok()
        );
        let have: Vec<ModelId> = offered.iter().map(|card| card.id).collect();
        let next = offers(&many, GROUP, &have);
        assert!(next.iter().all(|card| !have.contains(&card.id)));
        assert_eq!(held(&many).len(), MAX_OFFER_HAVE);
        assert!(
            TrainingPeerRequest::Offers { have: held(&many) }
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn a_small_model_arrives_in_one_part_and_a_large_one_in_several() {
        let file = micro_file(0.25);
        let mut assembly = ModelAssembly::new(card_for(&file), GROUP).unwrap();
        assert_eq!(assembly.received(), 0);
        assert_eq!(fetch(&file, &mut assembly).unwrap(), file);

        let architecture = BaseModel::Medium.architecture();
        let big = encode_model(
            &ModelHeader::fresh(BaseModel::Medium),
            &vec![0.125; architecture.parameters()],
        )
        .unwrap();
        assert!(big.len() > MAX_MODEL_PART_BYTES && big.len() <= MAX_MODEL_BYTES);
        let TrainingPeerResponse::ModelPart { total, bytes } = model_part(&big, 0) else {
            panic!("the first part");
        };
        assert_eq!(total, big.len() as u64);
        assert_eq!(bytes.0.len(), MAX_MODEL_PART_BYTES);
        let part = TrainingPeerResponse::ModelPart { total, bytes };
        assert!(part.validate().is_ok());
        let wire =
            serde_json::to_vec(&crate::group::PeerResponse::Training { work: part }).unwrap();
        assert!(wire.len() <= crate::group::MAX_PEER_MESSAGE_BYTES);
        let mut assembly = ModelAssembly::new(card_for(&big), GROUP).unwrap();
        assert_eq!(fetch(&big, &mut assembly).unwrap(), big);

        assert_eq!(
            model_part(&file, file.len() as u64),
            TrainingPeerResponse::Unavailable
        );
        assert_eq!(
            model_part(&file, u64::MAX),
            TrainingPeerResponse::Unavailable
        );
        assert_eq!(model_part(&[], 0), TrainingPeerResponse::Unavailable);
    }

    #[test]
    fn a_fetch_refuses_wrong_cards_totals_parts_and_contents() {
        let file = micro_file(0.25);
        let card = card_for(&file);
        // The card must be valid and for this group.
        assert!(ModelAssembly::new(card.clone(), GroupId([8; 32])).is_err());
        let mut private = card.clone();
        private.group = None;
        assert!(ModelAssembly::new(private, GROUP).is_err());
        let mut oversized = card.clone();
        oversized.bytes = MAX_MODEL_BYTES as u64 + 1;
        assert!(ModelAssembly::new(oversized, GROUP).is_err());

        let start = || ModelAssembly::new(card.clone(), GROUP).unwrap();
        let whole = Bytes(file.clone());
        let total = file.len() as u64;
        assert!(start().accept(total + 1, &whole).is_err());
        assert!(start().accept(total - 1, &whole).is_err());
        assert!(start().accept(total, &Bytes(Vec::new())).is_err());
        let mut long = file.clone();
        long.push(0);
        assert!(start().accept(total, &Bytes(long)).is_err());
        assert!(
            start()
                .accept(total, &Bytes(vec![0; MAX_MODEL_PART_BYTES + 1]))
                .is_err()
        );
        // The total may not change between parts, and parts may not overrun.
        let mut assembly = start();
        let half = file.len() / 2;
        assert_eq!(
            assembly.accept(total, &Bytes(file[..half].to_vec())),
            Ok(None)
        );
        assert_eq!(assembly.received(), half as u64);
        assert!(
            assembly
                .clone()
                .accept(total + 4, &Bytes(file[half..].to_vec()))
                .is_err()
        );
        assert!(assembly.clone().accept(total, &whole).is_err());
        assert_eq!(
            assembly.accept(total, &Bytes(file[half..].to_vec())),
            Ok(Some(file.clone()))
        );

        // Right size, wrong bytes: the hash does not match the card.
        let mut changed = file.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        assert_eq!(
            start().accept(total, &Bytes(changed)),
            Err(TrainingError::InvalidModel)
        );
        // A card that names the file's real hash but lies about what it holds.
        let mut lying = card.clone();
        lying.rounds += 1;
        assert!(
            ModelAssembly::new(lying, GROUP)
                .unwrap()
                .accept(total, &whole)
                .is_err()
        );
        // A card whose id is the hash of a damaged file of the right length.
        let mut damaged = file;
        damaged[0] = b'X';
        let mut forged = card.clone();
        forged.id = model_id(&damaged);
        assert_eq!(
            ModelAssembly::new(forged, GROUP)
                .unwrap()
                .accept(total, &Bytes(damaged)),
            Err(TrainingError::InvalidModel)
        );
        assert_eq!(card.origin, ModelOrigin::TrainedHere);
    }
}

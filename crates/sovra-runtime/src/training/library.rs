//! "Your models": finished models on this computer, trying one, sharing one.
//!
//! A model is a file named by its own content plus a small card. Nothing in
//! here ever runs a model as a program: a model only continues text.

use sovra_core::{
    group::{DeviceId, GroupId},
    training::{
        LibraryModel, MAX_LIBRARY_MODELS, MAX_TRAINED_BY_CHARS, ModelCard, ModelFacts, ModelId,
        ModelOrigin, Trial, sample_text,
    },
};
use tokio::task::JoinError;

use super::{Actor, storage};
use crate::private_storage::{write_private, write_private_bytes};

/// How much a model writes when a person tries it.
const TRIAL_BYTES: u16 = 240;
/// A little variety without losing the thread, as in the run's own samples.
const TRIAL_TEMPERATURE_MILLI: u32 = 800;
const UNNAMED: &str = "My model";
const THIS_COMPUTER: &str = "This computer";

pub(super) fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

impl Actor {
    /// Reads the library from disk into the state.
    pub(super) async fn load_library(&mut self) -> Result<(), String> {
        let Some(root) = self.root() else {
            return Ok(());
        };
        let models = tokio::task::spawn_blocking(move || storage::scan_library(&root))
            .await
            .map_err(|_| "Could not read your saved models.".to_owned())??;
        self.state.models = models;
        Ok(())
    }

    /// A library model's file, checked completely by the engine.
    pub(super) async fn read_library_model(
        &mut self,
        model: ModelId,
    ) -> Result<(ModelFacts, Vec<u8>), String> {
        let (Some(root), Some(engine)) = (self.root(), self.engine()) else {
            return Err("Training is unavailable.".into());
        };
        tokio::task::spawn_blocking(move || {
            let file = storage::read_model(&root, model)?;
            let facts = engine.inspect(&file).map_err(|error| error.to_string())?;
            Ok((facts, file))
        })
        .await
        .map_err(|_| "That model's file could not be read. Nothing was changed.".to_owned())?
    }

    /// Writes the run's model as it is now into the library. `group` is the
    /// group it was trained with, which it is then shared with.
    pub(super) async fn export(&mut self, group: Option<GroupId>) -> Result<(), String> {
        let engine = self.engine().ok_or("Training is unavailable.")?;
        let session = self
            .session
            .take()
            .ok_or("Resume the saved training to continue.")?;
        let (session, exported) = tokio::task::spawn_blocking(move || {
            let exported = session
                .export()
                .and_then(|file| engine.inspect(&file).map(|facts| (facts, file)));
            (session, exported)
        })
        .await
        .map_err(|_| "The training worker stopped.".to_owned())?;
        self.session = Some(session);
        let (facts, file) = exported.map_err(|error| error.to_string())?;
        let name = self
            .state
            .plan
            .as_ref()
            .map_or_else(|| UNNAMED.to_owned(), |plan| plan.name.clone());
        let trained_by: String = self
            .groups
            .identity()
            .await
            .unwrap_or_else(|| THIS_COMPUTER.to_owned())
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_TRAINED_BY_CHARS)
            .collect();
        let card = ModelCard::from_facts(
            &facts,
            name,
            trained_by,
            ModelOrigin::TrainedHere,
            group,
            unix_ms(),
        );
        card.validate().map_err(|error| error.to_string())?;
        self.store_model(card, file).await?;
        self.ending.exported = Some(facts.id);
        self.write_notes().await?;
        if group.is_some() {
            // Trained with the group: every computer of it gets the model.
            self.announce(facts.id).await;
        }
        Ok(())
    }

    /// Puts a checked model file and its card on disk and in the list. A model
    /// that is already in the library is left exactly as it is.
    pub(super) async fn store_model(
        &mut self,
        card: ModelCard,
        file: Vec<u8>,
    ) -> Result<(), String> {
        let root = self.root().ok_or("Training is unavailable.")?;
        let id = card.id;
        if self.state.model(id).is_some() {
            return Ok(());
        }
        let directory = storage::models_directory(&root);
        let path = directory.join(storage::model_filename(id));
        let mut model = LibraryModel {
            card,
            saved_at: path.display().to_string(),
            delivery: Vec::new(),
        };
        // Only a model shared from here keeps track of who has it.
        if let Some(group) = model.card.group
            && model.card.origin == ModelOrigin::TrainedHere
        {
            model.sync_delivery(&self.group_devices(group).await);
        }
        // The file first: a card is only ever listed with its model beside it.
        write_private_bytes(&directory, &storage::model_filename(id), file).await?;
        write_private(&directory, &storage::card_filename(id), &model).await?;
        self.state.models.insert(0, model);
        self.state.models.truncate(MAX_LIBRARY_MODELS);
        Ok(())
    }

    /// The other computers of `group`, if it is this computer's current group.
    pub(super) async fn group_devices(&mut self, group: GroupId) -> Vec<(DeviceId, String)> {
        self.groups
            .snapshot()
            .await
            .filter(|current| current.id == group)
            .map(|current| {
                current
                    .peers
                    .into_iter()
                    .map(|peer| (peer.device, peer.name))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Offers a model trained here to the current group. The card records it,
    /// every computer of the group is told at once, and each fetches it itself.
    pub(super) async fn share(&mut self, model: ModelId) -> Result<(), String> {
        let root = self.root().ok_or("Training is unavailable.")?;
        let group = self
            .groups
            .snapshot()
            .await
            .ok_or("Create or join a group first.")?;
        let devices: Vec<(DeviceId, String)> = group
            .peers
            .iter()
            .map(|peer| (peer.device, peer.name.clone()))
            .collect();
        let entry = self
            .state
            .models
            .iter_mut()
            .find(|entry| entry.card.id == model)
            .ok_or("That model is no longer in Your models.")?;
        if entry.card.origin != ModelOrigin::TrainedHere {
            return Err(
                "Only models trained on this computer can be shared from here. The computer that trained it shares it with the group."
                    .into(),
            );
        }
        let mut shared = entry.clone();
        shared.card.group = Some(group.id);
        shared.sync_delivery(&devices);
        write_private(
            &storage::models_directory(&root),
            &storage::card_filename(model),
            &shared,
        )
        .await?;
        *entry = shared;
        self.state.group = Some(group);
        self.apply_peer_status();
        self.announce(model).await;
        Ok(())
    }

    /// Lets a library model continue `prompt`, on a blocking task. One at a time.
    pub(super) fn try_model(&mut self, model: ModelId, prompt: String) -> Result<(), String> {
        if self.trial_task.is_some() {
            self.state.notice = Some("Wait for the model to finish writing first.".into());
            return Ok(());
        }
        let (Some(root), Some(engine)) = (self.root(), self.engine()) else {
            return Ok(());
        };
        if self.state.model(model).is_none() {
            return Err("That model is no longer in Your models.".into());
        }
        // A new seed each time, so trying again writes something new.
        self.trial_seed = self.trial_seed.wrapping_add(1);
        let seed = unix_ms() ^ self.trial_seed.rotate_left(32);
        let beginning = prompt.clone();
        self.trial_task = Some(tokio::task::spawn_blocking(move || {
            let file = storage::read_model(&root, model)?;
            engine
                .generate(
                    &file,
                    &beginning,
                    TRIAL_BYTES,
                    TRIAL_TEMPERATURE_MILLI,
                    seed,
                )
                .map(|text| sample_text(text.as_bytes()))
                .map_err(|error| error.to_string())
        }));
        self.state.trial = Some(Trial {
            model,
            prompt,
            output: None,
            running: true,
        });
        Ok(())
    }

    pub(super) fn finish_trial(&mut self, result: Result<Result<String, String>, JoinError>) {
        let Some(trial) = &mut self.state.trial else {
            return;
        };
        trial.running = false;
        match result {
            Ok(Ok(text)) => trial.output = Some(text),
            Ok(Err(error)) => self.state.error = Some(error),
            Err(_) => {
                self.state.error =
                    Some("The model could not finish writing. Nothing was changed.".into());
            }
        }
    }
}

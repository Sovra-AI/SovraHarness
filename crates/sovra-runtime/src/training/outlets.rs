//! Copies of finished models outside the library: a storage bucket and the
//! Ollama on this computer.
//!
//! The person's choices live in `outlets.json` beside the runs; the bucket's
//! secret key lives in the keychain and is read only when a copy is sent. Each
//! copy is an owned task, so the page stays live while a file travels and
//! closing the app drops whatever is still on its way. Only a finished model
//! and facts about it are ever sent: never a run, and never the text.

use std::sync::Arc;

use sovra_core::training::{
    BucketFile, BucketSettings, CopyState, ModelId, ModelOrigin, ModelOutletState, ModelOutlets,
    OLLAMA_NEEDED, OLLAMA_OLDEST, OllamaModel, Outlet, RepeatCheck, RepeatLevel, exports_to_gguf,
    gguf_needs_output_offsets, model_readme, ollama_name, ollama_refusal, outlet_name, to_gguf,
};

use super::{Actor, library::unix_ms, storage};
use crate::private_storage::{read_private_bounded, write_private};

const OUTLETS_FILE: &str = "outlets.json";
/// Far above what the bounded lists inside can come to.
const MAX_OUTLETS_BYTES: u64 = 512 * 1024;
const NO_OUTLETS: &str = "Copying models out of Sovra is unavailable in this build.";
const NO_BUCKET: &str = "Set up your storage bucket first.";
const NO_MODEL: &str = "That model is no longer in Your models.";

/// How one copy ended.
pub(super) struct Sent {
    model: ModelId,
    outlet: Outlet,
    /// Where the copy is, or why there is none.
    result: Result<String, String>,
}

impl Actor {
    fn outlets(&self) -> Option<Arc<dyn ModelOutlets>> {
        self.config
            .as_ref()
            .and_then(|config| config.outlets.clone())
    }

    /// Reads the choices and what has been sent. A file that is missing or
    /// unreadable means nothing is on, which is the safe way to be wrong.
    pub(super) async fn load_outlets(&mut self) {
        let Some(root) = self.root() else {
            return;
        };
        // Copies on their way now are kept: the file never records those.
        let working: Vec<_> = self
            .state
            .outlets
            .copies
            .iter()
            .filter(|copies| {
                matches!(copies.bucket, Some(CopyState::Working))
                    || matches!(copies.ollama, Some(CopyState::Working))
            })
            .cloned()
            .collect();
        let saved = read_private_bounded(&root, OUTLETS_FILE, MAX_OUTLETS_BYTES)
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ModelOutletState>(&bytes).ok())
            .unwrap_or_default();
        let mut outlets = saved.settled(&self.state.models);
        for copies in working {
            for (outlet, state) in [
                (Outlet::Bucket, &copies.bucket),
                (Outlet::Ollama, &copies.ollama),
            ] {
                if matches!(state, Some(CopyState::Working)) {
                    outlets.record(copies.model, outlet, CopyState::Working);
                }
            }
        }
        outlets.bucket_key_saved = match self.outlets() {
            Some(port) => tokio::task::spawn_blocking(move || port.bucket_secret())
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .is_some(),
            None => false,
        };
        self.state.outlets = outlets;
    }

    async fn save_outlets(&mut self) -> Result<(), String> {
        let root = self.root().ok_or("Training is unavailable.")?;
        let saved = self.state.outlets.clone().settled(&self.state.models);
        write_private(&root, OUTLETS_FILE, &saved).await
    }

    /// Remembers the bucket. The secret key goes to the keychain first: if
    /// that fails, nothing about the bucket changes.
    pub(super) async fn save_bucket(
        &mut self,
        settings: BucketSettings,
        secret: Option<String>,
    ) -> Result<(), String> {
        let port = self.outlets().ok_or(NO_OUTLETS)?;
        let settings = settings.tidied();
        settings.validate().map_err(|error| error.to_string())?;
        if secret.is_none() && !self.state.outlets.bucket_key_saved {
            return Err("Give the secret access key for the bucket.".into());
        }
        if let Some(secret) = secret {
            tokio::task::spawn_blocking(move || port.set_bucket_secret(Some(&secret)))
                .await
                .map_err(|_| "The secret key could not be saved.".to_owned())??;
            self.state.outlets.bucket_key_saved = true;
        }
        self.state.outlets.bucket = Some(settings);
        self.save_outlets().await?;
        self.state.notice = Some(
            "Your storage bucket is saved. Turn on Back up finished models, or press Back up now on a model."
                .into(),
        );
        Ok(())
    }

    /// Forgets the bucket and its key. Files already in the bucket stay there.
    pub(super) async fn forget_bucket(&mut self) -> Result<(), String> {
        if let Some(port) = self.outlets() {
            tokio::task::spawn_blocking(move || port.set_bucket_secret(None))
                .await
                .map_err(|_| "The secret key could not be removed.".to_owned())??;
        }
        self.state.outlets.bucket = None;
        self.state.outlets.bucket_key_saved = false;
        self.state.outlets.backup_automatically = false;
        self.save_outlets().await?;
        self.state.notice = Some(
            "Sovra has forgotten the bucket and its key. Files already in the bucket are still there."
                .into(),
        );
        Ok(())
    }

    /// Keeps a new model's privacy check with its other details on this
    /// computer. Losing it costs only the line on the page.
    pub(super) async fn record_check(&mut self, model: ModelId, check: RepeatCheck) {
        self.state.outlets.record_check(model, check);
        if let Err(error) = self.save_outlets().await {
            tracing::warn!(%error, "a model's privacy check could not be saved");
        }
    }

    pub(super) async fn set_automatic_backup(&mut self, on: bool) -> Result<(), String> {
        if on {
            self.outlets().ok_or(NO_OUTLETS)?;
            if self.state.outlets.bucket.is_none() || !self.state.outlets.bucket_key_saved {
                return Err(NO_BUCKET.into());
            }
        }
        self.state.outlets.backup_automatically = on;
        self.save_outlets().await
    }

    pub(super) async fn set_automatic_ollama(&mut self, on: bool) -> Result<(), String> {
        if on {
            self.outlets().ok_or(NO_OUTLETS)?;
        }
        self.state.outlets.ollama_automatically = on;
        self.save_outlets().await
    }

    /// Starts copying one library model to the bucket: the model file, its
    /// GGUF form when it has one, and a description.
    pub(super) fn back_up(&mut self, model: ModelId) -> Result<(), String> {
        let port = self.outlets().ok_or(NO_OUTLETS)?;
        let root = self.root().ok_or("Training is unavailable.")?;
        let bucket = self.state.outlets.bucket.clone().ok_or(NO_BUCKET)?;
        let entry = self.state.model(model).ok_or(NO_MODEL)?.clone();
        let repeats = self.state.outlets.repeats(model);
        if self.state.outlets.working(model, Outlet::Bucket) {
            return Ok(());
        }
        self.state
            .outlets
            .record(model, Outlet::Bucket, CopyState::Working);
        self.outlet_tasks.spawn(async move {
            let result = async {
                let keychain = port.clone();
                let secret = tokio::task::spawn_blocking(move || keychain.bucket_secret())
                    .await
                    .map_err(|_| "The bucket's secret key could not be read.".to_owned())??
                    .ok_or(NO_BUCKET)?;
                let stem = outlet_name(&entry.card.name, model);
                let architecture = entry.card.architecture;
                let name = entry.card.name.clone();
                let readme = model_readme(&entry, &stem, repeats);
                let (file, gguf) = tokio::task::spawn_blocking(move || {
                    let file = storage::read_model(&root, model)?;
                    let gguf = exports_to_gguf(architecture)
                        .then(|| to_gguf(&file, &name).map_err(|error| error.to_string()))
                        .transpose()?;
                    Ok::<_, String>((file, gguf))
                })
                .await
                .map_err(|_| "That model's file could not be read.".to_owned())??;
                let mut files = vec![BucketFile {
                    key: bucket.object_key(&format!("{stem}.model")),
                    content_type: "application/octet-stream",
                    bytes: file,
                }];
                if let Some(gguf) = gguf {
                    files.push(BucketFile {
                        key: bucket.object_key(&format!("{stem}.gguf")),
                        content_type: "application/octet-stream",
                        bytes: gguf,
                    });
                }
                files.push(BucketFile {
                    key: bucket.object_key(&format!("{stem}.md")),
                    content_type: "text/markdown; charset=utf-8",
                    bytes: readme.into_bytes(),
                });
                for file in files {
                    port.put(&bucket, &secret, file).await?;
                }
                Ok(bucket.place(&format!("{stem}.model")))
            }
            .await;
            Sent {
                model,
                outlet: Outlet::Bucket,
                result,
            }
        });
        Ok(())
    }

    /// Starts handing one library model to the Ollama on this computer.
    pub(super) fn add_to_ollama(&mut self, model: ModelId) -> Result<(), String> {
        let port = self.outlets().ok_or(NO_OUTLETS)?;
        let root = self.root().ok_or("Training is unavailable.")?;
        let entry = self.state.model(model).ok_or(NO_MODEL)?;
        if let Some(why) = ollama_refusal(entry.card.architecture) {
            return Err(why.into());
        }
        if self.state.outlets.working(model, Outlet::Ollama) {
            return Ok(());
        }
        let name = ollama_name(&entry.card.name, model);
        let title = entry.card.name.clone();
        let context = entry.card.architecture.context;
        self.state
            .outlets
            .record(model, Outlet::Ollama, CopyState::Working);
        self.outlet_tasks.spawn(async move {
            let result = async {
                let (gguf, needs) = tokio::task::spawn_blocking(move || {
                    let file = storage::read_model(&root, model)?;
                    let needs =
                        if gguf_needs_output_offsets(&file).map_err(|error| error.to_string())? {
                            OLLAMA_NEEDED
                        } else {
                            OLLAMA_OLDEST
                        };
                    let gguf = to_gguf(&file, &title).map_err(|error| error.to_string())?;
                    Ok::<_, String>((gguf, needs))
                })
                .await
                .map_err(|_| "That model's file could not be read.".to_owned())??;
                port.add_to_ollama(OllamaModel {
                    name: name.clone(),
                    gguf,
                    context,
                    needs,
                })
                .await?;
                Ok(name)
            }
            .await;
            Sent {
                model,
                outlet: Outlet::Ollama,
                result,
            }
        });
        Ok(())
    }

    /// A model has just finished training here: send it wherever the person
    /// asked for finished models to go. Nothing here can fail the run; a copy
    /// that does not arrive is shown on the model.
    ///
    /// A model the privacy check found repeating the text is held back from
    /// automatic copies. Switching a copy on was a choice about models in
    /// general; sending one that gives the text back is a choice about this
    /// model, so it waits for the person to press the button themselves.
    pub(super) fn send_finished(&mut self, model: ModelId) {
        let Some(entry) = self.state.model(model) else {
            return;
        };
        if entry.card.origin != ModelOrigin::TrainedHere {
            return;
        }
        let architecture = entry.card.architecture;
        let wanted = [
            (Outlet::Bucket, self.state.outlets.backup_automatically),
            (
                Outlet::Ollama,
                self.state.outlets.ollama_automatically && ollama_refusal(architecture).is_none(),
            ),
        ];
        let repeats = self
            .state
            .outlets
            .repeats(model)
            .is_some_and(|check| check.level() != RepeatLevel::NotFound);
        for (outlet, on) in wanted {
            if !on {
                continue;
            }
            if repeats {
                self.state.outlets.record(model, outlet, CopyState::Held);
                continue;
            }
            let _ = match outlet {
                Outlet::Bucket => self.back_up(model),
                Outlet::Ollama => self.add_to_ollama(model),
            };
        }
    }

    pub(super) async fn finish_sent(&mut self, sent: Sent) {
        let name = self
            .state
            .model(sent.model)
            .map_or_else(|| "The model".to_owned(), |entry| entry.card.name.clone());
        let state = match sent.result {
            Ok(place) => {
                self.state.notice = Some(match sent.outlet {
                    Outlet::Bucket => format!("{name} is backed up to {place}."),
                    Outlet::Ollama => {
                        format!("{name} is in Ollama as {place}. Run it with: ollama run {place}")
                    }
                });
                CopyState::Done {
                    place,
                    at_unix_ms: unix_ms(),
                }
            }
            Err(why) => CopyState::Failed(why),
        };
        self.state.outlets.record(sent.model, sent.outlet, state);
        if let Err(error) = self.save_outlets().await {
            tracing::warn!(%error, "where a model was copied to could not be saved");
        }
    }
}

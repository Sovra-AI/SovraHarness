//! Where finished models can be sent besides "Your models": a storage bucket
//! the person owns, and the Ollama on this computer.
//!
//! Both are off until the person turns them on, and both carry only a finished
//! model and facts about it. The text a model learned from is never sent
//! anywhere by these. Everything here is plain data and checks; the sending is
//! behind the [`ModelOutlets`] port.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::{Architecture, LibraryModel, ModelId, TrainingError, exports_to_gguf};

/// Longest text kept for any one bucket setting.
pub const MAX_BUCKET_TEXT_CHARS: usize = 200;
/// Longest secret key accepted. Real ones are far shorter.
pub const MAX_BUCKET_SECRET_CHARS: usize = 512;
/// Models whose copies are remembered: one for each model the library can hold.
pub const MAX_COPY_RECORDS: usize = super::MAX_LIBRARY_MODELS;
/// Longest message kept about a copy that failed.
pub const MAX_COPY_MESSAGE_CHARS: usize = 300;
/// The oldest Ollama that reads every part of a model trained here.
pub const OLLAMA_NEEDED: [u32; 3] = [0, 40, 15];
/// The oldest Ollama that takes a model the way Sovra hands it over. Enough
/// for the rare model with no output offsets.
pub const OLLAMA_OLDEST: [u32; 3] = [0, 5, 5];

fn refuse(text: &str) -> TrainingError {
    TrainingError::InvalidSetting(text.to_owned())
}

/// A bucket on Amazon S3 or any service that speaks the same language
/// (Cloudflare R2, Backblaze B2, Wasabi, `MinIO` and others). The secret key is
/// not here: it lives in the operating system's keychain.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BucketSettings {
    /// The service's web address, such as `https://s3.eu-west-2.amazonaws.com`.
    pub endpoint: String,
    /// The region the service signs requests for: `us-east-1`, `auto` on R2.
    pub region: String,
    pub bucket: String,
    /// A folder inside the bucket; may be empty.
    pub folder: String,
    pub access_key_id: String,
}

impl BucketSettings {
    /// The same settings with surrounding spaces and the folder's outer slashes
    /// removed, as they are checked and saved.
    #[must_use]
    pub fn tidied(&self) -> Self {
        Self {
            endpoint: self.endpoint.trim().trim_end_matches('/').to_owned(),
            region: self.region.trim().to_owned(),
            bucket: self.bucket.trim().to_owned(),
            folder: self.folder.trim().trim_matches('/').to_owned(),
            access_key_id: self.access_key_id.trim().to_owned(),
        }
    }

    /// Checks every field, naming the one that is wrong in plain words.
    pub fn validate(&self) -> Result<(), TrainingError> {
        let plain = |text: &str| {
            !text.is_empty()
                && text.chars().count() <= MAX_BUCKET_TEXT_CHARS
                && !text.chars().any(|c| c.is_control() || c.is_whitespace())
        };
        let url = url::Url::parse(&self.endpoint)
            .ok()
            .filter(|url| url.host_str().is_some())
            .ok_or_else(|| {
                refuse(
                    "The storage address must be a web address such as https://s3.eu-west-2.amazonaws.com.",
                )
            })?;
        let loopback = match url.host() {
            Some(url::Host::Domain(name)) => name == "localhost",
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        };
        // A key sent over plain http can be read on the way. Only a service on
        // this computer itself may be reached that way.
        if !(url.scheme() == "https" || (url.scheme() == "http" && loopback)) {
            return Err(refuse(
                "The storage address must start with https://. Only a service on this computer may use http://.",
            ));
        }
        if !plain(&self.endpoint)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err(refuse(
                "The storage address must be only the service's address, with nothing after it.",
            ));
        }
        if !plain(&self.region) {
            return Err(refuse(
                "Give the storage region, such as us-east-1. Cloudflare R2 uses auto.",
            ));
        }
        // The rules every such service shares for bucket names.
        let bucket_ok = (3..=63).contains(&self.bucket.len())
            && self
                .bucket
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
            && self.bucket.starts_with(|c: char| c.is_ascii_alphanumeric())
            && self.bucket.ends_with(|c: char| c.is_ascii_alphanumeric());
        if !bucket_ok {
            return Err(refuse(
                "The bucket name must be 3 to 63 lower-case letters, digits, dots or hyphens.",
            ));
        }
        let folder_ok = self.folder.chars().count() <= MAX_BUCKET_TEXT_CHARS
            && !self.folder.chars().any(char::is_control)
            && !self
                .folder
                .split('/')
                .any(|part| part == "." || part == "..")
            && !self.folder.contains("//");
        if !folder_ok {
            return Err(refuse(
                "The folder must be a plain folder name such as sovra-models.",
            ));
        }
        if !plain(&self.access_key_id) {
            return Err(refuse("Give the access key ID for the bucket."));
        }
        Ok(())
    }

    /// The full name of a file called `name` inside the bucket's folder.
    pub fn object_key(&self, name: &str) -> String {
        if self.folder.is_empty() {
            name.to_owned()
        } else {
            format!("{}/{name}", self.folder)
        }
    }

    /// Where a file called `name` ends up, as the page shows it.
    pub fn place(&self, name: &str) -> String {
        format!("{}/{}", self.bucket, self.object_key(name))
    }
}

/// Checks a secret key typed for the bucket.
pub fn check_bucket_secret(secret: &str) -> Result<(), TrainingError> {
    if secret.is_empty()
        || secret.chars().count() > MAX_BUCKET_SECRET_CHARS
        || secret.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(refuse("Give the secret access key for the bucket."));
    }
    Ok(())
}

/// How one copy of a model stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyState {
    /// Being sent now. Never saved: after a restart nothing is being sent.
    Working,
    /// Sent. `place` is where it is: a bucket path, or the name Ollama knows it by.
    Done { place: String, at_unix_ms: u64 },
    /// Not sent, with the reason in plain words.
    Failed(String),
}

/// The copies of one library model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCopies {
    pub model: ModelId,
    #[serde(default)]
    pub bucket: Option<CopyState>,
    #[serde(default)]
    pub ollama: Option<CopyState>,
}

/// Which of the two places a copy goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outlet {
    Bucket,
    Ollama,
}

/// The person's choices about where finished models go, and what has been sent.
/// Saved on this computer; it holds no secret.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelOutletState {
    #[serde(default)]
    pub bucket: Option<BucketSettings>,
    /// Copy every model that finishes training here to the bucket.
    #[serde(default)]
    pub backup_automatically: bool,
    /// Add every model that finishes training here to Ollama.
    #[serde(default)]
    pub ollama_automatically: bool,
    #[serde(default)]
    pub copies: Vec<ModelCopies>,
    /// Whether a secret key is stored for the bucket. Read from the keychain
    /// when the page loads; never saved in this file.
    #[serde(skip)]
    pub bucket_key_saved: bool,
}

impl ModelOutletState {
    pub fn copies(&self, model: ModelId) -> Option<&ModelCopies> {
        self.copies.iter().find(|copies| copies.model == model)
    }

    /// Whether a copy of `model` is on its way to `outlet` right now.
    pub fn working(&self, model: ModelId, outlet: Outlet) -> bool {
        self.copies(model).is_some_and(|copies| {
            let state = match outlet {
                Outlet::Bucket => &copies.bucket,
                Outlet::Ollama => &copies.ollama,
            };
            matches!(state, Some(CopyState::Working))
        })
    }

    /// Records how a copy stands, newest model first, within the bound.
    pub fn record(&mut self, model: ModelId, outlet: Outlet, state: CopyState) {
        let state = match state {
            CopyState::Failed(why) => {
                CopyState::Failed(why.chars().take(MAX_COPY_MESSAGE_CHARS).collect())
            }
            other => other,
        };
        let at = self
            .copies
            .iter()
            .position(|copies| copies.model == model)
            .unwrap_or_else(|| {
                self.copies.insert(
                    0,
                    ModelCopies {
                        model,
                        bucket: None,
                        ollama: None,
                    },
                );
                0
            });
        match outlet {
            Outlet::Bucket => self.copies[at].bucket = Some(state),
            Outlet::Ollama => self.copies[at].ollama = Some(state),
        }
        self.copies.truncate(MAX_COPY_RECORDS);
    }

    /// The state as it is saved and as it is after a restart: nothing is being
    /// sent, and models no longer in the library are forgotten.
    #[must_use]
    pub fn settled(mut self, library: &[LibraryModel]) -> Self {
        let idle = |state: &mut Option<CopyState>| {
            if matches!(state, Some(CopyState::Working)) {
                *state = None;
            }
        };
        self.copies
            .retain(|copies| library.iter().any(|model| model.card.id == copies.model));
        for copies in &mut self.copies {
            idle(&mut copies.bucket);
            idle(&mut copies.ollama);
        }
        self.copies
            .retain(|copies| copies.bucket.is_some() || copies.ollama.is_some());
        self.copies.truncate(MAX_COPY_RECORDS);
        self
    }
}

/// A short name safe to use in a file name and as an Ollama model name: lower
/// case letters and digits from `name` joined by hyphens, then the start of the
/// model's fingerprint so two models with one name never collide.
pub fn outlet_name(name: &str, model: ModelId) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    let id: String = model.to_hex().chars().take(8).collect();
    if slug.is_empty() {
        format!("model-{id}")
    } else {
        format!("{slug}-{id}")
    }
}

/// The name Ollama lists a model under.
pub fn ollama_name(name: &str, model: ModelId) -> String {
    format!("sovra-{}", outlet_name(name, model))
}

/// Why a model of this shape cannot go to Ollama, if it cannot.
pub fn ollama_refusal(architecture: Architecture) -> Option<&'static str> {
    (!exports_to_gguf(architecture)).then_some(
        "A Starter model is a table of letter pairs, which Ollama cannot run. Small, Medium and Large models can be added.",
    )
}

/// A plain-text description of a model to keep beside its file: what it is,
/// how it was trained and how to use it. It names no text the model learned
/// from.
pub fn model_readme(model: &LibraryModel, file_stem: &str) -> String {
    let card = &model.card;
    let loss = |milli: Option<u32>| {
        milli.map_or_else(
            || "not measured".to_owned(),
            |milli| format!("{:.3}", f64::from(milli) / 1000.0),
        )
    };
    let mut text = format!(
        "# {name}\n\nA small language model trained with Sovra. It continues text in the style it was taught. It does not chat or follow instructions.\n\n| | |\n| --- | --- |\n| Starting point | {base} |\n| Values | {parameters} |\n| Reach | {reach} bytes |\n| Rounds trained | {rounds} |\n| Loss | {loss} |\n| Check loss | {check} |\n| Trained by | {by} |\n| Fingerprint | `{id}` |\n\n## Files\n\n- `{file_stem}.model`: the model in Sovra's own format.\n",
        name = card.name,
        base = card.base.info().title,
        parameters = card.parameters,
        reach = card.architecture.context,
        rounds = card.rounds,
        loss = loss(card.loss_milli),
        check = loss(card.check_loss_milli),
        by = card.trained_by,
        id = card.id.to_hex(),
    );
    if exports_to_gguf(card.architecture) {
        let _ = writeln!(
            text,
            "- `{file_stem}.gguf`: the same model for Ollama, llama.cpp and LM Studio (their `gpt2` kind, one token for each byte). It needs Ollama {}.{}.{} or newer.",
            OLLAMA_NEEDED[0], OLLAMA_NEEDED[1], OLLAMA_NEEDED[2]
        );
    }
    text.push_str(
        "\n## Limits\n\nThe model reads and writes plain bytes and can look back only as far as its reach. A small model trained on a small text can repeat parts of that text.\n",
    );
    text
}

/// One file to put in the bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketFile {
    /// The file's full name inside the bucket, folder included.
    pub key: String,
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

/// A model on its way to Ollama.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaModel {
    /// The name Ollama will list it under.
    pub name: String,
    pub gguf: Vec<u8>,
    /// How many bytes back the model can look: Ollama must not read further.
    pub context: u16,
    /// The oldest Ollama that can run this file.
    pub needs: [u32; 3],
}

/// Sends finished models out of Sovra. The composition root supplies it; core
/// only decides what is sent. Every failure comes back as a sentence a person
/// can act on, and never contains a key.
#[async_trait::async_trait]
pub trait ModelOutlets: Send + Sync {
    /// The bucket's secret key from the keychain, if one is stored. May wait
    /// on the operating system: call it off the async threads.
    fn bucket_secret(&self) -> Result<Option<String>, String>;
    /// Stores the secret key, or removes it with `None`. May wait likewise.
    fn set_bucket_secret(&self, secret: Option<&str>) -> Result<(), String>;
    /// Puts one file in the bucket, replacing a file of the same name.
    async fn put(
        &self,
        bucket: &BucketSettings,
        secret: &str,
        file: BucketFile,
    ) -> Result<(), String>;
    /// Makes the model known to the Ollama on this computer, starting Ollama
    /// if it is installed and not running.
    async fn add_to_ollama(&self, model: OllamaModel) -> Result<(), String>;
}

#[cfg(test)]
mod tests;

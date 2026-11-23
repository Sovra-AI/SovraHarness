//! Sending finished models out of Sovra: to a storage bucket that speaks the S3
//! language, and to the Ollama on this computer.
//!
//! The bucket side signs each request itself (AWS Signature Version 4) with
//! the hashing Sovra already carries, so no storage SDK is built in and the
//! app stays one executable. Requests go to the address the person typed and
//! nowhere else; redirects are refused, because a signed request must not be
//! replayed to another host. A key is never written to a log or an error.
//!
//! The Ollama side talks only to this computer: it hands the model file over
//! and asks Ollama to list it under a name.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sovra_core::models::{ModelConnection, ModelReadiness, ModelSecret, ProviderKind};
use sovra_core::training::{BucketFile, BucketSettings, ModelOutlets, OllamaModel};

use crate::sigv4::{
    Signing, authorization, error_code, error_text, hex, sha256, stamps, uri_encode,
};
use crate::{CredentialStore, endpoint};

/// The keychain entry holding the bucket's secret key.
const BUCKET_SECRET: &str = "training-bucket-secret";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A model file is at most 16 MiB; this allows for a slow connection.
const SEND_TIMEOUT: Duration = Duration::from_secs(300);

fn bucket_failure(status: u16, body: &str) -> String {
    let code = error_code(body);
    let what = match (status, code) {
        (_, Some("SignatureDoesNotMatch" | "InvalidAccessKeyId")) | (401, _) => {
            "The storage service did not accept the keys. Check the access key ID and the secret key."
        }
        (_, Some("RequestTimeTooSkewed")) => {
            "The storage service refused the upload because this computer's clock is wrong. Set the date and time, then try again."
        }
        (403, _) => {
            "The storage service refused the upload. Check that the keys may write to this bucket and folder."
        }
        (_, Some("NoSuchBucket")) | (404, _) => {
            "The bucket was not found. Check the bucket name and the storage address."
        }
        (_, Some("AuthorizationHeaderMalformed")) | (301 | 307 | 308 | 400, _) => {
            "The storage service did not accept the request. Check the storage address and the region."
        }
        _ => "The storage service could not take the file. Try again later.",
    };
    match code {
        Some(code) => format!("{what} (It answered {status} {code}.)"),
        None => format!("{what} (It answered {status}.)"),
    }
}

/// `major.minor.patch` out of a version such as `0.40.15` or `0.40.15-rc1`.
fn version(text: &str) -> Option<[u32; 3]> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let mut number = || {
        let part = parts.next()?;
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u32>().ok()
    };
    Some([number()?, number()?, number().unwrap_or(0)])
}

const OLLAMA_MISSING: &str = "Ollama is not running on this computer. Install it from ollama.com or start it, then try again.";

/// The real outlets: a bucket over HTTPS and the Ollama on this computer.
pub struct HttpModelOutlets {
    store: Arc<dyn CredentialStore>,
    /// Starts the local Ollama when it is installed and not running.
    readiness: Option<Arc<dyn ModelReadiness>>,
    ollama: ModelConnection,
}

impl std::fmt::Debug for HttpModelOutlets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Nothing inside is worth printing, and a key must never be.
        f.debug_struct("HttpModelOutlets").finish_non_exhaustive()
    }
}

impl HttpModelOutlets {
    pub fn new(
        store: Arc<dyn CredentialStore>,
        readiness: Option<Arc<dyn ModelReadiness>>,
    ) -> Self {
        Self {
            store,
            readiness,
            ollama: ModelConnection::new("ollama", ProviderKind::Ollama),
        }
    }

    /// The same, talking to an Ollama at `connection` instead of the usual
    /// local address.
    #[must_use]
    pub fn with_ollama(mut self, connection: ModelConnection) -> Self {
        self.ollama = connection;
        self
    }

    fn http(timeout: Duration) -> Result<reqwest::Client, String> {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Could not start a connection.".to_owned())
    }

    async fn ollama_version(&self, http: &reqwest::Client) -> Result<[u32; 3], String> {
        let response = http
            .get(endpoint(&self.ollama, "api/version"))
            .send()
            .await
            .map_err(|_| OLLAMA_MISSING.to_owned())?;
        if !response.status().is_success() {
            return Err(OLLAMA_MISSING.into());
        }
        let text = error_text(response).await;
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| value["version"].as_str().and_then(version))
            .ok_or_else(|| OLLAMA_MISSING.to_owned())
    }
}

#[async_trait::async_trait]
impl ModelOutlets for HttpModelOutlets {
    fn bucket_secret(&self) -> Result<Option<String>, String> {
        let secret = self.store.get(BUCKET_SECRET).map_err(|_| {
            "The bucket's secret key could not be read from the keychain.".to_owned()
        })?;
        Ok((!secret.0.is_empty()).then_some(secret.0))
    }

    fn set_bucket_secret(&self, secret: Option<&str>) -> Result<(), String> {
        let stored = match secret {
            Some(secret) => self
                .store
                .set(BUCKET_SECRET, &ModelSecret(secret.to_owned())),
            None => self.store.delete(BUCKET_SECRET),
        };
        stored.map_err(|_| "The bucket's secret key could not be saved in the keychain.".to_owned())
    }

    async fn put(
        &self,
        bucket: &BucketSettings,
        secret: &str,
        file: BucketFile,
    ) -> Result<(), String> {
        let unreachable = || {
            "Could not reach the storage service. Check the storage address and your internet connection."
                .to_owned()
        };
        let base = url::Url::parse(&bucket.endpoint).map_err(|_| unreachable())?;
        let host = match (base.host_str(), base.port()) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            (Some(host), None) => host.to_owned(),
            (None, _) => return Err(unreachable()),
        };
        // The bucket goes in the path, the form every such service accepts.
        let path = format!(
            "/{}/{}",
            uri_encode(&bucket.bucket, false),
            uri_encode(&file.key, true)
        );
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let (date, time) = stamps(now);
        let payload_hash = hex(&sha256(&file.bytes));
        let headers = [
            ("content-type", file.content_type.to_owned()),
            ("host", host),
            ("x-amz-content-sha256", payload_hash.clone()),
            ("x-amz-date", time.clone()),
        ];
        let authorization = authorization(&Signing {
            method: "PUT",
            path: &path,
            query: "",
            headers: &headers,
            payload_hash: &payload_hash,
            region: &bucket.region,
            access_key_id: &bucket.access_key_id,
            secret,
            date: &date,
            time: &time,
        });
        let address = format!("{}://{}{path}", base.scheme(), headers[1].1);
        let response = Self::http(SEND_TIMEOUT)?
            .put(address)
            .header("content-type", file.content_type)
            .header("x-amz-content-sha256", &payload_hash)
            .header("x-amz-date", &time)
            .header("authorization", authorization)
            .body(file.bytes)
            .send()
            .await
            .map_err(|_| unreachable())?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(bucket_failure(status.as_u16(), &error_text(response).await))
    }

    async fn add_to_ollama(&self, model: OllamaModel) -> Result<(), String> {
        if let Some(readiness) = &self.readiness {
            readiness
                .ensure_ready(&self.ollama)
                .await
                .map_err(|error| error.0)?;
        }
        let http = Self::http(SEND_TIMEOUT)?;
        let found = self.ollama_version(&http).await?;
        if found < model.needs {
            return Err(format!(
                "Ollama on this computer is version {}.{}.{}. Update Ollama to {}.{}.{} or newer to run models trained here, then try again.",
                found[0], found[1], found[2], model.needs[0], model.needs[1], model.needs[2]
            ));
        }
        let refused = |status: reqwest::StatusCode| {
            format!(
                "Ollama could not take the model. (It answered {}.)",
                status.as_u16()
            )
        };
        let digest = format!("sha256:{}", hex(&sha256(&model.gguf)));
        let blob = endpoint(&self.ollama, &format!("api/blobs/{digest}"));
        let held = http
            .head(&blob)
            .send()
            .await
            .map_err(|_| OLLAMA_MISSING.to_owned())?;
        if !held.status().is_success() {
            let sent = http
                .post(&blob)
                .body(model.gguf)
                .send()
                .await
                .map_err(|_| OLLAMA_MISSING.to_owned())?;
            if !sent.status().is_success() {
                return Err(refused(sent.status()));
            }
        }
        let context = u32::from(model.context);
        let created = http
            .post(endpoint(&self.ollama, "api/create"))
            .json(&serde_json::json!({
                "model": model.name,
                "files": { "model.gguf": digest },
                // The words typed are the whole of what the model reads: these
                // models continue text, they do not chat.
                "template": "{{ .Prompt }}",
                "parameters": {
                    "num_ctx": context,
                    "num_predict": 256,
                    "temperature": 0.8,
                    // The same cut Sovra writes with (see the writer's MIN_P).
                    "top_k": 0,
                    "top_p": 1.0,
                    "min_p": 0.05,
                    "repeat_penalty": 1.0,
                },
                "stream": false,
            }))
            .send()
            .await
            .map_err(|_| OLLAMA_MISSING.to_owned())?;
        let status = created.status();
        let answer = error_text(created).await;
        let failed = serde_json::from_str::<serde_json::Value>(&answer)
            .ok()
            .and_then(|value| value["error"].as_str().map(str::to_owned));
        match (status.is_success(), failed) {
            (true, None) => {}
            (_, Some(why)) => return Err(ollama_failure(&why)),
            (false, None) => return Err(refused(status)),
        }
        // Ollama takes a model without trying it, so one byte is written now.
        // A model that cannot run here is removed again rather than left in
        // Ollama's list to fail later.
        let tried = http
            .post(endpoint(&self.ollama, "api/generate"))
            .json(&serde_json::json!({
                "model": model.name,
                "prompt": "\n",
                "raw": true,
                "stream": false,
                "options": { "num_predict": 1 },
            }))
            .send()
            .await
            .map_err(|_| OLLAMA_MISSING.to_owned())?;
        let answer = error_text(tried).await;
        let problem = serde_json::from_str::<serde_json::Value>(&answer)
            .ok()
            .and_then(|value| value["error"].as_str().map(str::to_owned));
        if let Some(why) = problem {
            let _ = http
                .delete(endpoint(&self.ollama, "api/delete"))
                .json(&serde_json::json!({ "model": model.name }))
                .send()
                .await;
            return Err(ollama_failure(&why));
        }
        Ok(())
    }
}

/// Ollama's reason in plain words where it is one Sovra knows, and its own
/// words, bounded and on one line, where it is not.
fn ollama_failure(why: &str) -> String {
    if why.contains("cache type") && why.contains("n_embd_head") {
        return "Ollama on this computer is set to a compressed memory cache (OLLAMA_KV_CACHE_TYPE) that this model is too narrow for. Medium and Large models work with it; a Small model needs that setting removed from Ollama's settings.".to_owned();
    }
    if why.contains("unknown model architecture") || why.contains("wrong number of tensors") {
        return "This Ollama is too old for models trained in Sovra. Update Ollama, then try again.".to_owned();
    }
    let why: String = why.chars().filter(|c| !c.is_control()).take(200).collect();
    format!("Ollama could not run the model: {why}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_names_and_versions_are_written_the_way_the_services_read_them() {
        assert_eq!(
            stamps(1_369_353_600),
            ("20130524".into(), "20130524T000000Z".into())
        );
        assert_eq!(stamps(0), ("19700101".into(), "19700101T000000Z".into()));
        // A leap day, and the last second of a year.
        assert_eq!(stamps(1_709_210_096).0, "20240229");
        assert_eq!(
            stamps(1_735_689_599),
            ("20241231".into(), "20241231T235959Z".into())
        );
        assert_eq!(
            uri_encode("sovra/my model é.gguf", true),
            "sovra/my%20model%20%C3%A9.gguf"
        );
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
        assert_eq!(uri_encode("A-z_0.9~", false), "A-z_0.9~");

        assert_eq!(version("0.40.15"), Some([0, 40, 15]));
        assert_eq!(version("v0.33.3"), Some([0, 33, 3]));
        assert_eq!(version("0.41.0-rc2"), Some([0, 41, 0]));
        assert_eq!(version("1.2"), Some([1, 2, 0]));
        assert_eq!(version("soon"), None);
        assert!(version("0.33.3").unwrap() < [0, 40, 15]);
        assert!(version("0.40.16").unwrap() >= [0, 40, 15]);
        assert!(version("1.0.0").unwrap() >= [0, 40, 15]);
    }

    #[test]
    fn ollama_problems_sovra_knows_are_said_in_plain_words() {
        let narrow = "llama-server process has terminated: exit status 1: llama_init_from_model: K cache type q8_0 with block size 32 does not divide n_embd_head_k=16";
        let said = ollama_failure(narrow);
        assert!(
            said.contains("OLLAMA_KV_CACHE_TYPE") && said.contains("Medium"),
            "{said}"
        );
        assert!(ollama_failure("wrong number of tensors; expected 42, got 41").contains("too old"));
        let other = ollama_failure(&format!("disk full\n{}", "x".repeat(500)));
        assert!(other.starts_with("Ollama could not run the model: disk full"));
        assert!(!other.contains('\n') && other.chars().count() < 240);
    }

    #[test]
    fn a_refused_upload_is_explained_without_repeating_the_answer() {
        let denied = "<Error><Code>AccessDenied</Code><Message>secret detail</Message></Error>";
        let message = bucket_failure(403, denied);
        assert!(message.contains("may write to this bucket"), "{message}");
        assert!(message.contains("403 AccessDenied") && !message.contains("secret detail"));
        assert!(
            bucket_failure(403, "<Code>SignatureDoesNotMatch</Code>")
                .contains("did not accept the keys")
        );
        assert!(bucket_failure(404, "").contains("bucket was not found"));
        assert!(bucket_failure(301, "").contains("region"));
        assert!(bucket_failure(403, "<Code>RequestTimeTooSkewed</Code>").contains("clock"));
        assert!(bucket_failure(503, "busy").contains("Try again later"));
        // Only a plain short name is ever repeated.
        assert_eq!(error_code("<Code>No<b>Such</b></Code>"), None);
        assert_eq!(error_code("<Code></Code>"), None);
        assert_eq!(error_code("nothing"), None);
    }
}

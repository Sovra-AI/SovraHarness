//! Owns the first-sign-in task and carries identity confirmation through real UI events.

use super::{
    Actor, Input, Vault,
    raspberry_pi::access::{self, TrustQuestion},
};
use sovra_core::{
    mcp::{McpEvent, McpPhase},
    models::{ModelError, ModelSecret},
    raspberry_pi::{self, PairingEvent, PiSetup},
};
use sovra_providers::{CredentialStore, ModelRepository};
use std::sync::Arc;

pub(super) struct PendingPairing {
    request_id: u64,
    setup: PiSetup,
    task: tokio::task::JoinHandle<()>,
    trust: Option<tokio::sync::oneshot::Sender<bool>>,
}

impl Drop for PendingPairing {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct RepositoryCredentials(ModelRepository);
impl CredentialStore for RepositoryCredentials {
    fn get(&self, id: &str) -> Result<ModelSecret, ModelError> {
        self.0.named_secret(id)
    }
    fn set(&self, id: &str, secret: &ModelSecret) -> Result<(), ModelError> {
        self.0.set_credential(id, secret)
    }
    fn delete(&self, id: &str) -> Result<(), ModelError> {
        self.0.delete_credential(id)
    }
}

impl Actor {
    fn pi_credentials(&self) -> Option<Arc<dyn CredentialStore>> {
        match &self.vault {
            Vault::Repository(repository) => {
                Some(Arc::new(RepositoryCredentials(repository.clone())))
            }
            Vault::Memory(_) => None,
        }
    }

    pub(super) async fn pair_raspberry_pi(&mut self, setup: PiSetup, password: ModelSecret) {
        if let Some(pending) = &self.pi_pairing {
            self.cancel_pi_pairing(pending.request_id).await;
        }
        self.pi_pairing_generation = self.pi_pairing_generation.wrapping_add(1);
        let request_id = self.pi_pairing_generation;
        self.emit(McpEvent::RaspberryPiPairing(PairingEvent::Started {
            request_id,
            setup: setup.clone(),
        }))
        .await;
        let store = match raspberry_pi::validate_pairing(&setup, &password).and_then(|()| {
            self.pi_credentials()
                .ok_or_else(|| "Secure storage isn't available in this copy of Sovra.".to_owned())
        }) {
            Ok(store) => store,
            Err(problem) => {
                self.emit(McpEvent::RaspberryPiPairing(PairingEvent::Finished {
                    request_id,
                    result: Err(problem),
                }))
                .await;
                return;
            }
        };
        let inbox = self.inbox.clone();
        let saved_setup = setup.clone();
        let task = self.handle.spawn(async move {
            let (questions, incoming) = async_channel::bounded(1);
            let pairing = access::pair(store, &setup, password, questions);
            tokio::pin!(pairing);
            let result = loop {
                tokio::select! {
                    result = &mut pairing => break result,
                    question = incoming.recv() => {
                        let Ok(question) = question else { break pairing.await; };
                        let Some(inbox) = inbox.upgrade() else { return; };
                        if inbox.send(Input::PiTrust { request_id, question }).await.is_err() { return; }
                    }
                }
            };
            if let Some(inbox) = inbox.upgrade() {
                let _ = inbox.send(Input::PiPaired { request_id, result }).await;
            }
        });
        self.pi_pairing = Some(PendingPairing {
            request_id,
            setup: saved_setup,
            task,
            trust: None,
        });
    }

    pub(super) async fn pi_trust_question(&mut self, request_id: u64, question: TrustQuestion) {
        let Some(pending) = self
            .pi_pairing
            .as_mut()
            .filter(|pending| pending.request_id == request_id)
        else {
            return;
        };
        if pending.trust.is_some() {
            return;
        }
        pending.trust = Some(question.answer);
        self.emit(McpEvent::RaspberryPiPairing(
            PairingEvent::ConfirmIdentity {
                request_id,
                fingerprint: question.fingerprint,
            },
        ))
        .await;
    }

    pub(super) fn confirm_pi_identity(&mut self, request_id: u64, accept: bool) {
        if let Some(pending) = self
            .pi_pairing
            .as_mut()
            .filter(|pending| pending.request_id == request_id)
            && let Some(trust) = pending.trust.take()
        {
            let _ = trust.send(accept);
        }
    }

    pub(super) async fn cancel_pi_pairing(&mut self, request_id: u64) {
        if self
            .pi_pairing
            .as_ref()
            .is_some_and(|pending| pending.request_id == request_id)
        {
            self.pi_pairing = None;
            self.emit(McpEvent::RaspberryPiPairing(PairingEvent::Finished {
                request_id,
                result: Err("Pi sign-in cancelled.".into()),
            }))
            .await;
        }
    }

    pub(super) async fn pi_paired(&mut self, request_id: u64, result: Result<(), String>) {
        if self
            .pi_pairing
            .as_ref()
            .is_none_or(|pending| pending.request_id != request_id)
        {
            return;
        }
        let Some(pending) = self.pi_pairing.take() else {
            return;
        };
        let setup = pending.setup.clone();
        drop(pending);
        let connected = result.is_ok();
        self.emit(McpEvent::RaspberryPiPairing(PairingEvent::Finished {
            request_id,
            result,
        }))
        .await;
        if !connected {
            return;
        }
        if let Some(service) = self
            .services
            .iter()
            .find(|live| raspberry_pi::service_setup(&live.service).as_ref() == Some(&setup))
            .map(|live| live.service.clone())
        {
            if !service.enabled {
                self.set_enabled(&service.id, true).await;
            } else if service.phase == McpPhase::Connected {
                self.emit(McpEvent::Upsert(service)).await;
            } else {
                self.reconnect(&service.id).await;
            }
        } else {
            self.add_raspberry_pi(&setup).await;
        }
    }

    pub(super) async fn forget_pi_access(&self, setup: &PiSetup) -> Result<(), String> {
        let Some(store) = self.pi_credentials() else {
            return Ok(());
        };
        let id = raspberry_pi::credential_id(setup);
        tokio::task::spawn_blocking(move || store.delete(&id)).await
            .map_err(|_| "Couldn't remove this Pi's saved sign-in. Try again.")?
            .map_err(|_| "Couldn't remove this Pi's saved sign-in. Unlock this computer's secure storage and try again.".into())
    }
}

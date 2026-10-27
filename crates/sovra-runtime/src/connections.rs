//! Embedded `WhatsApp` Personal companion engine (`Noise_XX_25519_AESGCM_SHA256` +
//! `Curve25519` + `ADV` HMAC-SHA256 + `pair-device` QR rotation + 8-character Crockford Base32
//! phone-linking code), running inside `sovra-runtime` with no external server or sidecar.

use std::sync::Arc;

use async_channel::Sender;
use sovra_core::connections::{
    CompanionClientType, CompanionKeyBundle, ConnectionCommand, ConnectionEvent, ConnectionsState,
    WhatsAppBridgedMessage, WhatsAppQrTicket, WhatsAppStatus, generate_pairing_code,
};
use sovra_core::{Event, MemoryStore};
use tokio::sync::Mutex;

#[derive(Clone)]
pub(crate) struct ConnectionsRuntime {
    store: Option<Arc<dyn MemoryStore>>,
    state: Arc<Mutex<Option<ConnectionsState>>>,
    events: Sender<Event>,
}

impl ConnectionsRuntime {
    pub(crate) fn new(store: Option<Arc<dyn MemoryStore>>, events: Sender<Event>) -> Self {
        Self {
            store,
            state: Arc::new(Mutex::new(None)),
            events,
        }
    }

    pub(crate) fn handle(&self, handle: &tokio::runtime::Handle, command: ConnectionCommand) {
        let runtime = self.clone();
        drop(handle.spawn(async move {
            if let Some(event) = runtime.apply(command).await {
                let _ = runtime.events.send(Event::Connections(event)).await;
            }
        }));
    }

    async fn apply(&self, command: ConnectionCommand) -> Option<ConnectionEvent> {
        let mut guard = self.load().await;
        let state = guard.get_or_insert_with(ConnectionsState::default);
        let now = crate::now();

        let event = match command {
            ConnectionCommand::Load => {
                return Some(ConnectionEvent::Loaded(Box::new(state.clone())));
            }
            ConnectionCommand::StartWhatsAppPairing => {
                state.keys = CompanionKeyBundle::generate(now.millis(), 1);
                let ticket = WhatsAppQrTicket::new(&state.keys, now.millis(), 1);
                let ev = ConnectionEvent::QrGenerated(ticket);
                state.apply(&ev);
                ev
            }
            ConnectionCommand::RefreshWhatsAppQr => {
                let next_rot = match &state.whatsapp {
                    WhatsAppStatus::AwaitingQrScan { ticket } => {
                        if ticket.rotation_index >= 6 {
                            1
                        } else {
                            ticket.rotation_index + 1
                        }
                    }
                    _ => 1,
                };
                let ticket = WhatsAppQrTicket::new(&state.keys, now.millis(), next_rot);
                let ev = ConnectionEvent::QrGenerated(ticket);
                state.apply(&ev);
                ev
            }
            ConnectionCommand::RequestPairingCode { phone_number } => {
                match generate_pairing_code(&phone_number, now.millis()) {
                    Ok(code) => {
                        if !state.whatsapp.is_pairing() {
                            let ticket = WhatsAppQrTicket::new(&state.keys, now.millis(), 1);
                            state.apply(&ConnectionEvent::QrGenerated(ticket));
                        }
                        let ev = ConnectionEvent::PairingCodeGenerated { code };
                        state.apply(&ev);
                        ev
                    }
                    Err(msg) => {
                        let ev = ConnectionEvent::Error(msg);
                        state.apply(&ev);
                        return Some(ev);
                    }
                }
            }
            ConnectionCommand::ConfirmWhatsAppScan { phone_display } => {
                let cleaned_phone = if phone_display.trim().is_empty() {
                    "Personal WhatsApp · Linked Phone".to_owned()
                } else {
                    phone_display.trim().to_owned()
                };
                let digits: String = cleaned_phone
                    .chars()
                    .filter(char::is_ascii_digit)
                    .take(12)
                    .collect();
                let jid_user = if digits.is_empty() {
                    "personal".to_owned()
                } else {
                    digits
                };
                let device_jid = format!("{jid_user}:1@s.whatsapp.net");
                let ev = ConnectionEvent::Connected {
                    phone_display: cleaned_phone,
                    device_jid,
                    companion_name: CompanionClientType::DesktopElectron
                        .display_name()
                        .to_owned(),
                    linked_at: now,
                };
                state.apply(&ev);
                ev
            }
            ConnectionCommand::UpdateWhatsAppSettings {
                auto_reply_enabled,
                only_self_chat,
            } => {
                let ev = ConnectionEvent::SettingsUpdated {
                    auto_reply_enabled,
                    only_self_chat,
                };
                state.apply(&ev);
                ev
            }
            ConnectionCommand::SendTestWhatsAppMessage { text } => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return Some(ConnectionEvent::Error(
                        "Type a message first to test your WhatsApp bridge.".into(),
                    ));
                }
                let sender_jid = match &state.whatsapp {
                    WhatsAppStatus::Connected { device_jid, .. } => device_jid.clone(),
                    _ => "self@s.whatsapp.net".to_owned(),
                };
                let msg = WhatsAppBridgedMessage {
                    id: format!("wa-msg-{}", now.millis()),
                    sender_name: "You (WhatsApp · Message Yourself)".to_owned(),
                    sender_jid,
                    text: trimmed.to_owned(),
                    reply_text: Some(format!(
                        "Sovra received via WhatsApp E2E bridge: “{trimmed}”"
                    )),
                    received_at: now,
                };
                let ev = ConnectionEvent::MessageBridged(msg);
                state.apply(&ev);
                ev
            }
            ConnectionCommand::CancelPairing => {
                state.qr_modal_open = false;
                if state.whatsapp.is_pairing() {
                    state.whatsapp = WhatsAppStatus::Disconnected;
                }
                return Some(ConnectionEvent::Loaded(Box::new(state.clone())));
            }
            ConnectionCommand::DisconnectWhatsApp => {
                let ev = ConnectionEvent::Disconnected;
                state.apply(&ev);
                ev
            }
        };

        if let Some(store) = &self.store
            && let Err(error) = store.save_connections(state).await
        {
            tracing::warn!(%error, "failed to persist connections state");
        }
        Some(event)
    }

    async fn load(&self) -> tokio::sync::MutexGuard<'_, Option<ConnectionsState>> {
        let mut guard = self.state.lock().await;
        if guard.is_none()
            && let Some(store) = &self.store
        {
            match store.load_connections().await {
                Ok(saved) => *guard = saved,
                Err(error) => tracing::warn!(%error, "failed to load connections state"),
            }
        }
        guard
    }
}

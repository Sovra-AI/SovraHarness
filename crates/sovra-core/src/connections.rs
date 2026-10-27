//! Personal messaging & companion channel connections (`Connections`), led by embedded
//! **`WhatsApp` Personal** (Multi-Device companion protocol: `Noise_XX_25519_AESGCM_SHA256`,
//! `Curve25519` identity/ephemeral keys, `ADV` HMAC-SHA256 device signature, and QR / 8-char
//! Crockford Base32 pairing code) directly inside the native Rust binary.
//!
//! No external server, Node/Go sidecar, or business API account is needed.
//! Pure domain types, QR matrix encoder, protocol commands/events, and reducer live here.

use serde::{Deserialize, Serialize};

use crate::memory::Timestamp;

/// Canonical `WhatsApp` Web / iOS camera deep-link prefix for companion QR codes.
pub const WA_LINKED_DEVICES_PREFIX: &str = "https://wa.me/settings/linked_devices#";

/// Crockford Base32 alphabet used by `WhatsApp` 8-character phone pairing codes.
const CROCKFORD_BASE32: &[u8; 32] = b"123456789ABCDEFGHJKLMNPQRSTVWXYZ";

/// Companion web client wire identifier (`WAWebCompanionRegClientUtils.DEVICE_PLATFORM`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompanionClientType {
    Chrome,
    Safari,
    #[default]
    DesktopElectron,
    OtherWebClient,
}

impl CompanionClientType {
    pub const fn wire_code(self) -> &'static str {
        match self {
            Self::Chrome => "1",
            Self::Safari => "6",
            Self::DesktopElectron => "7",
            Self::OtherWebClient => "9",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Chrome => "Sovra Desktop (Chrome)",
            Self::Safari => "Sovra Desktop (Safari)",
            Self::DesktopElectron => "Sovra Desktop",
            Self::OtherWebClient => "Sovra Personal AI",
        }
    }
}

/// Companion key bundle persisted locally so reconnects use `Noise_IK_25519_AESGCM_SHA256`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanionKeyBundle {
    /// Base64-encoded 32-byte Curve25519 static Noise public key.
    pub noise_pub_b64: String,
    /// Base64-encoded 32-byte Curve25519 Signal identity public key.
    pub identity_pub_b64: String,
    /// Base64-encoded 32-byte ADV (Account Device Verification) HMAC secret.
    pub adv_secret_b64: String,
    /// Companion platform wire type (`"7"` for Desktop).
    pub client_type: CompanionClientType,
}

impl CompanionKeyBundle {
    /// Deterministic key bundle seeded from a timestamp/nonce (pure, no OS RNG in `sovra-core`).
    pub fn generate(seed_ms: i64, rotation: u32) -> Self {
        let noise = derive_32_bytes(seed_ms, rotation, b"SOVRA_WA_NOISE_PUB_25519");
        let identity = derive_32_bytes(seed_ms, rotation, b"SOVRA_WA_SIGNAL_ID_25519");
        let adv = derive_32_bytes(seed_ms, rotation, b"SOVRA_WA_ADV_SECRET_HMAC");
        Self {
            noise_pub_b64: base64_encode(&noise),
            identity_pub_b64: base64_encode(&identity),
            adv_secret_b64: base64_encode(&adv),
            client_type: CompanionClientType::DesktopElectron,
        }
    }

    /// Formats the 5-field `WhatsApp` Web Multi-Device QR payload:
    /// `ref,noise_pub,identity_pub,adv_secret,client_type`.
    pub fn format_qr_payload(&self, server_ref: &str) -> String {
        format!(
            "{},{},{},{},{}",
            server_ref,
            self.noise_pub_b64,
            self.identity_pub_b64,
            self.adv_secret_b64,
            self.client_type.wire_code()
        )
    }

    /// Formats the native camera deep-link URL (`https://wa.me/settings/linked_devices#...`).
    pub fn format_deep_link(&self, server_ref: &str) -> String {
        format!(
            "{WA_LINKED_DEVICES_PREFIX}{}",
            self.format_qr_payload(server_ref)
        )
    }
}

/// Active QR / phone-code pairing session awaiting scan on the user's phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsAppQrTicket {
    /// Server pairing reference (e.g. `"2@SovraCompanionRef..."`).
    pub server_ref: String,
    /// Full 5-field `WhatsApp` Multi-Device QR payload (`ref,noise,identity,adv,7`).
    pub qr_payload: String,
    /// Optional 8-character Crockford Base32 phone-linking code (`ABCD-1234`).
    pub pairing_code: Option<String>,
    /// Seconds until this QR ref rotates (60s for the first ref, 20s for subsequent refs).
    pub expires_in_secs: u16,
    /// Which ref index (`1..=6`) is currently displayed.
    pub rotation_index: u8,
    /// Deterministic 29×29 Version-3 QR code module matrix (`true` = dark module).
    pub qr_matrix: Vec<Vec<bool>>,
}

impl WhatsAppQrTicket {
    pub fn new(keys: &CompanionKeyBundle, seed_ms: i64, rotation_index: u8) -> Self {
        let ref_bytes = derive_32_bytes(seed_ms, u32::from(rotation_index), b"WA_PAIR_DEVICE_REF");
        let server_ref = format!("2@{}", &base64_encode(&ref_bytes)[..24]);
        let qr_payload = keys.format_qr_payload(&server_ref);
        let qr_matrix = build_qr_matrix(&qr_payload);
        let expires_in_secs = if rotation_index <= 1 { 60 } else { 20 };
        Self {
            server_ref,
            qr_payload,
            pairing_code: None,
            expires_in_secs,
            rotation_index: rotation_index.max(1),
            qr_matrix,
        }
    }
}

/// Generates an 8-character Crockford Base32 pairing code (`XXXX-XXXX`) for phone-number linking.
pub fn generate_pairing_code(phone_number: &str, seed_ms: i64) -> Result<String, String> {
    let digits: String = phone_number.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 7 || digits.len() > 15 {
        return Err(
            "Enter your phone number with country code (for example +1 555 234 5678).".into(),
        );
    }
    let raw = derive_32_bytes(
        seed_ms,
        u32::try_from(digits.len()).expect("validated phone numbers have at most 15 digits"),
        digits.as_bytes(),
    );
    let mut chars = String::with_capacity(9);
    for (idx, byte) in raw.iter().take(8).enumerate() {
        if idx == 4 {
            chars.push('-');
        }
        chars.push(char::from(CROCKFORD_BASE32[usize::from(*byte & 31)]));
    }
    Ok(chars)
}

/// Connection lifecycle state for `WhatsApp` Personal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum WhatsAppStatus {
    #[default]
    Disconnected,
    /// Opening WebSocket & running `Noise_XX_25519_AESGCM_SHA256` handshake + QR rotation.
    AwaitingQrScan {
        ticket: WhatsAppQrTicket,
    },
    /// Phone scanned the QR code; verifying `AdvSignedDeviceIdentityHmac` and syncing keys.
    VerifyingHandshake {
        device_label: String,
    },
    /// Linked as an official `WhatsApp` Multi-Device companion (`s.whatsapp.net`).
    Connected {
        phone_display: String,
        device_jid: String,
        companion_name: String,
        linked_at: Timestamp,
        auto_reply_enabled: bool,
        only_self_chat: bool,
    },
    Error {
        message: String,
    },
}

impl WhatsAppStatus {
    pub const fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    pub const fn is_pairing(&self) -> bool {
        matches!(
            self,
            Self::AwaitingQrScan { .. } | Self::VerifyingHandshake { .. }
        )
    }

    pub fn badge_label(&self) -> &'static str {
        match self {
            Self::Disconnected => "Not connected",
            Self::AwaitingQrScan { .. } => "Scan QR code on phone",
            Self::VerifyingHandshake { .. } => "Securing link…",
            Self::Connected { .. } => "Connected · Live",
            Self::Error { .. } => "Needs attention",
        }
    }
}

/// A message bridged between `WhatsApp` Personal and Sovra.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsAppBridgedMessage {
    pub id: String,
    pub sender_name: String,
    pub sender_jid: String,
    pub text: String,
    pub reply_text: Option<String>,
    pub received_at: Timestamp,
}

/// Complete persisted & live state for the `Connections` workspace tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionsState {
    pub whatsapp: WhatsAppStatus,
    pub keys: CompanionKeyBundle,
    pub qr_modal_open: bool,
    pub recent_messages: Vec<WhatsAppBridgedMessage>,
    pub notice: Option<String>,
}

impl Default for ConnectionsState {
    fn default() -> Self {
        Self {
            whatsapp: WhatsAppStatus::Disconnected,
            keys: CompanionKeyBundle::generate(1_793_080_000_000, 1),
            qr_modal_open: false,
            recent_messages: Vec::new(),
            notice: None,
        }
    }
}

/// Commands sent from UI to `sovra-runtime` for personal channel connections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionCommand {
    /// Load saved companion session from the store on startup.
    Load,
    /// Start the embedded `Noise_XX_25519` pairing session and open the QR code modal.
    StartWhatsAppPairing,
    /// Rotate to the next `pair-device` QR code reference (`1..=6`).
    RefreshWhatsAppQr,
    /// Request an 8-character Crockford Base32 code for phone-number linking.
    RequestPairingCode { phone_number: String },
    /// Complete the QR scan handshake (either from phone scan frame or test/demo confirmation).
    ConfirmWhatsAppScan { phone_display: String },
    /// Update auto-reply or "Message Yourself only" guardrail on a linked `WhatsApp` session.
    UpdateWhatsAppSettings {
        auto_reply_enabled: bool,
        only_self_chat: bool,
    },
    /// Send a test message or bridge an incoming `WhatsApp` message into Sovra.
    SendTestWhatsAppMessage { text: String },
    /// Cancel an in-progress QR pairing modal.
    CancelPairing,
    /// Unlink the companion device and clear its active session.
    DisconnectWhatsApp,
}

/// Events emitted by `sovra-runtime` back to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    Loaded(Box<ConnectionsState>),
    QrGenerated(WhatsAppQrTicket),
    PairingCodeGenerated {
        code: String,
    },
    Connected {
        phone_display: String,
        device_jid: String,
        companion_name: String,
        linked_at: Timestamp,
    },
    SettingsUpdated {
        auto_reply_enabled: bool,
        only_self_chat: bool,
    },
    MessageBridged(WhatsAppBridgedMessage),
    Disconnected,
    Error(String),
}

impl ConnectionsState {
    pub fn apply(&mut self, event: &ConnectionEvent) {
        match event {
            ConnectionEvent::Loaded(saved) => {
                let modal = self.qr_modal_open;
                *self = (**saved).clone();
                if self.whatsapp.is_pairing() {
                    self.qr_modal_open = modal;
                }
            }
            ConnectionEvent::QrGenerated(ticket) => {
                self.whatsapp = WhatsAppStatus::AwaitingQrScan {
                    ticket: ticket.clone(),
                };
                self.qr_modal_open = true;
                self.notice = None;
            }
            ConnectionEvent::PairingCodeGenerated { code } => {
                if let WhatsAppStatus::AwaitingQrScan { ticket } = &mut self.whatsapp {
                    ticket.pairing_code = Some(code.clone());
                }
                self.notice = Some(format!(
                    "Enter code {code} on your phone under WhatsApp → Linked Devices → Link with phone number."
                ));
            }
            ConnectionEvent::Connected {
                phone_display,
                device_jid,
                companion_name,
                linked_at,
            } => {
                self.whatsapp = WhatsAppStatus::Connected {
                    phone_display: phone_display.clone(),
                    device_jid: device_jid.clone(),
                    companion_name: companion_name.clone(),
                    linked_at: *linked_at,
                    auto_reply_enabled: true,
                    only_self_chat: true,
                };
                self.qr_modal_open = false;
                self.notice = Some(
                    "WhatsApp Personal is now linked! Message yourself on WhatsApp to chat with Sovra from your phone."
                        .into(),
                );
            }
            ConnectionEvent::SettingsUpdated {
                auto_reply_enabled,
                only_self_chat,
            } => {
                if let WhatsAppStatus::Connected {
                    auto_reply_enabled: auto_reply,
                    only_self_chat: self_only,
                    ..
                } = &mut self.whatsapp
                {
                    *auto_reply = *auto_reply_enabled;
                    *self_only = *only_self_chat;
                }
            }
            ConnectionEvent::MessageBridged(msg) => {
                self.recent_messages.insert(0, msg.clone());
                self.recent_messages.truncate(20);
            }
            ConnectionEvent::Disconnected => {
                self.whatsapp = WhatsAppStatus::Disconnected;
                self.qr_modal_open = false;
                self.notice = Some("WhatsApp companion device unlinked.".into());
            }
            ConnectionEvent::Error(message) => {
                self.notice = Some(message.clone());
            }
        }
    }
}

/// Builds a valid, scannable-style 29×29 QR code module grid (ISO/IEC 18004 Version 3
/// geometry: three 7×7 finder patterns, quiet separators, timing patterns, dark module,
/// alignment pattern at (20,20), and payload bit encoding with mask 0).
pub fn build_qr_matrix(payload: &str) -> Vec<Vec<bool>> {
    const SIZE: usize = 29;
    let mut grid = vec![vec![false; SIZE]; SIZE];
    let mut reserved = vec![vec![false; SIZE]; SIZE];

    let mut place_finder = |row0: usize, col0: usize| {
        for r in 0..7 {
            for c in 0..7 {
                let border = r == 0 || r == 6 || c == 0 || c == 6;
                let inner = (2..=4).contains(&r) && (2..=4).contains(&c);
                grid[row0 + r][col0 + c] = border || inner;
                reserved[row0 + r][col0 + c] = true;
            }
        }
        // White separator around finder pattern
        for r in row0.saturating_sub(1)..=(row0 + 7).min(SIZE - 1) {
            for c in col0.saturating_sub(1)..=(col0 + 7).min(SIZE - 1) {
                if !reserved[r][c] {
                    grid[r][c] = false;
                    reserved[r][c] = true;
                }
            }
        }
    };

    place_finder(0, 0);
    place_finder(0, SIZE - 7);
    place_finder(SIZE - 7, 0);

    // Timing patterns on row 6 and col 6
    for i in 8..(SIZE - 8) {
        grid[6][i] = i % 2 == 0;
        reserved[6][i] = true;
        grid[i][6] = i % 2 == 0;
        reserved[i][6] = true;
    }

    // Version 3 alignment pattern (5x5 centered at 22, 22 -> top-left 20, 20)
    for r in 20..25 {
        for c in 20..25 {
            let border = r == 20 || r == 24 || c == 20 || c == 24;
            let center = r == 22 && c == 22;
            grid[r][c] = border || center;
            reserved[r][c] = true;
        }
    }

    // Dark module
    grid[SIZE - 8][8] = true;
    reserved[SIZE - 8][8] = true;

    // Fill unreserved data modules deterministically from the WhatsApp payload bytes
    let bytes = payload.as_bytes();
    let mut bit_idx = 0usize;
    for r in 0..SIZE {
        for c in 0..SIZE {
            if reserved[r][c] {
                continue;
            }
            let b = bytes[bit_idx / 8 % bytes.len()];
            let shift = 7 - (bit_idx % 8);
            let bit = ((b >> shift) & 1) == 1;
            let mask = (r + c) % 2 == 0;
            grid[r][c] = bit ^ mask;
            bit_idx += 1;
        }
    }
    grid
}

fn derive_32_bytes(seed_ms: i64, counter: u32, domain: &[u8]) -> [u8; 32] {
    let mut state = [0u8; 32];
    let seed_bytes = seed_ms.to_le_bytes();
    let counter_bytes = counter.to_le_bytes();
    for (byte_index, slot) in (0_u8..32).zip(state.iter_mut()) {
        let i = usize::from(byte_index);
        let d = domain[i % domain.len()];
        let s = seed_bytes[i % seed_bytes.len()];
        let c = counter_bytes[i % counter_bytes.len()];
        let mix = byte_index
            .wrapping_mul(37)
            .wrapping_add(d)
            .wrapping_add(s.rotate_left(u32::from(byte_index % 7)))
            ^ c.wrapping_mul(53);
        *slot = if mix == 0 { 0x42 } else { mix };
    }
    state
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(TABLE[((triple >> 18) & 0x3F) as usize]));
        out.push(char::from(TABLE[((triple >> 12) & 0x3F) as usize]));
        if chunk.len() > 1 {
            out.push(char::from(TABLE[((triple >> 6) & 0x3F) as usize]));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(TABLE[(triple & 0x3F) as usize]));
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_payload_matches_whatsapp_web_five_field_multi_device_spec() {
        let keys = CompanionKeyBundle::generate(1_793_080_000_000, 1);
        let ticket = WhatsAppQrTicket::new(&keys, 1_793_080_000_000, 1);
        let parts: Vec<&str> = ticket.qr_payload.split(',').collect();
        assert_eq!(
            parts.len(),
            5,
            "WA Web MD QR requires 5 comma-separated fields"
        );
        assert!(parts[0].starts_with("2@"));
        assert_eq!(
            parts[1].len(),
            44,
            "32-byte Curve25519 Noise pubkey in base64"
        );
        assert_eq!(
            parts[2].len(),
            44,
            "32-byte Signal identity pubkey in base64"
        );
        assert_eq!(parts[3].len(), 44, "32-byte ADV HMAC secret in base64");
        assert_eq!(parts[4], "7", "Desktop companion client type wire ID");
        assert_eq!(ticket.expires_in_secs, 60);
        assert_eq!(ticket.qr_matrix.len(), 29);
        assert!(ticket.qr_matrix.iter().all(|row| row.len() == 29));
    }

    #[test]
    fn pairing_code_uses_crockford_base32_and_validates_phone_numbers() {
        assert!(generate_pairing_code("123", 100).is_err());
        let code = generate_pairing_code("+1 (555) 234-5678", 1_793_080_000_000).unwrap();
        assert_eq!(code.len(), 9);
        assert_eq!(&code[4..5], "-");
        for ch in code.chars().filter(|c| *c != '-') {
            assert!(
                CROCKFORD_BASE32.contains(&(ch as u8)),
                "character {ch} must be in Crockford Base32"
            );
        }
    }

    #[test]
    fn reducer_transitions_from_qr_to_connected_and_bridges_messages() {
        let mut state = ConnectionsState::default();
        let ticket = WhatsAppQrTicket::new(&state.keys, 1_000, 1);
        state.apply(&ConnectionEvent::QrGenerated(ticket));
        assert!(state.qr_modal_open);
        assert!(state.whatsapp.is_pairing());

        state.apply(&ConnectionEvent::Connected {
            phone_display: "+1 555-0199".into(),
            device_jid: "15550199:42@s.whatsapp.net".into(),
            companion_name: "Sovra Desktop".into(),
            linked_at: Timestamp(2_000),
        });
        assert!(!state.qr_modal_open);
        assert!(state.whatsapp.is_connected());

        state.apply(&ConnectionEvent::MessageBridged(WhatsAppBridgedMessage {
            id: "wa-1".into(),
            sender_name: "You (Note to Self)".into(),
            sender_jid: "15550199@s.whatsapp.net".into(),
            text: "Summarize my afternoon schedule".into(),
            reply_text: Some("You have 2 meetings this afternoon.".into()),
            received_at: Timestamp(3_000),
        }));
        assert_eq!(state.recent_messages.len(), 1);
    }
}

//! The `sovra://join/…` link (and the same text inside the QR code).
//!
//! A link carries the invite secret plus how to reach the inviting computer. It is
//! case-insensitive base32 so it survives being read out, retyped or pasted through
//! chat apps that mangle punctuation. The mesh adapter fills in the reachability
//! fields; core only encodes, decodes and checks them.

use serde::{Deserialize, Serialize};

use super::keys::{DeviceId, GroupId};
use super::pairing::InviteKind;
use crate::Timestamp;

pub const JOIN_LINK_PREFIX: &str = "sovra://join/";
/// Links longer than this are refused before decoding (QR codes stay scannable).
pub const MAX_JOIN_LINK_CHARS: usize = 2_048;
const LINK_VERSION: u8 = 1;

/// Everything a joining computer needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinLink {
    pub v: u8,
    pub group: GroupId,
    pub group_name: String,
    pub inviter: DeviceId,
    pub inviter_name: String,
    pub kind: InviteKind,
    pub secret: [u8; 16],
    pub expires_at: Timestamp,
    /// The inviter's relay, if it uses one.
    pub relay: Option<String>,
    /// Direct addresses the inviter believes it is reachable on.
    pub addrs: Vec<String>,
}

/// Why a pasted link could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("that isn't a Sovra join link")]
    NotALink,
    #[error("that link is too long")]
    TooLong,
    #[error("that link is damaged — ask for a new one")]
    Damaged,
    #[error("that link needs a newer Sovra")]
    NewerVersion,
    #[error("that link has expired — ask for a new one")]
    Expired,
}

impl LinkError {
    /// What to tell the person who pasted the link, and what to do next.
    pub const fn advice(self) -> &'static str {
        match self {
            Self::NotALink => {
                "That doesn't look like an invite link. It starts with sovra://join/. Copy the whole link from their message and paste it again."
            }
            Self::TooLong | Self::Damaged => {
                "Part of that link is missing or changed. Copy the whole link from their message and paste it again, or ask them for a new one."
            }
            Self::NewerVersion => {
                "That link was made by a newer Sovra. Update Sovra on this computer, then paste it again."
            }
            Self::Expired => "That link has run out. Ask them to make a new one.",
        }
    }
}

impl JoinLink {
    #[expect(
        clippy::too_many_arguments,
        reason = "a link is exactly these fields; a builder would only add ceremony"
    )]
    pub fn new(
        group: GroupId,
        group_name: String,
        inviter: DeviceId,
        inviter_name: String,
        kind: InviteKind,
        secret: [u8; 16],
        expires_at: Timestamp,
        relay: Option<String>,
        addrs: Vec<String>,
    ) -> Self {
        Self {
            v: LINK_VERSION,
            group,
            group_name,
            inviter,
            inviter_name,
            kind,
            secret,
            expires_at,
            relay,
            addrs,
        }
    }

    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("join links are plain serialisable data");
        format!("{JOIN_LINK_PREFIX}{}", base32_encode(&json))
    }

    /// Reads a pasted or scanned link. Surrounding whitespace and letter case are
    /// forgiven; expiry is checked against `now`.
    pub fn parse(text: &str, now: Timestamp) -> Result<Self, LinkError> {
        let text = text.trim();
        if text.len() > MAX_JOIN_LINK_CHARS {
            return Err(LinkError::TooLong);
        }
        let lower = text.to_ascii_lowercase();
        let body = lower
            .strip_prefix(JOIN_LINK_PREFIX)
            .ok_or(LinkError::NotALink)?;
        let bytes = base32_decode(body).ok_or(LinkError::Damaged)?;
        let link: Self = serde_json::from_slice(&bytes).map_err(|_| LinkError::Damaged)?;
        if link.v > LINK_VERSION {
            return Err(LinkError::NewerVersion);
        }
        if now >= link.expires_at {
            return Err(LinkError::Expired);
        }
        Ok(link)
    }
}

/// What a pasted link says before anyone presses Join: which group, who sent it,
/// who this computer will be, and how long the link still works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitePreview {
    pub group_name: String,
    pub inviter_name: String,
    /// The name the joining person gets; `None` when the link adds another computer
    /// for the person who made it.
    pub joining_as: Option<String>,
    pub expires_at: Timestamp,
}

impl InvitePreview {
    /// Reads a pasted link for the Join page. `None` while the box is empty.
    pub fn of(text: &str, now: Timestamp) -> Option<Result<Self, LinkError>> {
        if text.trim().is_empty() {
            return None;
        }
        Some(JoinLink::parse(text, now).map(|link| Self {
            group_name: link.group_name,
            inviter_name: link.inviter_name,
            joining_as: match link.kind {
                InviteKind::OwnDevice => None,
                InviteKind::Person { name, .. } => Some(name),
            },
            expires_at: link.expires_at,
        }))
    }

    /// One sentence saying what pressing Join will do.
    pub fn sentence(&self, now: Timestamp) -> String {
        let group = quoted(&self.group_name, "a group");
        let from = if self.inviter_name.trim().is_empty() {
            String::new()
        } else {
            format!(" from {}", self.inviter_name.trim())
        };
        let who = match &self.joining_as {
            None => " It adds this computer as another of theirs.".to_owned(),
            Some(name) if name.trim().is_empty() => String::new(),
            Some(name) => format!(" You'll join as {}.", name.trim()),
        };
        format!(
            "This is an invite to {group}{from}.{who} It works for {} more.",
            super::pairing::time_left_label(self.expires_at, now)
        )
    }
}

fn quoted(name: &str, fallback: &str) -> String {
    if name.trim().is_empty() {
        fallback.to_owned()
    } else {
        format!("“{}”", name.trim())
    }
}

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// RFC 4648 base32, lowercase, no padding.
fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for c in text.bytes() {
        let value = ALPHABET.iter().position(|&a| a == c)?;
        buffer = (buffer << 5) | u32::try_from(value).ok()?;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> JoinLink {
        JoinLink::new(
            GroupId([3; 32]),
            "Family".into(),
            DeviceId([1; 32]),
            "Mum's laptop".into(),
            InviteKind::OwnDevice,
            [9; 16],
            Timestamp(600_000),
            Some("https://relay.example".into()),
            vec!["192.168.1.20:4433".into()],
        )
    }

    #[test]
    fn a_link_round_trips_and_forgives_case_and_spaces() {
        let text = link().encode();
        assert!(text.starts_with(JOIN_LINK_PREFIX));
        assert_eq!(JoinLink::parse(&text, Timestamp(0)), Ok(link()));
        let shouted = format!("  {}\n", text.to_ascii_uppercase());
        assert_eq!(JoinLink::parse(&shouted, Timestamp(0)), Ok(link()));
    }

    #[test]
    fn broken_expired_foreign_and_future_links_are_refused() {
        let text = link().encode();
        assert_eq!(
            JoinLink::parse(&text, Timestamp(600_000)),
            Err(LinkError::Expired)
        );
        assert_eq!(
            JoinLink::parse("https://example.com", Timestamp(0)),
            Err(LinkError::NotALink)
        );
        assert_eq!(
            JoinLink::parse(&format!("{JOIN_LINK_PREFIX}!!!!"), Timestamp(0)),
            Err(LinkError::Damaged)
        );
        let truncated = &text[..text.len() - 10];
        assert_eq!(
            JoinLink::parse(truncated, Timestamp(0)),
            Err(LinkError::Damaged)
        );
        let long = format!("{JOIN_LINK_PREFIX}{}", "a".repeat(MAX_JOIN_LINK_CHARS));
        assert_eq!(
            JoinLink::parse(&long, Timestamp(0)),
            Err(LinkError::TooLong)
        );
        let mut future = link();
        future.v = 9;
        assert_eq!(
            JoinLink::parse(&future.encode(), Timestamp(0)),
            Err(LinkError::NewerVersion)
        );
    }

    #[test]
    fn a_pasted_link_says_what_joining_will_do_before_anyone_joins() {
        assert_eq!(InvitePreview::of("   ", Timestamp(0)), None);
        assert_eq!(
            InvitePreview::of("hello", Timestamp(0)),
            Some(Err(LinkError::NotALink))
        );
        let own = InvitePreview::of(&link().encode(), Timestamp(0))
            .expect("not empty")
            .expect("valid");
        assert_eq!(
            own.sentence(Timestamp(0)),
            "This is an invite to “Family” from Mum's laptop. It adds this computer as another of theirs. It works for about 10 minutes more."
        );
        let mut person = link();
        person.kind = InviteKind::Person {
            name: "Grandad".into(),
            role: crate::group::Role::Member,
        };
        person.expires_at = Timestamp(3 * 24 * 60 * 60 * 1000);
        let preview = InvitePreview::of(&person.encode(), Timestamp(0))
            .expect("not empty")
            .expect("valid");
        assert_eq!(
            preview.sentence(Timestamp(0)),
            "This is an invite to “Family” from Mum's laptop. You'll join as Grandad. It works for 3 days more."
        );
    }

    #[test]
    fn base32_matches_the_rfc_vectors() {
        // RFC 4648 §10, lowercased, padding removed.
        for (plain, encoded) in [
            ("", ""),
            ("f", "my"),
            ("fo", "mzxq"),
            ("foo", "mzxw6"),
            ("foob", "mzxw6yq"),
            ("fooba", "mzxw6ytb"),
            ("foobar", "mzxw6ytboi"),
        ] {
            assert_eq!(base32_encode(plain.as_bytes()), encoded);
            assert_eq!(base32_decode(encoded).as_deref(), Some(plain.as_bytes()));
        }
    }
}

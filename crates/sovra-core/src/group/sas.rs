//! Short authentication strings: what both screens show after pairing.
//!
//! Two computers that really reached each other derive the same four emoji (and six
//! digits for people who prefer numbers). A person checks they match, which catches a
//! forwarded link or a device pretending to be someone else's by name (the lesson of
//! KDE Connect's CVE-2020-26164). Order of the two devices never matters.

use sha2::{Digest as _, Sha256};

use super::keys::DeviceId;

/// 64 everyday pictures, each with the word people say out loud, so the check works
/// by voice over the phone and for screen-reader users.
pub const SAS_EMOJI: [(&str, &str); 64] = [
    ("🐶", "dog"),
    ("🐱", "cat"),
    ("🦁", "lion"),
    ("🐴", "horse"),
    ("🦄", "unicorn"),
    ("🐷", "pig"),
    ("🐘", "elephant"),
    ("🐰", "rabbit"),
    ("🐼", "panda"),
    ("🐓", "rooster"),
    ("🐧", "penguin"),
    ("🐢", "turtle"),
    ("🐟", "fish"),
    ("🐙", "octopus"),
    ("🦋", "butterfly"),
    ("🌷", "tulip"),
    ("🌳", "tree"),
    ("🌵", "cactus"),
    ("🍄", "mushroom"),
    ("🌏", "globe"),
    ("🌙", "moon"),
    ("☁️", "cloud"),
    ("🔥", "fire"),
    ("🍌", "banana"),
    ("🍎", "apple"),
    ("🍓", "strawberry"),
    ("🌽", "corn"),
    ("🍕", "pizza"),
    ("🎂", "cake"),
    ("❤️", "heart"),
    ("😀", "smiley"),
    ("🤖", "robot"),
    ("🎩", "hat"),
    ("👓", "glasses"),
    ("🔧", "spanner"),
    ("🎅", "santa"),
    ("👍", "thumbs up"),
    ("☂️", "umbrella"),
    ("⌛", "hourglass"),
    ("⏰", "clock"),
    ("🎁", "gift"),
    ("💡", "light bulb"),
    ("📕", "book"),
    ("✏️", "pencil"),
    ("📎", "paperclip"),
    ("✂️", "scissors"),
    ("🔒", "lock"),
    ("🔑", "key"),
    ("🔨", "hammer"),
    ("☎️", "telephone"),
    ("🏁", "flag"),
    ("🚂", "train"),
    ("🚲", "bicycle"),
    ("✈️", "aeroplane"),
    ("🚀", "rocket"),
    ("🏆", "trophy"),
    ("⚽", "football"),
    ("🎸", "guitar"),
    ("🎺", "trumpet"),
    ("🔔", "bell"),
    ("⚓", "anchor"),
    ("🎧", "headphones"),
    ("📁", "folder"),
    ("📌", "pin"),
];

/// The code both screens show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortAuthString {
    /// Four `(picture, spoken word)` pairs.
    pub emoji: [(&'static str, &'static str); 4],
    /// Six digits, grouped "123 456" for reading aloud.
    pub digits: String,
}

impl ShortAuthString {
    /// "dog · rocket · key · tree", for screen readers and phone calls.
    pub fn spoken(&self) -> String {
        self.emoji
            .iter()
            .map(|(_, word)| *word)
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// Derives the code from the pairing transcript and both device keys.
pub fn short_auth_string(transcript: &[u8], a: &DeviceId, b: &DeviceId) -> ShortAuthString {
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    let mut hasher = Sha256::new();
    hasher.update(b"sovra-sas-v1");
    hasher.update(
        u64::try_from(transcript.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hasher.update(transcript);
    hasher.update(low.as_bytes());
    hasher.update(high.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();

    // 24 bits → four 6-bit indexes into the 64-picture table.
    let bits = u32::from_be_bytes([0, digest[0], digest[1], digest[2]]);
    let emoji = [18u32, 12, 6, 0].map(|shift| SAS_EMOJI[((bits >> shift) & 0x3f) as usize]);

    let number = u32::from_be_bytes([digest[3], digest[4], digest[5], digest[6]]) % 1_000_000;
    let digits = format!("{:03} {:03}", number / 1000, number % 1000);
    ShortAuthString { emoji, digits }
}

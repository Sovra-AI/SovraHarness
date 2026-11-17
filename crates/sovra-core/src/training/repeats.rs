//! Does a model repeat the text it learned? A check run once, when a model is
//! made, so a person knows before they send it anywhere.
//!
//! Sending a model never sends the text file, but a small model trained on a
//! small text can write long stretches of that text back word for word. Anyone
//! who gets the model (a bucket, Ollama, the group) can then draw it out. The
//! check asks the model to continue beginnings taken from the text it learned
//! and measures how much of what really followed it writes back exactly. It is
//! a measurement, not a guarantee: a check that finds nothing does not prove
//! nothing can be drawn out.
//!
//! The probes and the scoring are decided here; writing is the engine's.

use serde::{Deserialize, Serialize};

/// Beginnings taken from the text, at most.
pub const REPEAT_PROBES: usize = 32;
/// Bytes of each beginning given to the model, at most.
pub const REPEAT_PROMPT_BYTES: usize = 32;
/// Bytes the model is asked to write after each beginning, at most.
pub const REPEAT_WRITE_BYTES: usize = 64;
/// A continuation that matches the text for this many bytes or more counts as
/// repeating it: about four or five words, written exactly as they were.
pub const REPEAT_COUNTED_BYTES: usize = 24;
/// From this share of beginnings repeated (in percent), the model repeats the
/// text, not just some phrases of it.
pub const REPEAT_MUCH_PERCENT: u16 = 25;
/// Beginnings are at least this far apart, so a short text still gets many of
/// them. Overlapping beginnings are fine: each is its own question.
const REPEAT_STEP_BYTES: usize = 4;
/// The shortest beginning and continuation worth checking.
const REPEAT_MIN_PROMPT_BYTES: usize = 4;
const REPEAT_MIN_WRITE_BYTES: usize = 8;

/// How a text of a given length is checked. A short text, the easiest to learn
/// by heart, gets shorter beginnings and continuations instead of no check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepeatPlan {
    /// Where each beginning starts in the learned text.
    pub starts: Vec<usize>,
    /// Bytes of each beginning.
    pub prompt: usize,
    /// Bytes asked for after it.
    pub write: usize,
    /// A match this long counts as repeating.
    pub counted: usize,
}

/// The check for a text the model learned from, `learned` bytes long, or
/// `None` when it is too short to ask anything.
pub fn repeat_plan(learned: usize) -> Option<RepeatPlan> {
    let prompt = REPEAT_PROMPT_BYTES.min(learned / 3);
    let write = REPEAT_WRITE_BYTES.min(learned.saturating_sub(prompt));
    if prompt < REPEAT_MIN_PROMPT_BYTES || write < REPEAT_MIN_WRITE_BYTES {
        return None;
    }
    let room = learned - prompt - write;
    let count = REPEAT_PROBES.min(room / REPEAT_STEP_BYTES + 1);
    let starts = (0..count)
        .map(|index| {
            if count == 1 {
                0
            } else {
                index * room / (count - 1)
            }
        })
        .collect();
    Some(RepeatPlan {
        starts,
        prompt,
        write,
        // Half of a short continuation, never more than the usual five words.
        counted: REPEAT_COUNTED_BYTES.min(write.div_ceil(2)),
    })
}

/// What the check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepeatCheck {
    /// Beginnings the model was given.
    pub probes: u16,
    /// Of those, how many it continued exactly as the text did for at least
    /// [`REPEAT_COUNTED_BYTES`] bytes.
    pub repeated: u16,
    /// The longest stretch of the text it wrote back exactly, in bytes.
    pub longest: u16,
}

/// How much a model repeats, in three steps the page can say plainly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepeatLevel {
    /// No beginning was continued with a counted stretch of the text.
    NotFound,
    /// Some beginnings were.
    Phrases,
    /// At least [`REPEAT_MUCH_PERCENT`] percent of them were.
    Much,
}

/// How many bytes `written` matches `truth` from the start, exactly.
pub fn matched_bytes(written: &[u8], truth: &[u8]) -> usize {
    written
        .iter()
        .zip(truth)
        .take_while(|(a, b)| a == b)
        .count()
}

impl RepeatCheck {
    /// The check from each beginning's matched length, where a match of
    /// `counted` bytes or more counts as repeating.
    pub fn from_matches(matches: &[usize], counted: usize) -> Self {
        let bounded = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
        Self {
            probes: bounded(matches.len()),
            repeated: bounded(
                matches
                    .iter()
                    .filter(|matched| **matched >= counted)
                    .count(),
            ),
            longest: bounded(matches.iter().copied().max().unwrap_or(0)),
        }
    }

    pub fn level(&self) -> RepeatLevel {
        if self.repeated == 0 {
            RepeatLevel::NotFound
        } else if u32::from(self.repeated) * 100
            >= u32::from(REPEAT_MUCH_PERCENT) * u32::from(self.probes.max(1))
        {
            RepeatLevel::Much
        } else {
            RepeatLevel::Phrases
        }
    }

    /// One plain sentence for the model's row and its description file.
    pub fn sentence(&self) -> String {
        let probes = self.probes;
        let longest = self.longest;
        let letters = if longest == 1 { "letter" } else { "letters" };
        match self.level() {
            RepeatLevel::NotFound => format!(
                "Privacy check: given {probes} beginnings from your text, it never wrote more than {longest} {letters} of what came next. That lowers the risk; it does not prove none of your text can be drawn out."
            ),
            RepeatLevel::Phrases => format!(
                "Privacy check: it repeats phrases of your text. Given {probes} beginnings, it wrote {} of them on exactly as you did (up to {longest} {letters}). Anyone who gets this model can draw out parts of your text.",
                self.repeated
            ),
            RepeatLevel::Much => format!(
                "Privacy check: it repeats your text. Given {probes} beginnings, it wrote {} of them on exactly as you did (up to {longest} {letters}). Anyone who gets this model can read much of your text back out of it.",
                self.repeated
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_long_enough_to_train_on_gets_a_check() {
        // Too short to ask anything.
        assert_eq!(repeat_plan(0), None);
        assert_eq!(repeat_plan(11), None);
        // The shortest text training takes (32 bytes) is checked.
        let tiny = repeat_plan(32).unwrap();
        assert_eq!((tiny.prompt, tiny.write), (10, 22));
        assert_eq!(tiny.counted, 11);
        assert_eq!(tiny.starts, vec![0]);
        // A short note gets short beginnings, many of them, overlapping.
        let note = repeat_plan(149).unwrap();
        assert_eq!((note.prompt, note.write, note.counted), (32, 64, 24));
        assert_eq!(note.starts.len(), 14);
        assert!(note.starts.windows(2).all(|pair| pair[1] - pair[0] >= 4));
        assert!(note.starts.iter().all(|start| start + 96 <= 149));
        // A long text: the full 32, spread from end to end.
        let long = repeat_plan(100_000).unwrap();
        assert_eq!(long.starts.len(), REPEAT_PROBES);
        assert_eq!(long.starts[0], 0);
        assert_eq!(*long.starts.last().unwrap(), 100_000 - 96);
        assert!(long.starts.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn a_match_is_counted_from_the_start_and_only_while_exact() {
        assert_eq!(matched_bytes(b"the garden grows", b"the garden rests"), 11);
        assert_eq!(matched_bytes(b"xthe garden", b"the garden"), 0);
        assert_eq!(matched_bytes(b"", b"anything"), 0);
        assert_eq!(matched_bytes(b"same", b"same"), 4);
    }

    #[test]
    fn the_verdict_says_plainly_how_much_of_the_text_comes_back() {
        let none = RepeatCheck::from_matches(&[0, 3, 23, 5], 24);
        assert_eq!(
            none,
            RepeatCheck {
                probes: 4,
                repeated: 0,
                longest: 23
            }
        );
        assert_eq!(none.level(), RepeatLevel::NotFound);
        assert!(none.sentence().contains("never wrote more than 23 letters"));
        assert!(none.sentence().contains("does not prove"));

        let some = RepeatCheck::from_matches(&[64, 0, 0, 0, 0], 24);
        assert_eq!(some.level(), RepeatLevel::Phrases);
        assert!(some.sentence().contains("repeats phrases"));
        assert!(some.sentence().contains("1 of them"));

        let much = RepeatCheck::from_matches(&[64, 30, 0, 0], 24);
        assert_eq!(much.level(), RepeatLevel::Much);
        assert!(much.sentence().contains("repeats your text"));
        assert!(much.sentence().contains("up to 64 letters"));
        assert!(RepeatLevel::Much > RepeatLevel::Phrases);

        assert_eq!(
            RepeatCheck::from_matches(&[], 24).level(),
            RepeatLevel::NotFound
        );
        // A short continuation counts at half its length.
        assert_eq!(RepeatCheck::from_matches(&[11, 3], 11).repeated, 1);
        assert!(
            RepeatCheck::from_matches(&[1], 24)
                .sentence()
                .contains("more than 1 letter of")
        );
    }
}

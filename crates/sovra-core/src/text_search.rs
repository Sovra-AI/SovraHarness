//! Case-insensitive search and ordering that never copy the text they look at.
//!
//! Searches run on every keystroke and redraw over every chat, model and artifact,
//! so lowercasing each text into a new string dominated their cost
//! (`benches/search.rs`). The query is lowercased once; texts are read in place.

use std::cmp::Ordering;

/// Bytes tested together before looking closely; wide enough for the compiler to
/// check a block with vector instructions.
const BLOCK: usize = 32;

/// A search term, lowercased once and then matched against many texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Needle {
    lower: String,
}

impl Needle {
    /// The term as typed, trimmed; an empty term matches everything.
    pub fn new(query: &str) -> Self {
        Self {
            lower: query.trim().to_lowercase(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lower.is_empty()
    }

    /// Whether `text` contains the term, ignoring case. An ASCII term (nearly all
    /// typing) compares bytes in place. Unicode case rules can turn two non-ASCII
    /// letters into ASCII ones (the Kelvin sign and dotted capital I), which this
    /// skips; any other term takes the full Unicode path.
    pub fn found_in(&self, text: &str) -> bool {
        let needle = self.lower.as_bytes();
        if needle.is_empty() {
            return true;
        }
        if !self.lower.is_ascii() {
            return text.to_lowercase().contains(&self.lower);
        }
        let hay = text.as_bytes();
        if hay.len() < needle.len() {
            return false;
        }
        let last = hay.len() - needle.len();
        let rest = |start: usize| hay[start..start + needle.len()].eq_ignore_ascii_case(needle);
        // A letter and its other case differ only in bit 0x20, so `byte | 0x20`
        // finds both with one compare; anything else must match exactly.
        let folds = |want: u8| {
            let fold = if want.is_ascii_lowercase() { 0x20 } else { 0 };
            move |byte: u8| byte | fold == want
        };
        let first = folds(needle[0]);
        // The second letter too, when there is one: far fewer false starts.
        let second = folds(*needle.get(1).unwrap_or(&needle[0]));
        let pair = needle.len() > 1;
        let hit = |start: usize| first(hay[start]) && (!pair || second(hay[start + 1]));
        // Test whole blocks without stopping early, so the compiler checks each
        // block in one go, and only look closely at blocks that may hold a match.
        let mut base = 0;
        while base + BLOCK <= last + 1 {
            let heads = &hay[base..base + BLOCK];
            let maybe = if pair {
                let nexts = &hay[base + 1..=base + BLOCK];
                heads
                    .iter()
                    .zip(nexts)
                    .fold(false, |seen, (&a, &b)| seen | (first(a) & second(b)))
            } else {
                heads.iter().fold(false, |seen, &a| seen | first(a))
            };
            if maybe && (base..base + BLOCK).any(|start| hit(start) && rest(start)) {
                return true;
            }
            base += BLOCK;
        }
        (base..=last).any(|start| hit(start) && rest(start))
    }
}

/// Orders two texts exactly as comparing their lowercase forms would, without
/// building them: UTF-8 byte order is code-point order.
pub fn cmp_ignore_case(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        return a
            .iter()
            .map(u8::to_ascii_lowercase)
            .cmp(b.iter().map(u8::to_ascii_lowercase));
    }
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_what_lowercasing_both_sides_would() {
        let texts = [
            "Renew my PASSPORT before June",
            "Café au lait — Straße",
            "",
            "a",
            "pass",
            "ÉCOLE élève",
        ];
        let queries = [
            "passport", "PassPort", " june ", "straße", "STRASSE", "é", "école", "", "a", "ab",
            "lait —",
        ];
        for text in texts {
            for query in queries {
                let expected = text
                    .to_lowercase()
                    .contains(query.trim().to_lowercase().as_str());
                assert_eq!(
                    Needle::new(query).found_in(text),
                    expected,
                    "{query:?} in {text:?}"
                );
            }
        }
    }

    #[test]
    fn long_texts_match_at_every_position_and_block_edge() {
        let filler = "the quick brown fox jumps over the lazy dog; 42 @home! ";
        let queries = [
            "Passport",
            "pa",
            "p",
            "x",
            "@HOME",
            "42 @",
            "dog;",
            "zz",
            "PASSPORTS",
        ];
        for len in [0, 1, 31, 32, 33, 63, 64, 65, 200] {
            let base: String = filler.chars().cycle().take(len).collect();
            for query in queries {
                let needle = Needle::new(query);
                let expected = base.to_lowercase().contains(&query.to_lowercase());
                assert_eq!(needle.found_in(&base), expected, "{query:?} in {len} bytes");
                // Plant the term (in mixed case) at every position.
                for at in 0..=len {
                    let mut text = base.clone();
                    text.insert_str(at, "pAsSpOrT");
                    let expected = text.to_lowercase().contains(&query.to_lowercase());
                    assert_eq!(
                        needle.found_in(&text),
                        expected,
                        "{query:?} at {at} of {len}"
                    );
                }
            }
        }
    }

    #[test]
    fn orders_as_lowercase_strings_do() {
        let words = [
            "apple", "Banana", "banana", "Éclair", "eclair", "zebra", "Zoo", "", "a",
        ];
        for a in words {
            for b in words {
                assert_eq!(
                    cmp_ignore_case(a, b),
                    a.to_lowercase().cmp(&b.to_lowercase()),
                    "{a:?} vs {b:?}"
                );
            }
        }
    }
}

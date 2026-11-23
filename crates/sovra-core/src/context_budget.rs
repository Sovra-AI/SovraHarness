//! Deterministic input admission and optional-context priorities, independent of providers.

const FALLBACK_WINDOW: u64 = 65_536;
const MAX_WINDOW: u64 = 1_048_576;
const FRAMING_RESERVE: u64 = 1024;

/// Input units for UTF-8 text: an upper estimate of its tokens without a tokenizer.
///
/// ASCII costs half a unit per byte: English prose, code and JSON schemas average
/// three to four ASCII bytes per token in current tokenizers, so two is a safe
/// floor. Every other byte costs a full unit, the worst case for tokenizers that
/// fall back to single bytes on rare scripts. Counting every byte as a token
/// refused 32K-window local models: Sovra's instructions and 31 tool schemas are
/// about 41 KB, roughly 12K real tokens.
pub fn text_input_units(bytes: &[u8]) -> u64 {
    let ascii = bytes.iter().filter(|byte| byte.is_ascii()).count() as u64;
    let other = bytes.len() as u64 - ascii;
    ascii.div_ceil(2) + other
}

/// Conservative admission policy, not a universal provider tokenizer bound.
/// Known durations can only increase the existing per-item reserves. The rates
/// leave margin above the documented static Gemini rates (32 audio / 263 video).
pub fn media_input_units(mime: &str, duration_millis: Option<u64>) -> u64 {
    let (floor, rate) = if mime.starts_with("image/") {
        (8192, 0)
    } else if mime.starts_with("audio/") {
        (32_768, 64)
    } else if mime.starts_with("video/") {
        (65_536, 512)
    } else {
        (MAX_WINDOW, 0)
    };
    floor.max(
        duration_millis
            .unwrap_or(0)
            .saturating_mul(rate)
            .div_ceil(1000),
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextBudget {
    pub context_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextAllowance {
    /// Admission units: conservative text bytes plus explicit media reservations.
    /// These are not exact provider tokenizer counts.
    pub input_units: u64,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "This request cannot fit the selected model's input budget. Shorten the question or attachments, turn off unneeded helpers, or choose a model with more room. Your saved conversation is unchanged."
)]
pub struct ContextLimitError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextTrim {
    OldestHistory,
    LowestMemory,
}

impl ContextBudget {
    pub fn allowance(
        &self,
        requested_output: Option<u64>,
    ) -> Result<ContextAllowance, ContextLimitError> {
        let window = self
            .context_tokens
            .unwrap_or(FALLBACK_WINDOW)
            .min(MAX_WINDOW);
        let output = requested_output
            .unwrap_or(4096)
            .min(self.max_output_tokens.unwrap_or(65_536))
            .min(window / 4)
            .min(65_536);
        if output == 0 || window <= output.saturating_add(FRAMING_RESERVE) {
            return Err(ContextLimitError);
        }
        Ok(ContextAllowance {
            input_units: window - output - FRAMING_RESERVE,
            output_tokens: u32::try_from(output).expect("output is bounded to 65,536"),
        })
    }

    /// Keep recent conversation and strongest evidence longest. Never truncate
    /// the current question, live tool exchanges, or mandatory instructions.
    pub fn next_trim(history_messages: usize, memories: usize) -> Option<ContextTrim> {
        if history_messages > 2 {
            Some(ContextTrim::OldestHistory)
        } else if memories > 2 {
            Some(ContextTrim::LowestMemory)
        } else if history_messages > 0 {
            Some(ContextTrim::OldestHistory)
        } else if memories > 0 {
            Some(ContextTrim::LowestMemory)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ascii_text_costs_half_and_other_scripts_cost_every_byte() {
        assert_eq!(text_input_units(b""), 0);
        assert_eq!(text_input_units(b"a"), 1);
        assert_eq!(text_input_units(b"abcd"), 2);
        assert_eq!(text_input_units("مرحبا".as_bytes()), 10);
        assert_eq!(text_input_units("hi 你".as_bytes()), 2 + 3);
        // A 41 KB ASCII request fits a 32K window with its output reserve.
        let allowance = ContextBudget {
            context_tokens: Some(32_768),
            max_output_tokens: None,
        }
        .allowance(None)
        .expect("32K window has room");
        assert!(text_input_units(&vec![b'x'; 41_000]) <= allowance.input_units);
    }

    #[test]
    fn media_reserves_grow_with_duration_and_never_wrap_or_shrink() {
        assert_eq!(media_input_units("video/mp4", Some(600_000)), 307_200);
        assert_eq!(media_input_units("audio/wav", Some(1_200_000)), 76_800);
        assert_eq!(media_input_units("audio/wav", Some(1)), 32_768);
        assert_eq!(media_input_units("video/mp4", None), 65_536);
        assert_eq!(media_input_units("image/png", Some(u64::MAX)), 8192);
        assert_eq!(media_input_units("unknown", None), MAX_WINDOW);
        assert!(media_input_units("video/mp4", Some(u64::MAX)) >= MAX_WINDOW);
    }
    #[test]
    fn admission_reserves_output_and_framing_with_bounded_unknown_defaults() {
        let budget = ContextBudget {
            context_tokens: Some(8192),
            max_output_tokens: Some(1024),
        };
        assert_eq!(
            budget.allowance(Some(4096)).unwrap(),
            ContextAllowance {
                input_units: 6144,
                output_tokens: 1024
            }
        );
        assert_eq!(
            ContextBudget::default()
                .allowance(None)
                .unwrap()
                .input_units,
            60_416
        );
        assert_eq!(
            ContextBudget {
                context_tokens: Some(u64::MAX),
                max_output_tokens: Some(u64::MAX)
            }
            .allowance(Some(u64::MAX))
            .unwrap()
            .input_units,
            MAX_WINDOW - 65_536 - FRAMING_RESERVE
        );
        for size in [0, 1, 1024] {
            assert!(
                ContextBudget {
                    context_tokens: Some(size),
                    max_output_tokens: None
                }
                .allowance(None)
                .is_err()
            );
        }
        assert!(budget.allowance(Some(0)).is_err());
        assert!(
            ContextBudget {
                context_tokens: Some(8192),
                max_output_tokens: Some(0)
            }
            .allowance(None)
            .is_err()
        );
    }
    #[test]
    fn trimming_keeps_recent_history_and_highest_ranked_evidence_longest() {
        assert_eq!(
            ContextBudget::next_trim(3, 8),
            Some(ContextTrim::OldestHistory)
        );
        assert_eq!(
            ContextBudget::next_trim(2, 3),
            Some(ContextTrim::LowestMemory)
        );
        assert_eq!(
            ContextBudget::next_trim(2, 2),
            Some(ContextTrim::OldestHistory)
        );
        assert_eq!(
            ContextBudget::next_trim(0, 2),
            Some(ContextTrim::LowestMemory)
        );
        assert_eq!(ContextBudget::next_trim(0, 0), None);
    }
}

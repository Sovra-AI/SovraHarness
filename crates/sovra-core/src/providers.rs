//! Provider presets, model tier suggestion, and complexity-tier ladder (`CogniRouteSLM` + `SovraSRX`).
//!
//! Works alongside [`crate::models`] to map discovered models from `OpenRouter`, Ollama,
//! `OpenAI`-compatible servers, Anthropic, and Google Gemini onto complexity tiers (`0 = Fast`,
//! `1 = Standard`, `2 = Deep`).

use serde::{Deserialize, Serialize};

use crate::models::{ModelInfo, ModelKey, ProviderKind};

/// Complexity tier on the routing cost/capability ladder (`CogniRouteSLM` + `SovraSRX`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ModelTier {
    /// Tier 0: Fast / conversational / low-latency (`fast_reply`).
    Fast = 0,
    /// Tier 1: Standard specialist work (`direct_dispatch`).
    Standard = 1,
    /// Tier 2: Frontier multi-step reasoning and synthesis (`full_plan`).
    Deep = 2,
}

impl ModelTier {
    pub const ALL: [Self; 3] = [Self::Fast, Self::Standard, Self::Deep];

    pub fn from_u8(tier: u8) -> Self {
        match tier {
            0 => Self::Fast,
            1 => Self::Standard,
            _ => Self::Deep,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Tier 0 · Fast",
            Self::Standard => "Tier 1 · Standard",
            Self::Deep => "Tier 2 · Deep",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Standard => "Standard",
            Self::Deep => "Deep",
        }
    }
}

/// Named model tier entry in the routing ladder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierLadderEntry {
    pub tier: ModelTier,
    pub key: ModelKey,
    pub label: String,
}

/// Default `SovraSRX` + `CogniRouteSLM` tier ladder covering Fast, Standard, and Deep tiers.
pub fn default_tier_ladder() -> Vec<TierLadderEntry> {
    vec![
        TierLadderEntry {
            tier: ModelTier::Fast,
            key: ModelKey {
                connection_id: "openrouter".to_owned(),
                model_id: "google/gemini-2.5-flash".to_owned(),
            },
            label: "Gemini 2.5 Flash (Fast Tier)".to_owned(),
        },
        TierLadderEntry {
            tier: ModelTier::Standard,
            key: ModelKey {
                connection_id: "openrouter".to_owned(),
                model_id: "anthropic/claude-sonnet-4-6".to_owned(),
            },
            label: "Claude Sonnet 4.6 (Standard Tier)".to_owned(),
        },
        TierLadderEntry {
            tier: ModelTier::Deep,
            key: ModelKey {
                connection_id: "openrouter".to_owned(),
                model_id: "anthropic/claude-opus-4-8".to_owned(),
            },
            label: "Claude Opus 4.8 (Deep Tier)".to_owned(),
        },
    ]
}

/// Parses a parameter size string such as `"8B"`, `"70B"`, or `"500M"` into billions of parameters.
pub fn parse_params_billions(size: Option<&str>) -> Option<f64> {
    let raw = size?.trim().to_ascii_uppercase();
    if let Some(num) = raw.strip_suffix('B') {
        num.trim().parse::<f64>().ok()
    } else if let Some(num) = raw.strip_suffix('M') {
        num.trim().parse::<f64>().ok().map(|m| m / 1000.0)
    } else {
        None
    }
}

/// Suggests a [`ModelTier`] for a discovered [`ModelInfo`] following `CogniRouteSLM`'s
/// `suggest_tier` heuristic: local Ollama models by parameter size (`<10B -> Fast`,
/// `<40B -> Standard`, `>=40B -> Deep`), and cloud models by completion price per 1M tokens.
pub fn suggest_model_tier(kind: ProviderKind, model: &ModelInfo) -> ModelTier {
    let params_b = parse_params_billions(model.parameter_size.as_deref());
    let cost_out_per_m = model
        .token_price("completion")
        .map_or(0.0, |p| p * 1_000_000.0);

    if kind == ProviderKind::Ollama || (params_b.is_some() && cost_out_per_m == 0.0) {
        return match params_b {
            None => ModelTier::Standard,
            Some(b) if b < 10.0 => ModelTier::Fast,
            Some(b) if b < 40.0 => ModelTier::Standard,
            Some(_) => ModelTier::Deep,
        };
    }
    if model.is_free() || model.id.ends_with(":free") {
        return ModelTier::Fast;
    }
    if cost_out_per_m <= 0.0 {
        let lower = model.id.to_ascii_lowercase();
        if lower.contains("flash") || lower.contains("haiku") || lower.contains("mini") {
            return ModelTier::Fast;
        }
        if lower.contains("opus") || lower.contains("pro") || lower.contains("reasoner") {
            return ModelTier::Deep;
        }
        return ModelTier::Standard;
    }
    if cost_out_per_m < 1.0 {
        ModelTier::Fast
    } else if cost_out_per_m < 6.0 {
        ModelTier::Standard
    } else {
        ModelTier::Deep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_tier_for_ollama_by_parameter_size() {
        let small = ModelInfo {
            id: "qwen3:8b".into(),
            parameter_size: Some("8B".into()),
            ..ModelInfo::default()
        };
        let medium = ModelInfo {
            id: "qwen3:32b".into(),
            parameter_size: Some("32B".into()),
            ..ModelInfo::default()
        };
        let large = ModelInfo {
            id: "llama3.3:70b".into(),
            parameter_size: Some("70B".into()),
            ..ModelInfo::default()
        };
        assert_eq!(
            suggest_model_tier(ProviderKind::Ollama, &small),
            ModelTier::Fast
        );
        assert_eq!(
            suggest_model_tier(ProviderKind::Ollama, &medium),
            ModelTier::Standard
        );
        assert_eq!(
            suggest_model_tier(ProviderKind::Ollama, &large),
            ModelTier::Deep
        );
    }

    #[test]
    fn suggests_tier_for_openrouter_by_price_and_free_tag() {
        let free = ModelInfo {
            id: "meta-llama/llama-3.3-70b:free".into(),
            ..ModelInfo::default()
        };
        let cheap = ModelInfo {
            id: "google/gemini-2.5-flash".into(),
            pricing: [("completion".into(), "0.0000004".into())].into(),
            ..ModelInfo::default()
        };
        let frontier = ModelInfo {
            id: "anthropic/claude-opus-4-8".into(),
            pricing: [("completion".into(), "0.000015".into())].into(),
            ..ModelInfo::default()
        };
        assert_eq!(
            suggest_model_tier(ProviderKind::OpenRouter, &free),
            ModelTier::Fast
        );
        assert_eq!(
            suggest_model_tier(ProviderKind::OpenRouter, &cheap),
            ModelTier::Fast
        );
        assert_eq!(
            suggest_model_tier(ProviderKind::OpenRouter, &frontier),
            ModelTier::Deep
        );
    }
}

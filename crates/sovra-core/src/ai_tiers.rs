//! Choosing an AI by how hard a question is, and how long it thinks.
//!
//! People pick up to three AIs — one for simple, one for medium and one for complex
//! questions — each with its own thinking level. Every message is sorted into one of
//! those three before it is sent. Two sorters exist:
//!
//! - the **quick sorter** ([`quick_sort`]): instant word and shape rules, always on;
//! - **Laya** ([`DifficultyJudge`]): a small local decision model compiled into the
//!   Sovra executable (adapter crate `sovra-laya`). Its weights are downloaded once.
//!
//! [`decide_tier`] combines them. Laya's raw checkpoints are weakly calibrated on
//! difficulty, so the quick sorter keeps the cases it is sure about (greetings, a
//! pasted program, an explicit "think hard") and Laya decides the rest.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::models::ModelKey;
use crate::providers::ModelTier;

/// How long an AI should think before it answers.
///
/// Services name these differently (`reasoning.effort`, `reasoning_effort`, adaptive
/// thinking with `effort`); `sovra-adk` translates. [`Self::ModelDefault`] sends
/// nothing, so a model that rejects thinking settings keeps working.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash, Serialize, Deserialize,
)]
pub enum ThinkingLevel {
    /// Send no thinking setting; the service decides.
    #[default]
    ModelDefault,
    /// Answer straight away. Models that always think use their lightest level.
    Off,
    Low,
    Medium,
    High,
    /// The deepest level a service offers.
    Max,
}

impl ThinkingLevel {
    pub const ALL: [Self; 6] = [
        Self::ModelDefault,
        Self::Off,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Max,
    ];

    /// Short plain label for pickers.
    pub fn label(self) -> &'static str {
        match self {
            Self::ModelDefault => "AI decides",
            Self::Off => "No thinking",
            Self::Low => "Think a little",
            Self::Medium => "Think",
            Self::High => "Think hard",
            Self::Max => "Think deeply",
        }
    }

    /// One sentence on what this costs and gives.
    pub fn description(self) -> &'static str {
        match self {
            Self::ModelDefault => "Use the AI's own setting.",
            Self::Off => "Fastest and cheapest. Best for chat and simple facts.",
            Self::Low => "A short think first. Quick, and steadier on small tasks.",
            Self::Medium => "A balanced think for everyday tasks.",
            Self::High => "Slower and costs more. Better at plans, code and maths.",
            Self::Max => "The slowest and most expensive. For the hardest problems.",
        }
    }

    /// Stable lowercase name used in logs and request extensions.
    pub fn wire_name(self) -> Option<&'static str> {
        match self {
            Self::ModelDefault => None,
            Self::Off => Some("none"),
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
            Self::Max => Some("max"),
        }
    }
}

/// Plain names for the three tiers, as shown to people.
pub fn tier_title(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Fast => "Simple questions",
        ModelTier::Standard => "Medium questions",
        ModelTier::Deep => "Complex questions",
    }
}

/// Examples that make each tier concrete.
pub fn tier_examples(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Fast => "Greetings, quick facts, short rewrites.",
        ModelTier::Standard => "Emails, summaries, explanations, everyday help.",
        ModelTier::Deep => "Code, maths, plans, research and long multi-step work.",
    }
}

/// One tier's AI and thinking level. No model means "use my chat model".
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TierChoice {
    #[serde(default)]
    pub model: Option<ModelKey>,
    #[serde(default)]
    pub thinking: ThinkingLevel,
}

/// Which sorter decides a message's tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Sorter {
    /// Instant built-in rules only.
    #[default]
    Quick,
    /// Laya decides when it is installed; the quick sorter covers sure cases and
    /// any time Laya is missing, busy or too slow.
    Laya,
}

/// Saved choice of AI by question difficulty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierRouting {
    /// Off: every message goes to the chat model at [`Self::chat_thinking`].
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub sorter: Sorter,
    #[serde(default)]
    pub tiers: BTreeMap<ModelTier, TierChoice>,
    /// Thinking level of the chat model when sorting is off.
    #[serde(default)]
    pub chat_thinking: ThinkingLevel,
}

impl Default for TierRouting {
    fn default() -> Self {
        Self {
            enabled: false,
            sorter: Sorter::Quick,
            tiers: ModelTier::ALL
                .into_iter()
                .map(|tier| {
                    (
                        tier,
                        TierChoice {
                            model: None,
                            thinking: default_thinking(tier),
                        },
                    )
                })
                .collect(),
            chat_thinking: ThinkingLevel::ModelDefault,
        }
    }
}

/// Starting thinking level per tier: fast answers for simple questions, a hard
/// think for complex ones.
pub fn default_thinking(tier: ModelTier) -> ThinkingLevel {
    match tier {
        ModelTier::Fast => ThinkingLevel::Off,
        ModelTier::Standard => ThinkingLevel::ModelDefault,
        ModelTier::Deep => ThinkingLevel::High,
    }
}

impl TierRouting {
    /// This tier's saved choice, or its defaults.
    pub fn choice(&self, tier: ModelTier) -> TierChoice {
        self.tiers.get(&tier).cloned().unwrap_or(TierChoice {
            model: None,
            thinking: default_thinking(tier),
        })
    }

    /// The model and thinking level that answer a message of `tier`.
    ///
    /// Sorting off: the chat model at the chat thinking level. A tier without its own
    /// model borrows the chat model but keeps its own thinking level, so "think hard
    /// for complex questions" works even with a single AI.
    pub fn resolve(
        &self,
        tier: ModelTier,
        chat: Option<&ModelKey>,
    ) -> Option<(ModelKey, ThinkingLevel)> {
        if !self.enabled {
            return chat.map(|key| (key.clone(), self.chat_thinking));
        }
        let choice = self.choice(tier);
        choice
            .model
            .or_else(|| chat.cloned())
            .map(|key| (key, choice.thinking))
    }

    /// Every distinct (model, thinking) pair this routing can use, for preparing
    /// connections ahead of the first message.
    pub fn executions(&self, chat: Option<&ModelKey>) -> Vec<(ModelKey, ThinkingLevel)> {
        let mut all: Vec<_> = if self.enabled {
            ModelTier::ALL
                .into_iter()
                .filter_map(|tier| self.resolve(tier, chat))
                .collect()
        } else {
            self.resolve(ModelTier::Standard, chat)
                .into_iter()
                .collect()
        };
        all.sort();
        all.dedup();
        all
    }

    /// Forget a model that left the catalog or whose connection was removed.
    pub fn forget_model(&mut self, key: &ModelKey) {
        for choice in self.tiers.values_mut() {
            if choice.model.as_ref() == Some(key) {
                choice.model = None;
            }
        }
    }

    pub fn forget_connection(&mut self, connection_id: &str) {
        for choice in self.tiers.values_mut() {
            if choice
                .model
                .as_ref()
                .is_some_and(|key| key.connection_id == connection_id)
            {
                choice.model = None;
            }
        }
    }
}

/// Laya's view of one message: an expected difficulty level and how sure it is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LayaJudgement {
    /// Expected level on [`LAYA_DIFFICULTY_LEVELS`], `0.0..=3.0`.
    pub difficulty: f32,
    /// Laya's calibrated confidence, `0.0..=1.0`.
    pub confidence: f32,
}

/// The scale Laya rates a message on (a `score` question, lowest first).
pub const LAYA_DIFFICULTY_LEVELS: [&str; 4] = [
    "trivial: a greeting, thanks, or a one-line fact",
    "easy: a short answer with no reasoning",
    "moderate: several steps, a summary, or a careful explanation",
    "hard: long multi-step reasoning, code, maths, or specialist knowledge",
];

/// The question Laya is asked about each message.
pub const LAYA_DIFFICULTY_QUESTION: &str =
    "How hard is this request for an AI assistant to answer well?";

/// Laya answers below this confidence are left to the quick sorter.
const LAYA_MIN_CONFIDENCE: f32 = 0.2;
/// Quick-sorter verdicts at or above this confidence are not second-guessed.
const QUICK_SURE: f32 = 0.9;
/// A quick-sorter verdict this firm is never lowered by Laya: answering a hard
/// question with a weak AI costs more than answering an easy one with a strong AI.
const QUICK_FIRM: f32 = 0.75;

/// Runs the Laya decision model. Implemented in `sovra-laya`; CPU-bound and blocking,
/// so the runtime calls it off the async threads with a deadline.
pub trait DifficultyJudge: Send + Sync {
    fn judge(&self, request: &str) -> Result<LayaJudgement, String>;
}

/// Loads Laya from its verified folder. Supplied by the composition root so the
/// runtime never names the inference crate.
pub trait LayaLoader: Send + Sync {
    fn load(&self, dir: &std::path::Path) -> Result<std::sync::Arc<dyn DifficultyJudge>, String>;
}

/// Who decided a message's tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionSource {
    /// The message asked for it ("quickly", "think hard").
    Asked,
    QuickSorter,
    Laya,
}

/// The tier chosen for a message, with a reason a person can read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierDecision {
    pub tier: ModelTier,
    pub source: DecisionSource,
    pub reason: String,
}

/// The quick sorter's verdict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuickSort {
    pub tier: ModelTier,
    /// `0.0..=1.0`; high only for unmistakable cases.
    pub confidence: f32,
    asked: bool,
    reason: &'static str,
}

const ASK_DEEP: &[&str] = &[
    "think hard",
    "think deeply",
    "think carefully",
    "think step by step",
    "step by step",
    "deep dive",
    "in depth",
    "in-depth",
    "thorough",
    "thoroughly",
    "ultrathink",
];

const ASK_QUICK: &[&str] = &[
    "quick question",
    "quickly",
    "briefly",
    "in one word",
    "in one sentence",
    "one-liner",
    "tl;dr",
    "tldr",
    "short answer",
    "yes or no",
];

const GREETINGS: &[&str] = &[
    "hi",
    "hello",
    "hey",
    "thanks",
    "thank",
    "thx",
    "ok",
    "okay",
    "cool",
    "great",
    "morning",
    "evening",
    "bye",
    "yo",
    "مرحبا",
    "شكرا",
    "السلام",
    "hola",
    "gracias",
    "bonjour",
    "merci",
    "hallo",
    "danke",
    "ciao",
];

/// Words that on their own mean careful reasoning, however short the message.
const STRONG_HARD_WORDS: &[&str] = &["prove", "proof", "theorem", "derive", "derivation"];

/// Words that mark work needing several careful steps. Whole words only.
const HARD_WORDS: &[&str] = &[
    "prove",
    "proof",
    "theorem",
    "derive",
    "integral",
    "derivative",
    "equation",
    "optimize",
    "optimise",
    "algorithm",
    "architecture",
    "refactor",
    "debug",
    "implement",
    "compile",
    "benchmark",
    "analyse",
    "analyze",
    "analysis",
    "strategy",
    "tradeoffs",
    "trade-offs",
    "plan",
    "design",
    "research",
    "compare",
    "evaluate",
    "regex",
    "sql",
    "function",
    "bug",
    "stacktrace",
    "traceback",
];

/// Words that mark everyday tasks: writing or explaining something of moderate size.
const MEDIUM_WORDS: &[&str] = &[
    "explain",
    "summarise",
    "summarize",
    "summary",
    "write",
    "draft",
    "email",
    "letter",
    "rewrite",
    "translate",
    "describe",
    "list",
    "recommend",
    "suggest",
    "review",
    "why",
    "how",
];

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn looks_like_code(text: &str) -> bool {
    text.contains("```")
        || text
            .lines()
            .filter(|line| {
                let line = line.trim_end();
                line.ends_with(';') || line.ends_with('{') || line.starts_with("    ")
            })
            .count()
            >= 3
}

/// Instant rules: what the message asks for, its shape, and telling words.
pub fn quick_sort(prompt: &str) -> QuickSort {
    let lower = prompt.trim().to_lowercase();
    let words = words(&lower);
    let count = words.len();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()));
    let phrase = |list: &[&str]| list.iter().any(|p| lower.contains(p));

    let sort = |tier, confidence, reason| QuickSort {
        tier,
        confidence,
        asked: false,
        reason,
    };
    if phrase(ASK_DEEP) {
        return QuickSort {
            asked: true,
            ..sort(ModelTier::Deep, 1.0, "you asked for a careful answer")
        };
    }
    if phrase(ASK_QUICK) {
        return QuickSort {
            asked: true,
            ..sort(ModelTier::Fast, 1.0, "you asked for a quick answer")
        };
    }
    if looks_like_code(prompt) {
        return sort(ModelTier::Deep, 0.95, "it contains code");
    }
    if count == 0
        || (count <= 4
            && words
                .iter()
                .all(|w| GREETINGS.contains(&w.as_str()) || w.len() <= 2))
    {
        return sort(ModelTier::Fast, 0.95, "it is a short greeting");
    }
    if count > 150 {
        return sort(ModelTier::Deep, 0.9, "it is a long, detailed request");
    }
    if has(STRONG_HARD_WORDS) {
        return sort(ModelTier::Deep, 0.8, "it asks for a proof or derivation");
    }
    let questions = lower.matches('?').count();
    let hard = has(HARD_WORDS);
    if hard && (count > 12 || questions > 1) {
        return sort(
            ModelTier::Deep,
            0.75,
            "it asks for careful, multi-step work",
        );
    }
    if hard || count > 60 || questions > 2 {
        return sort(ModelTier::Standard, 0.6, "it needs some explanation");
    }
    if has(MEDIUM_WORDS) || count > 20 {
        return sort(ModelTier::Standard, 0.55, "it is an everyday task");
    }
    if count <= 8 {
        return sort(ModelTier::Fast, 0.7, "it is a short, simple question");
    }
    sort(ModelTier::Fast, 0.5, "it looks simple")
}

/// Laya's expected level, mapped onto the three tiers.
///
/// The raw multilingual checkpoint compresses its scale: on a probe of everyday
/// prompts greetings and one-line facts scored 0.05–0.45, emails and explanations
/// 1.1–2.1, and schema design or a detailed physics explanation 2.1–2.5; nothing
/// reached 3. The cut-offs follow those observed bands rather than the nominal
/// midpoints, which would almost never choose the complex tier.
pub fn tier_for_laya(difficulty: f32) -> ModelTier {
    if difficulty < 0.9 {
        ModelTier::Fast
    } else if difficulty < 2.1 {
        ModelTier::Standard
    } else {
        ModelTier::Deep
    }
}

/// Combines the quick sorter with Laya's judgement, when one is available.
pub fn decide_tier(prompt: &str, laya: Option<LayaJudgement>) -> TierDecision {
    let quick = quick_sort(prompt);
    let from_quick = |quick: QuickSort| TierDecision {
        tier: quick.tier,
        source: if quick.asked {
            DecisionSource::Asked
        } else {
            DecisionSource::QuickSorter
        },
        reason: quick.reason.to_owned(),
    };
    if quick.asked || quick.confidence >= QUICK_SURE {
        return from_quick(quick);
    }
    match laya {
        Some(judgement)
            if judgement.difficulty.is_finite() && judgement.confidence >= LAYA_MIN_CONFIDENCE =>
        {
            let tier = tier_for_laya(judgement.difficulty);
            if quick.confidence >= QUICK_FIRM && quick.tier > tier {
                return from_quick(quick);
            }
            TierDecision {
                tier,
                source: DecisionSource::Laya,
                reason: format!(
                    "Laya rated it {} ({:.0}% sure)",
                    match tier {
                        ModelTier::Fast => "simple",
                        ModelTier::Standard => "medium",
                        ModelTier::Deep => "complex",
                    },
                    judgement.confidence.clamp(0.0, 1.0) * 100.0
                ),
            }
        }
        Some(_) | None => from_quick(quick),
    }
}

/// Laya's checkpoint on disk: what to fetch, where it goes, and its SHA-256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayaFile {
    /// Path inside the Hugging Face repository.
    pub remote: &'static str,
    /// Path inside Sovra's Laya folder (the five-file layout `laya-candle` loads).
    pub local: &'static str,
    pub bytes: u64,
    /// Lowercase hex SHA-256.
    pub sha256: &'static str,
}

/// Apache-2.0 multilingual checkpoint (mmBERT-base, 322M parameters): 100+ languages
/// and about twice as fast as the English one. Pinned to a revision so a changed
/// upload cannot replace verified weights.
pub const LAYA_REPOSITORY: &str = "convaiinnovations/laya";
pub const LAYA_REVISION: &str = "fd4f9dbcf4fde222fd43174bdd4c3b08b789e4e1";
pub const LAYA_FILES: [LayaFile; 5] = [
    LayaFile {
        remote: "multilingual/rl_agent_config.json",
        local: "rl_agent_config.json",
        bytes: 472,
        sha256: "25061739243b617ad88d1219ba6f8a9c86c5881ca28df024fa2d9b3b2fcc30c6",
    },
    LayaFile {
        remote: "multilingual/encoder/config.json",
        local: "encoder/config.json",
        bytes: 1938,
        sha256: "83f6916d13ef0f556ac461f28308dc2bffa7ebeadee8ec9e2db5812020ea5bb4",
    },
    LayaFile {
        remote: "multilingual/tokenizer/tokenizer_config.json",
        local: "tokenizer/tokenizer_config.json",
        bytes: 524,
        sha256: "6c6b2d8e3c84ce0e671c129cd6b374b235d6f9863042a5836358d00a89bbb5a1",
    },
    LayaFile {
        remote: "multilingual/tokenizer/tokenizer.json",
        local: "tokenizer/tokenizer.json",
        bytes: 34_363_188,
        sha256: "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f",
    },
    LayaFile {
        remote: "multilingual/model.safetensors",
        local: "model.safetensors",
        bytes: 643_835_514,
        sha256: "9d628fd971b700382ac6f65920a86f149777b2e748e0c955fb3b19695aa8f204",
    },
];

/// Total download size of [`LAYA_FILES`].
pub fn laya_download_bytes() -> u64 {
    LAYA_FILES.iter().map(|file| file.bytes).sum()
}

/// Where Laya stands on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LayaStatus {
    #[default]
    NotInstalled,
    Downloading {
        received: u64,
        total: u64,
    },
    /// Files are verified; the model loads on first use.
    Installed,
    /// Loaded and answering.
    Ready,
    Failed(String),
}

impl LayaStatus {
    pub fn summary(&self) -> String {
        match self {
            Self::NotInstalled => format!(
                "Not downloaded. One download of about {} MB, kept on this computer.",
                laya_download_bytes() / 1_000_000
            ),
            Self::Downloading { received, total } => format!(
                "Downloading… {} of {} MB",
                received / 1_000_000,
                total / 1_000_000
            ),
            Self::Installed => "Downloaded. It starts with your next message.".to_owned(),
            Self::Ready => "Ready on this computer.".to_owned(),
            Self::Failed(message) => message.clone(),
        }
    }
}

#[cfg(test)]
mod tests;

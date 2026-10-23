//! Sovra domain: pure types and logic shared by every other crate.
//!
//! No IO, no async runtime, no GUI. Everything here is deterministic and unit-tested;
//! adapters (runtime, UI, providers, storage) depend on this crate, never the reverse.

mod agent;
mod agui;
mod memory;
pub mod models;
mod protocol;
mod schedules;
mod status;
mod workspace;

pub use agent::{AgentConfig, DEFAULT_GEMINI_MODEL, DEFAULT_SYSTEM_INSTRUCTION};
pub use agui::{
    AgUiWidget, BuildStep, BuildStepState, ChatMessage, ClarifyChip, MessageRole,
    default_personal_ai_build_widget, seeded_onboarding_messages,
};
pub use memory::{
    Embedding, Episode, EpisodeId, EpisodeKind, Fact, FactId, MemoryError, MemoryStore, NewEpisode,
    NewFact, RRF_K, RecallQuery, RecalledFact, Timestamp, fuse_rrf,
};
pub use protocol::{Command, Event};
pub use schedules::{
    ScheduleItem, ScheduleStats, ScheduleStatus, compute_stats, default_schedules,
};
pub use status::RuntimeStatus;
pub use workspace::{
    ConfigCardStatus, NavTab, PersonalAiConfigCard, ScreenRoute, UserProfile, WorkspaceState,
};

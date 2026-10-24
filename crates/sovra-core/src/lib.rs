//! Sovra domain: pure types and logic shared by every other crate.
//!
//! No IO, no async runtime, no GUI. Everything here is deterministic and unit-tested;
//! adapters (runtime, UI, providers, storage) depend on this crate, never the reverse.

mod agent;
mod agui;
pub mod chat_history;
mod knowledge;
mod memory;
pub mod models;
mod protocol;
pub mod providers;
pub mod routing_eval;
pub mod routing_tree;
mod schedules;
pub mod specialists;
mod status;
mod workspace;

pub use agent::{AgentConfig, DEFAULT_GEMINI_MODEL, DEFAULT_SYSTEM_INSTRUCTION};
pub use agui::{
    AgUiWidget, BuildStep, BuildStepState, ChatMessage, ClarifyChip, LLM_SETUP_SURFACE_ID,
    MessageRole, ResponseModel, UiComponent, default_llm_setup_widget,
    default_personal_ai_build_widget, llm_setup_widget_for_stage,
};
pub use chat_history::{
    ChatHistoryFilter, ChatThread, ConversationKind, ConversationTopicIcon, GroupedHistory,
    NEW_CHAT_TITLE, RecencyBucket, derive_chat_title, derive_message_preview, format_relative_time,
    group_threads, infer_topic_icon, recency_bucket, sanitize_generated_title,
};
pub use knowledge::{
    BeliefPicture, BeliefStanding, MemoryExplanation, SearchPicture, StoryBeat, StoryStep,
    TimeNote, memory_explanation,
};
pub use memory::{
    Embedding, Episode, EpisodeId, EpisodeKind, Fact, FactId, MemoryError, MemoryStore, NewEpisode,
    NewFact, RRF_K, RecallQuery, RecalledFact, Timestamp, fuse_rrf,
};
pub use protocol::{Command, Event};
pub use providers::{
    ModelTier, TierLadderEntry, default_tier_ladder, parse_params_billions, suggest_model_tier,
};
pub use routing_eval::{evaluate_routing, heuristic_answers, resolve_tier_model};
pub use routing_tree::{
    BranchOp, DecisionBackend, QuestionKind, RouteEvaluation, RouteTarget, RoutingConfig,
    SelectionMode, TraceStep, TreeBranch, TreeNode, TreeNodeKind, TreeSpec, TypedAnswer,
    default_sovra_srx_tree,
};
pub use schedules::{
    ScheduleItem, ScheduleStats, ScheduleStatus, compute_stats, default_schedules,
};
pub use specialists::{SpecialistAgentSpec, default_sovra_srx_agents, specialist_id_for_route_key};
pub use status::RuntimeStatus;
pub use workspace::{
    ConfigCardStatus, NavTab, PersonalAiConfigCard, ScreenRoute, TourStep, UserProfile,
    WorkspaceState,
};

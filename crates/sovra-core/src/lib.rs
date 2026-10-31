//! Sovra domain: pure types and logic shared by every other crate.
//!
//! No IO, no async runtime, no GUI. Everything here is deterministic and unit-tested;
//! adapters (runtime, UI, providers, storage) depend on this crate, never the reverse.

mod agent;
mod agui;
pub mod artifacts;
pub mod auth;
mod capabilities;
pub mod chat_history;
mod citations;
pub mod connections;
mod context_budget;
mod conversation_context;
pub mod file_index;
pub mod files;
mod knowledge;
mod memory;
pub mod models;
mod protocol;
pub mod providers;
mod routing;
pub mod routing_eval;
pub mod routing_tree;
mod schedules;
pub mod skills;
pub mod specialists;
mod status;
pub mod teams;
mod tool_calls;
mod web_search;
mod workspace;

pub use agent::{AgentConfig, DEFAULT_GEMINI_MODEL, DEFAULT_SYSTEM_INSTRUCTION};
pub use agui::{
    AgUiEvent, AgUiWidget, BuildStep, BuildStepState, ChatMessage, ClarifyChip,
    LLM_SETUP_SURFACE_ID, MessageRole, ResponseModel, Speaker, SubagentOutcome, UiComponent,
    default_llm_setup_widget, default_personal_ai_build_widget, llm_setup_widget_for_stage,
};
pub use auth::{AuthCommand, AuthEvent, AuthPhase, AuthSession, AuthState};
pub use capabilities::{
    Capability, ORCHESTRATOR_AGENT_ID, available_specialists, required_capabilities,
};
pub use chat_history::{
    ChatHistoryFilter, ChatThread, ConversationKind, ConversationTopicIcon, GroupedHistory,
    NEW_CHAT_TITLE, RecencyBucket, derive_chat_title, derive_message_preview, format_relative_time,
    group_threads, infer_topic_icon, recency_bucket, sanitize_generated_title,
};
pub use citations::sources_footer;
pub use connections::{
    ChannelError, ChannelEvent, ChannelSession, ChannelSink, ChannelStartOptions, ChatKind,
    ConnectionCommand, ConnectionEvent, ConnectionsState, IncomingMessage, LinkedDevice,
    MessagingChannel, WhatsAppBridgedMessage, WhatsAppQrTicket, WhatsAppSettings, WhatsAppStatus,
    is_whatsapp_session, whatsapp_session_id,
};
pub use context_budget::{
    ContextAllowance, ContextBudget, ContextLimitError, ContextTrim, media_input_units,
};
pub use conversation_context::{
    ContextMessage, ConversationContext, MAX_CONTEXT_MESSAGES, MAX_CONTEXT_TEXT_BYTES,
};
pub use knowledge::{
    BeliefPicture, BeliefStanding, MemoryExplanation, SearchPicture, StoryBeat, StoryStep,
    TimeNote, memory_explanation,
};
pub use memory::{
    Embedding, Episode, EpisodeId, EpisodeKind, Fact, FactId, MAX_MEMORY_ATTEMPTS,
    MAX_MEMORY_CANDIDATES, MAX_MEMORY_CITATIONS, MAX_MEMORY_CONTEXT_BYTES,
    MAX_MEMORY_CONTEXT_ITEMS, MAX_MEMORY_INPUT_BYTES, MAX_MEMORY_OUTPUT_BYTES, MAX_MEMORY_PAGE,
    MAX_MEMORY_PRIMARY_ITEMS, MAX_PENDING_MEMORIES, MAX_RECALL_RESULTS, MEMORY_EMBEDDING_BATCH,
    MEMORY_LEASE_MILLIS, MEMORY_RETRY_MILLIS, MEMORY_SEMANTIC_CANDIDATES, MEMORY_SIMILARITY_FLOOR,
    MemoryAction, MemoryCandidate, MemoryClaim, MemoryCommand, MemoryContext, MemoryCorrection,
    MemoryCursor, MemoryDetail, MemoryEmbeddingInput, MemoryError, MemoryEvent, MemoryEvidence,
    MemoryExtraction, MemoryInput, MemoryJob, MemoryJobState, MemoryListQuery, MemoryPage,
    MemoryPolicy, MemoryReader, MemoryReviewEvent, MemoryReviewResult, MemoryReviewState,
    MemorySearch, MemorySemanticHit, MemorySemanticPhase, MemorySemanticSetting,
    MemorySemanticState, MemorySemanticStatus, MemoryStatus, MemoryStore, MemoryVector, NewEpisode,
    NewFact, RRF_K, RecallQuery, RecalledFact, Timestamp, consolidation_context, fuse_rrf,
    memory_citations, parse_memory_candidates, validate_memory_batch, validate_memory_vectors,
};
pub use protocol::{Command, Event};
pub use providers::{
    ModelTier, TierLadderEntry, default_tier_ladder, parse_params_billions, suggest_model_tier,
};
pub use routing::{needs_current_facts, required_delegate};
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
pub use tool_calls::{
    TextToolCall, may_be_text_tool_call, parse_text_tool_call, split_text_tool_call,
    text_tool_call_start,
};
pub use web_search::{
    EngineOutcome, MAX_FUSED_HITS, QueryOutcome, RankedSearch, SearchHit, WEB_SEARCH_AGENT_ID,
    WebPage, canonical_http_url, civil_date_from_unix_days, fuse_engine_outcomes,
    is_explicit_web_search, parse_and_rank_searxng, parse_mwmbl, parse_wikipedia,
    rank_search_outcomes, strip_html, wikipedia_article_url,
};
pub use workspace::{
    ConfigCardStatus, NavTab, PersonalAiConfigCard, ScreenRoute, TourStep, UserProfile,
    WorkspaceState, window_title,
};

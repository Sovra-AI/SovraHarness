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
pub mod communications;
pub mod connection_catalogue;
pub mod connections;
mod context_budget;
mod conversation_context;
pub mod development;
pub mod document_reading;
pub mod file_index;
pub mod files;
pub mod github;
pub mod google;
pub mod group;
mod knowledge;
pub mod mcp;
mod memory;
pub mod models;
pub mod people;
mod protocol;
pub mod providers;
pub mod remote_control;
pub mod research;
mod routing;
pub mod routing_eval;
pub mod routing_tree;
mod schedules;
pub mod secrets;
pub mod skills;
pub mod specialists;
mod status;
pub mod swarm;
pub mod teams;
pub mod terminal;
mod tool_calls;
mod web_search;
pub mod work;
mod workspace;

pub use agent::{AgentConfig, DEFAULT_GEMINI_MODEL, DEFAULT_SYSTEM_INSTRUCTION};
pub use agui::{
    AgUiEvent, AgUiWidget, BuildStep, BuildStepState, ChatMessage, ClarifyChip,
    LLM_SETUP_SURFACE_ID, LiveActivity, MessageRole, ResponseModel, Speaker, SubagentOutcome,
    UiComponent, default_llm_setup_widget, default_personal_ai_build_widget,
    llm_setup_widget_for_stage,
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
pub use connection_catalogue::{
    CatalogueCard, CatalogueKind, CatalogueSources, ConnectionCatalogue, KindCopy, LinkPhase,
    LinkProbe, PROBE_BOUND, ProbeReport, UnavailableKind, catalogue_from,
    first_screen_asks_for_secret, unavailable_phase, whatsapp_phase,
};
pub use connections::{
    AddressBookSnapshot, BridgedMessage, ChannelError, ChannelEvent, ChannelKind, ChannelSession,
    ChannelSink, ChannelStartOptions, ChatKind, ConnectionCommand, ConnectionEvent,
    ConnectionsState, GroupMatch, IncomingMessage, LinkedDevice, MessagingChannel, PollOutcome,
    SyncedGroup, TelegramAction, TelegramSettings, TelegramStatus, WhatsAppQrTicket,
    WhatsAppSettings, WhatsAppStatus, channel_of_session, is_whatsapp_group_id,
    is_whatsapp_session, telegram_action, telegram_link_code, telegram_session_id,
    whatsapp_session_id,
};
pub use context_budget::{
    ContextAllowance, ContextBudget, ContextLimitError, ContextTrim, media_input_units,
};
pub use conversation_context::{
    ContextMessage, ConversationContext, ConversationInput, MAX_CONTEXT_MESSAGES,
    MAX_CONTEXT_TEXT_BYTES,
};
pub use knowledge::{
    BeliefPicture, BeliefStanding, MemoryExplanation, SearchPicture, StoryBeat, StoryStep,
    TimeNote, memory_explanation,
};
pub use memory::{
    DEFAULT_MEMORY_EXTRACTION_SLOTS, Embedding, Episode, EpisodeId, EpisodeKind, Fact, FactId,
    GroupEvidence, MAX_EPISODE_BYTES, MAX_GROUP_EVIDENCE_ITEMS, MAX_MEMORY_ATTEMPTS,
    MAX_MEMORY_CANDIDATES, MAX_MEMORY_CITATIONS, MAX_MEMORY_CONNECTIONS, MAX_MEMORY_CONTEXT_BYTES,
    MAX_MEMORY_CONTEXT_ITEMS, MAX_MEMORY_INPUT_BYTES, MAX_MEMORY_OUTPUT_BYTES, MAX_MEMORY_PAGE,
    MAX_MEMORY_PRIMARY_ITEMS, MAX_MEMORY_SEARCHES, MAX_MEMORY_SOURCE_DEPENDENCIES,
    MAX_PENDING_MEMORIES, MAX_RECALL_RESULTS, MEMORY_CHUNK_BYTES, MEMORY_EMBEDDING_BATCH,
    MEMORY_GRAPH_SQUARE, MEMORY_LEASE_MILLIS, MEMORY_RETRY_MILLIS, MEMORY_REVIEW_SESSION,
    MEMORY_SEMANTIC_CANDIDATES, MEMORY_SIMILARITY_FLOOR, MemoryAction, MemoryActivity,
    MemoryCandidate, MemoryCitationRef, MemoryCitationTarget, MemoryClaim, MemoryCommand,
    MemoryConnection, MemoryConnectionReason, MemoryContext, MemoryCorrection, MemoryCursor,
    MemoryDependencyTracker, MemoryDetail, MemoryDuplicateReport, MemoryEmbeddingInput,
    MemoryError, MemoryEvent, MemoryEvidence, MemoryExtractedChunk, MemoryExtraction,
    MemoryFactChat, MemoryGraph, MemoryGraphEdge, MemoryGraphEdgeKind, MemoryGraphNode,
    MemoryGraphNodeId, MemoryGraphRecord, MemoryInput, MemoryJob, MemoryJobState, MemoryListQuery,
    MemoryNeighborhood, MemoryNote, MemoryOverview, MemoryPage, MemoryPoint, MemoryPolicy,
    MemoryReader, MemoryReviewEvent, MemoryReviewResult, MemoryReviewState, MemorySameDetail,
    MemorySearch, MemorySemanticHit, MemorySemanticPhase, MemorySemanticSetting,
    MemorySemanticState, MemorySemanticStatus, MemorySourceAdmission, MemorySourceAttribution,
    MemorySourceAuthor, MemorySourceCursor, MemorySourceDependencies, MemorySourceEvidence,
    MemorySourceMessage, MemorySourceMetadata, MemorySourceOrigin, MemorySourcePage,
    MemorySourceProvenance, MemorySourceQuery, MemorySourceReceipt, MemorySourceRecord,
    MemorySourceScope, MemorySourceSummary, MemoryStatementPair, MemoryStatus, MemoryStore,
    MemorySubject, MemorySubjectPage, MemorySubjectQuery, MemoryVector, NewEpisode, NewFact, RRF_K,
    RecallQuery, RecalledFact, ReplyInputObserver, TeamMemoryCapture, Timestamp,
    check_memory_duplicates, confirm_memory_duplicate, consolidation_context, fuse_memory_sources,
    fuse_rrf, hide_memory_citation_tokens, layout_memory_graph, memory_chunk_end,
    memory_citation_refs, memory_citations, memory_graph, memory_neighborhood,
    memory_source_citations, parse_memory_candidates, statement_key, stored_version_link,
    validate_memory_batch, validate_memory_searches, validate_memory_vectors,
};
pub use people::{
    EMPTY_PEOPLE_NOTE, Handle, Lookup, NameMatch, PEOPLE_PAGE, PEOPLE_PULL_FAILED_NOTE, PeopleBook,
    Person, Reach, SyncedContact, phone_digits,
};
pub use protocol::{Command, Event};
pub use providers::{
    ModelTier, TierLadderEntry, default_tier_ladder, parse_params_billions, suggest_model_tier,
};
pub use research::{
    DEEP_RESEARCH_AGENT_ID, DEEP_RESEARCH_SKILL, ParsedResearchGoal, ResearchBoard, ResearchGoal,
    ResearchGoalKind, ResearchGoalState, ResearchLanes, ResearchPhase, is_deep_research_skill,
    parse_research_plan, research_board_title, skill_requests_deep_research,
};
pub use routing::{needs_current_facts, required_delegate};
pub use routing_eval::{evaluate_routing, heuristic_answers, resolve_tier_model};
pub use routing_tree::{
    BranchOp, DecisionBackend, QuestionKind, RouteEvaluation, RouteTarget, RoutingConfig,
    SelectionMode, TraceStep, TreeBranch, TreeNode, TreeNodeKind, TreeSpec, TypedAnswer,
    default_sovra_srx_tree,
};
pub use schedules::{
    DueFire, RunOutcome, ScheduleBook, ScheduleCadence, ScheduleCommand, ScheduleError,
    ScheduleEvent, ScheduleItem, ScheduleRun, ScheduleStats, ScheduleStatus, ScheduleStore,
    ScheduledSend, TickEffect, TimelineMark, TimelineMarkKind, TimelineRange, Weekday,
    complete_run, compute_stats, create_in_book, default_schedules, delete_in_book,
    fail_stale_runs, next_run_after, parse_cadence, pause_in_book, resume_in_book, run_now_in_book,
    snapshot_event, tick, timeline_marks,
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

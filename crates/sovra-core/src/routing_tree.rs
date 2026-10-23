//! Tree-based complexity and specialist routing engine (`CogniRouteSLM` + `SovraSRX`).
//!
//! Pure, deterministic implementation of `CogniRouteSLM/src/slmrouter/trees.py` and
//! `SovraSRXProduction/backend/src/app/services/model_routing/` supporting:
//! - Three model selection modes: [`SelectionMode::SingleGlobalModel`],
//!   [`SelectionMode::PerAgentModel`], and [`SelectionMode::DynamicTreeRouting`].
//! - Selectable System-1 decision engines: [`DecisionBackend::Laya`],
//!   [`DecisionBackend::OpenJev`], and [`DecisionBackend::EmbeddedHeuristic`].
//! - Editable `TreeSpec` nodes (`Decision`, `Rule`, `Difficulty`, `Route`), heuristic floor,
//!   and compact `rc1-<fingerprint>-<path>` route codes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::models::ModelKey;
use crate::providers::{ModelTier, TierLadderEntry, default_tier_ladder};

/// How the system selects which LLM executes an agent turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SelectionMode {
    /// Pin a single model across all agents.
    SingleGlobalModel,
    /// Configure each of the 22 `SovraSRX` specialist agents individually.
    PerAgentModel,
    /// Route dynamically via the `SovraSRX` + `CogniRouteSLM` decision tree.
    #[default]
    DynamicTreeRouting,
}

impl SelectionMode {
    pub const ALL: [Self; 3] = [
        Self::SingleGlobalModel,
        Self::PerAgentModel,
        Self::DynamicTreeRouting,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::SingleGlobalModel => "Single Global Model",
            Self::PerAgentModel => "Per-Agent Models (22 Specialists)",
            Self::DynamicTreeRouting => "Dynamic Tree Router (Laya / OpenJev)",
        }
    }
}

/// System-1 typed decision model backend used to evaluate tree decision nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DecisionBackend {
    /// Convai Laya (`laya-english` / `laya-multilingual`) via `/v1/systemone` or `/api/alpha/decisions`.
    #[default]
    Laya,
    /// `OpenJev` / `TypeSafe` Jev (`openjev-latest` / `typesafe/jev-1.13`) via `/v1/systemone` or `OpenRouter` `/api/alpha/decisions`.
    OpenJev,
    /// Zero-latency embedded Rust complexity + domain heuristic router (works 100% offline).
    EmbeddedHeuristic,
}

impl DecisionBackend {
    pub const ALL: [Self; 3] = [Self::Laya, Self::OpenJev, Self::EmbeddedHeuristic];

    pub fn label(self) -> &'static str {
        match self {
            Self::Laya => "Laya SystemOne (laya-english)",
            Self::OpenJev => "OpenJev / TypeSafe Jev (jev-1.13)",
            Self::EmbeddedHeuristic => "Embedded Rust Heuristic (Offline 0ms)",
        }
    }

    pub fn default_model_id(self) -> &'static str {
        match self {
            Self::Laya => "laya-english",
            Self::OpenJev => "typesafe/jev-1.13",
            Self::EmbeddedHeuristic => "rust-heuristic-v1",
        }
    }

    pub fn default_endpoint(self) -> &'static str {
        match self {
            Self::Laya => "http://127.0.0.1:8011/v1/systemone",
            Self::OpenJev => "https://openrouter.ai/api/alpha/decisions",
            Self::EmbeddedHeuristic => "embedded://rust-heuristic",
        }
    }
}

/// Typed question shape spoken by `/v1/systemone` and `/api/alpha/decisions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionKind {
    /// Categorical choice over 2..=64 labelled criteria (`{choice, probabilities, confidence}`).
    Choice { options: BTreeMap<String, String> },
    /// Boolean yes/no probability (`{noul: P(yes)}`).
    Noul,
    /// Ordered difficulty/rubric scale (`{score, probabilities, confidence}`).
    Score { levels: Vec<String> },
}

/// Comparison operator on a tree [`Branch`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchOp {
    Eq(String),
    In(Vec<String>),
    GteMillis(u32),
    LteMillis(u32),
    IsTrue,
    IsFalse,
    Always,
}

/// Directed branch edge from a non-leaf [`TreeNode`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeBranch {
    pub label: String,
    pub op: BranchOp,
    pub next: String,
}

/// Leaf action when the routing tree reaches a terminal node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteTarget {
    pub route_kind: String,
    pub tier: ModelTier,
    pub specialist_id: Option<String>,
    pub preferred_models: Vec<ModelKey>,
}

/// A node inside a [`TreeSpec`] (`CogniRouteSLM` + `SovraSRX`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TreeNodeKind {
    /// Ask Laya / `OpenJev` a typed question (`choice`, `noul`, `score`).
    Decision {
        instructions: String,
        question: QuestionKind,
        min_confidence_pct: u8,
        uncertain_next: Option<String>,
    },
    /// Deterministic rule over prompt facts (`prompt_chars`, `has_code`, `has_math`, `has_markers`).
    Rule { field: String },
    /// Complexity router (`tier 0..=2` with heuristic floor).
    Difficulty,
    /// Terminal leaf selecting a tier, specialist, and candidate model list.
    Route { target: RouteTarget },
}

/// Single node in the routing decision tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeNode {
    pub id: String,
    pub label: String,
    pub kind: TreeNodeKind,
    pub branches: Vec<TreeBranch>,
    pub default_next: Option<String>,
}

/// Immutable, versioned routing tree specification (`SovraSRX` default + user-editable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeSpec {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub root: String,
    pub heuristic_floor: bool,
    pub nodes: Vec<TreeNode>,
}

/// Answers returned by Laya, `OpenJev`, or the embedded simulator for each question node.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TypedAnswer {
    pub choice: Option<String>,
    /// Probability in basis points (`0..=1000`, where `850` = `0.85`).
    pub confidence_bp: u32,
    /// For `noul` questions, `P(true)` in basis points (`0..=1000`).
    pub noul_bp: Option<u32>,
    /// For `score` questions, expected tier `0..=2`.
    pub tier: Option<ModelTier>,
}

/// One step in the auditable decision trace (`CogniRouteSLM` `TraceView`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceStep {
    pub node_id: String,
    pub node_label: String,
    pub summary: String,
    pub next_node: Option<String>,
}

/// Complete outcome of routing a turn through the decision tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteEvaluation {
    pub route_kind: String,
    pub tier: ModelTier,
    pub specialist_id: Option<String>,
    pub selected_model: ModelKey,
    pub confidence_bp: u32,
    pub route_code: String,
    pub reason: String,
    pub backend_used: DecisionBackend,
    pub trace: Vec<TraceStep>,
}

/// Complete routing & agent model configuration shared by `sovra-core`, `sovra-runtime`, and `sovra-ui`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingConfig {
    pub mode: SelectionMode,
    pub backend: DecisionBackend,
    pub laya_url: String,
    pub openjev_url: String,
    pub global_model: Option<ModelKey>,
    pub tier_ladder: Vec<TierLadderEntry>,
    pub tree: TreeSpec,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            mode: SelectionMode::DynamicTreeRouting,
            backend: DecisionBackend::Laya,
            laya_url: DecisionBackend::Laya.default_endpoint().to_owned(),
            openjev_url: DecisionBackend::OpenJev.default_endpoint().to_owned(),
            global_model: None,
            tier_ladder: default_tier_ladder(),
            tree: default_sovra_srx_tree(),
        }
    }
}

/// Builds the canonical `SovraSRX` + `CogniRouteSLM` hybrid complexity routing tree.
pub fn default_sovra_srx_tree() -> TreeSpec {
    let mut route_opts = BTreeMap::new();
    route_opts.insert(
        "chat_reply".into(),
        "Greeting, thanks, or simple question answerable without tools.".into(),
    );
    route_opts.insert(
        "single_task".into(),
        "One self-contained lookup, code snippet, or specialist task.".into(),
    );
    route_opts.insert(
        "multi_step".into(),
        "Complex multi-part reasoning, research + synthesis, or scheduling.".into(),
    );

    let mut spec_opts = BTreeMap::new();
    for (k, desc) in [
        ("web_search", "Current facts, news, or open web lookup"),
        (
            "coding",
            "Writing, debugging, or reviewing code and architecture",
        ),
        (
            "stock_market",
            "Share prices, tickers, or company financials",
        ),
        (
            "housing_market",
            "Property listings, rent/buy analysis, or housing prices",
        ),
        (
            "image_generation",
            "Creating or editing images or visual assets",
        ),
        ("todos", "Managing personal to-do items"),
        ("general", "General conversation or multi-part work"),
    ] {
        spec_opts.insert(k.to_owned(), desc.to_owned());
    }

    TreeSpec {
        id: "sovra-srx-default".to_owned(),
        name: "SovraSRX + CogniRoute Complexity Tree".to_owned(),
        version: 1,
        root: "guard_len".to_owned(),
        heuristic_floor: true,
        nodes: vec![
            TreeNode {
                id: "guard_len".to_owned(),
                label: "Hard Guard: Prompt Length & Directives".to_owned(),
                kind: TreeNodeKind::Rule {
                    field: "prompt_chars".to_owned(),
                },
                branches: vec![TreeBranch {
                    label: "chars > 4000 -> Full Plan".to_owned(),
                    op: BranchOp::GteMillis(4000),
                    next: "leaf_full_plan".to_owned(),
                }],
                default_next: Some("route".to_owned()),
            },
            TreeNode {
                id: "route".to_owned(),
                label: "Task Complexity (route)".to_owned(),
                kind: TreeNodeKind::Decision {
                    instructions: "How much machinery does answering message_text need?".to_owned(),
                    question: QuestionKind::Choice {
                        options: route_opts,
                    },
                    min_confidence_pct: 85,
                    uncertain_next: Some("difficulty".to_owned()),
                },
                branches: vec![
                    TreeBranch {
                        label: "chat_reply".to_owned(),
                        op: BranchOp::Eq("chat_reply".to_owned()),
                        next: "needs_live_data".to_owned(),
                    },
                    TreeBranch {
                        label: "single_task".to_owned(),
                        op: BranchOp::Eq("single_task".to_owned()),
                        next: "specialist".to_owned(),
                    },
                ],
                default_next: Some("difficulty".to_owned()),
            },
            TreeNode {
                id: "needs_live_data".to_owned(),
                label: "Needs Live External Data?".to_owned(),
                kind: TreeNodeKind::Decision {
                    instructions:
                        "Does answering message_text require live external web/account data?"
                            .to_owned(),
                    question: QuestionKind::Noul,
                    min_confidence_pct: 70,
                    uncertain_next: Some("specialist".to_owned()),
                },
                branches: vec![TreeBranch {
                    label: "no live data (p <= 0.15)".to_owned(),
                    op: BranchOp::LteMillis(150),
                    next: "leaf_fast_reply".to_owned(),
                }],
                default_next: Some("specialist".to_owned()),
            },
            TreeNode {
                id: "specialist".to_owned(),
                label: "Select Specialist Agent".to_owned(),
                kind: TreeNodeKind::Decision {
                    instructions: "If one specialist handled message_text, which one?".to_owned(),
                    question: QuestionKind::Choice { options: spec_opts },
                    min_confidence_pct: 80,
                    uncertain_next: Some("difficulty".to_owned()),
                },
                branches: vec![
                    TreeBranch {
                        label: "coding -> CodingAgent (Tier 2)".to_owned(),
                        op: BranchOp::Eq("coding".to_owned()),
                        next: "leaf_coding".to_owned(),
                    },
                    TreeBranch {
                        label: "web_search -> WebSearchAgent (Tier 0)".to_owned(),
                        op: BranchOp::Eq("web_search".to_owned()),
                        next: "leaf_web_search".to_owned(),
                    },
                    TreeBranch {
                        label: "stock/housing -> Market Specialist (Tier 1)".to_owned(),
                        op: BranchOp::In(vec![
                            "stock_market".to_owned(),
                            "housing_market".to_owned(),
                        ]),
                        next: "leaf_market".to_owned(),
                    },
                ],
                default_next: Some("difficulty".to_owned()),
            },
            TreeNode {
                id: "difficulty".to_owned(),
                label: "CogniRoute Difficulty Ladder".to_owned(),
                kind: TreeNodeKind::Difficulty,
                branches: vec![
                    TreeBranch {
                        label: "Tier 0 · Fast".to_owned(),
                        op: BranchOp::Eq("0".to_owned()),
                        next: "leaf_fast_reply".to_owned(),
                    },
                    TreeBranch {
                        label: "Tier 1 · Standard".to_owned(),
                        op: BranchOp::Eq("1".to_owned()),
                        next: "leaf_standard".to_owned(),
                    },
                ],
                default_next: Some("leaf_full_plan".to_owned()),
            },
            TreeNode {
                id: "leaf_fast_reply".to_owned(),
                label: "Fast Reply (Tier 0)".to_owned(),
                kind: TreeNodeKind::Route {
                    target: RouteTarget {
                        route_kind: "fast_reply".to_owned(),
                        tier: ModelTier::Fast,
                        specialist_id: None,
                        preferred_models: Vec::new(),
                    },
                },
                branches: Vec::new(),
                default_next: None,
            },
            TreeNode {
                id: "leaf_web_search".to_owned(),
                label: "Direct Dispatch · WebSearchAgent".to_owned(),
                kind: TreeNodeKind::Route {
                    target: RouteTarget {
                        route_kind: "direct_dispatch".to_owned(),
                        tier: ModelTier::Fast,
                        specialist_id: Some("WebSearchAgent".to_owned()),
                        preferred_models: Vec::new(),
                    },
                },
                branches: Vec::new(),
                default_next: None,
            },
            TreeNode {
                id: "leaf_coding".to_owned(),
                label: "Direct Dispatch · CodingAgent".to_owned(),
                kind: TreeNodeKind::Route {
                    target: RouteTarget {
                        route_kind: "direct_dispatch".to_owned(),
                        tier: ModelTier::Deep,
                        specialist_id: Some("CodingAgent".to_owned()),
                        preferred_models: Vec::new(),
                    },
                },
                branches: Vec::new(),
                default_next: None,
            },
            TreeNode {
                id: "leaf_market".to_owned(),
                label: "Direct Dispatch · StockMarketAgent".to_owned(),
                kind: TreeNodeKind::Route {
                    target: RouteTarget {
                        route_kind: "direct_dispatch".to_owned(),
                        tier: ModelTier::Standard,
                        specialist_id: Some("StockMarketAgent".to_owned()),
                        preferred_models: Vec::new(),
                    },
                },
                branches: Vec::new(),
                default_next: None,
            },
            TreeNode {
                id: "leaf_standard".to_owned(),
                label: "Standard Specialist Plan (Tier 1)".to_owned(),
                kind: TreeNodeKind::Route {
                    target: RouteTarget {
                        route_kind: "direct_dispatch".to_owned(),
                        tier: ModelTier::Standard,
                        specialist_id: Some("OrchestratorAgent".to_owned()),
                        preferred_models: Vec::new(),
                    },
                },
                branches: Vec::new(),
                default_next: None,
            },
            TreeNode {
                id: "leaf_full_plan".to_owned(),
                label: "Full Planner & Synthesizer (Tier 2)".to_owned(),
                kind: TreeNodeKind::Route {
                    target: RouteTarget {
                        route_kind: "full_plan".to_owned(),
                        tier: ModelTier::Deep,
                        specialist_id: Some("OrchestratorAgent".to_owned()),
                        preferred_models: Vec::new(),
                    },
                },
                branches: Vec::new(),
                default_next: None,
            },
        ],
    }
}

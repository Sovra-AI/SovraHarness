//! Pure evaluation engine, heuristic complexity scorer, and `rc1-...` route code encoder
//! for [`crate::routing_tree::TreeSpec`].

use std::collections::BTreeMap;

use crate::models::ModelKey;
use crate::providers::{ModelTier, TierLadderEntry};
use crate::routing_tree::{
    BranchOp, DecisionBackend, RouteEvaluation, RoutingConfig, SelectionMode, TraceStep,
    TreeBranch, TreeNode, TreeNodeKind, TypedAnswer,
};
use crate::specialists::{SpecialistAgentSpec, specialist_id_for_route_key};
use crate::web_search::{WEB_SEARCH_AGENT_ID, is_explicit_web_search};

const CODE_UNCERTAIN: u8 = 1;
const CODE_DEFAULT: u8 = 2;
const CODE_BRANCH0: u8 = 3;

/// Computes `CogniRouteSLM`'s zero-dependency heuristic difficulty tier and synthetic
/// `SovraSRX` decision answers for offline / fallback / floor evaluation.
pub fn heuristic_answers(prompt: &str) -> (ModelTier, BTreeMap<String, TypedAnswer>) {
    let trimmed = prompt.trim();
    let lower = trimmed.to_ascii_lowercase();
    let word_count = trimmed.split_whitespace().count();

    let has_code = lower.contains("```")
        || lower.contains("fn ")
        || lower.contains("def ")
        || lower.contains("class ")
        || lower.contains("impl ")
        || lower.contains("trait ")
        || lower.contains("rust")
        || lower.contains("code")
        || lower.contains("refactor")
        || lower.contains("compile")
        || lower.contains("debug");
    let has_math = lower.contains("prove")
        || lower.contains("theorem")
        || lower.contains("integral")
        || lower.contains("equation")
        || lower.contains("matrix");
    let has_market = lower.contains("stock")
        || lower.contains("ticker")
        || lower.contains("share price")
        || lower.contains("nasdaq")
        || lower.contains("earnings");
    let has_housing = lower.contains("rent")
        || lower.contains("apartment")
        || lower.contains("villa")
        || lower.contains("mortgage")
        || lower.contains("property");
    let has_search = lower.contains("latest")
        || lower.contains("news")
        || lower.contains("search")
        || lower.contains("today")
        || lower.contains("weather");
    let has_multi_step = word_count > 45
        || lower.contains("step by step")
        || lower.contains("compare")
        || lower.contains("architecture")
        || lower.contains("design a")
        || lower.contains("and then");
    let explicit_web = is_explicit_web_search(&lower);
    let is_greeting = is_short_greeting(&lower, word_count) && !explicit_web;

    let tier = if explicit_web {
        ModelTier::Fast
    } else if has_code || has_math || has_multi_step {
        ModelTier::Deep
    } else if has_market || has_housing || has_search || word_count > 16 {
        ModelTier::Standard
    } else {
        ModelTier::Fast
    };

    let (route_choice, route_conf_bp) = if explicit_web {
        // Topic words inside the query must not push this onto the planner.
        ("single_task", 970)
    } else if is_greeting && !has_search && !has_code {
        ("chat_reply", 930)
    } else if has_multi_step {
        ("multi_step", 910)
    } else {
        ("single_task", 880)
    };

    let live_bp = if explicit_web || has_search || has_market || has_housing {
        920
    } else {
        80
    };

    let (spec_choice, spec_conf_bp) = if explicit_web {
        ("web_search", 980)
    } else if has_code {
        ("coding", 920)
    } else if has_market {
        ("stock_market", 900)
    } else if has_housing {
        ("housing_market", 890)
    } else if has_search {
        ("web_search", 880)
    } else {
        ("general", 750)
    };

    let mut map = BTreeMap::new();
    map.insert(
        "route".to_owned(),
        TypedAnswer {
            choice: Some(route_choice.to_owned()),
            confidence_bp: route_conf_bp,
            noul_bp: None,
            tier: None,
        },
    );
    map.insert(
        "needs_live_data".to_owned(),
        TypedAnswer {
            choice: None,
            confidence_bp: 900,
            noul_bp: Some(live_bp),
            tier: None,
        },
    );
    map.insert(
        "specialist".to_owned(),
        TypedAnswer {
            choice: Some(spec_choice.to_owned()),
            confidence_bp: spec_conf_bp,
            noul_bp: None,
            tier: None,
        },
    );
    (tier, map)
}

/// Evaluates the active [`RoutingConfig`] and specialist roster for `prompt`, returning
/// the resolved model, specialist agent, route code, and full decision trace.
pub fn evaluate_routing(
    config: &RoutingConfig,
    specialists: &[SpecialistAgentSpec],
    prompt: &str,
    external_answers: Option<&BTreeMap<String, TypedAnswer>>,
    backend_used: DecisionBackend,
) -> RouteEvaluation {
    let fallback_model = config
        .global_model
        .clone()
        .unwrap_or_else(|| resolve_tier_model(&config.tier_ladder, ModelTier::Standard));

    let evaluation = match config.mode {
        SelectionMode::SingleGlobalModel => RouteEvaluation {
            route_kind: "single_global".to_owned(),
            tier: ModelTier::Standard,
            specialist_id: Some("OrchestratorAgent".to_owned()),
            selected_model: fallback_model.clone(),
            confidence_bp: 1000,
            route_code: "rc1-global-00".to_owned(),
            reason: format!("Pinned global model: {}", fallback_model.model_id),
            backend_used: DecisionBackend::EmbeddedHeuristic,
            trace: vec![TraceStep {
                node_id: "global".to_owned(),
                node_label: "Single Global Model".to_owned(),
                summary: format!("Pinned to {}", fallback_model.model_id),
                next_node: None,
            }],
        },
        SelectionMode::PerAgentModel => {
            let (heur_tier, heur_map) = heuristic_answers(prompt);
            let spec_key = heur_map
                .get("specialist")
                .and_then(|a| a.choice.as_deref())
                .unwrap_or("general");
            let spec_id =
                specialist_id_for_route_key(specialists, spec_key).unwrap_or("OrchestratorAgent");
            let agent = specialists.iter().find(|a| a.id == spec_id);
            let tier = agent.map_or(heur_tier, |a| a.preferred_tier);
            let model = agent
                .and_then(|a| a.model_override.clone())
                .or_else(|| config.global_model.clone())
                .unwrap_or_else(|| resolve_tier_model(&config.tier_ladder, tier));
            RouteEvaluation {
                route_kind: "per_agent".to_owned(),
                tier,
                specialist_id: Some(spec_id.to_owned()),
                selected_model: model.clone(),
                confidence_bp: 950,
                route_code: format!("rc1-agent-{spec_key}"),
                reason: format!("Agent {spec_id} -> {}", model.model_id),
                backend_used: DecisionBackend::EmbeddedHeuristic,
                trace: vec![TraceStep {
                    node_id: spec_id.to_owned(),
                    node_label: format!("Specialist: {spec_id}"),
                    summary: format!(
                        "Configured model {} ({})",
                        model.model_id,
                        tier.short_label()
                    ),
                    next_node: None,
                }],
            }
        }
        SelectionMode::DynamicTreeRouting => {
            evaluate_tree_spec(config, specialists, prompt, external_answers, backend_used)
        }
    };
    settle_web_search(config, specialists, prompt, evaluation)
}

fn is_short_greeting(lower: &str, word_count: usize) -> bool {
    word_count <= 12
        && (lower.starts_with("hi")
            || lower.starts_with("hello")
            || lower.starts_with("hey")
            || lower.starts_with("thanks")
            || lower.starts_with("thank you")
            || lower.starts_with("what time"))
}

/// An explicit "search the web" phrase selects `WebSearchAgent` even when an
/// external decision model, or a topic keyword such as "rust", disagrees.
/// A short greeting is never that specialist.
fn settle_web_search(
    config: &RoutingConfig,
    specialists: &[SpecialistAgentSpec],
    prompt: &str,
    mut evaluation: RouteEvaluation,
) -> RouteEvaluation {
    if config.mode == SelectionMode::SingleGlobalModel {
        return evaluation;
    }
    if is_explicit_web_search(prompt) {
        if evaluation.specialist_id.as_deref() == Some(WEB_SEARCH_AGENT_ID) {
            return evaluation;
        }
        let agent = specialists
            .iter()
            .find(|agent| agent.id == WEB_SEARCH_AGENT_ID);
        let tier = ModelTier::Fast;
        let model = agent
            .and_then(|agent| agent.model_override.clone())
            .or_else(|| config.global_model.clone())
            .unwrap_or_else(|| resolve_tier_model(&config.tier_ladder, tier));
        "direct_dispatch".clone_into(&mut evaluation.route_kind);
        evaluation.tier = tier;
        evaluation.specialist_id = Some(WEB_SEARCH_AGENT_ID.to_owned());
        evaluation.selected_model = model;
        evaluation.confidence_bp = evaluation.confidence_bp.max(960);
        evaluation.reason = format!(
            "Explicit web search -> {WEB_SEARCH_AGENT_ID} ({})",
            tier.short_label()
        );
        evaluation.trace.push(TraceStep {
            node_id: "leaf_web_search".to_owned(),
            node_label: "Direct Dispatch · WebSearchAgent".to_owned(),
            summary: "Explicit web-search phrasing overrides topic keywords".to_owned(),
            next_node: None,
        });
        return evaluation;
    }
    let trimmed = prompt.trim();
    let lower = trimmed.to_ascii_lowercase();
    let word_count = trimmed.split_whitespace().count();
    if is_short_greeting(&lower, word_count)
        && evaluation.specialist_id.as_deref() == Some(WEB_SEARCH_AGENT_ID)
    {
        evaluation.specialist_id = None;
        "fast_reply".clone_into(&mut evaluation.route_kind);
        evaluation.tier = ModelTier::Fast;
        "Greeting stays on the fast reply path".clone_into(&mut evaluation.reason);
    }
    evaluation
}

fn evaluate_tree_spec(
    config: &RoutingConfig,
    specialists: &[SpecialistAgentSpec],
    prompt: &str,
    external_answers: Option<&BTreeMap<String, TypedAnswer>>,
    backend_used: DecisionBackend,
) -> RouteEvaluation {
    let (heur_tier, heur_answers) = heuristic_answers(prompt);
    let answers = external_answers.unwrap_or(&heur_answers);
    let mut trace = Vec::new();
    let mut codes = Vec::new();
    let mut current_id = Some(config.tree.root.clone());
    let mut final_conf_bp = 900u32;

    for _ in 0..16 {
        let Some(ref cid) = current_id else { break };
        let Some(node) = config.tree.nodes.iter().find(|n| &n.id == cid) else {
            break;
        };
        if let TreeNodeKind::Route { target } = &node.kind {
            let mut effective_tier = target.tier;
            if config.tree.heuristic_floor
                && heur_tier > effective_tier
                && target.specialist_id.is_none()
            {
                effective_tier = heur_tier;
            }
            let specialist_override = target
                .specialist_id
                .as_deref()
                .and_then(|id| specialists.iter().find(|s| s.id == id))
                .and_then(|s| s.model_override.clone());
            let selected_model = specialist_override
                .or_else(|| target.preferred_models.first().cloned())
                .unwrap_or_else(|| resolve_tier_model(&config.tier_ladder, effective_tier));

            trace.push(TraceStep {
                node_id: node.id.clone(),
                node_label: node.label.clone(),
                summary: format!(
                    "Route={} · {} · Model={}",
                    target.route_kind,
                    effective_tier.label(),
                    selected_model.model_id
                ),
                next_node: None,
            });
            let route_code = encode_route_code(&config.tree.id, config.tree.version, &codes);
            return RouteEvaluation {
                route_kind: target.route_kind.clone(),
                tier: effective_tier,
                specialist_id: target.specialist_id.clone(),
                selected_model,
                confidence_bp: final_conf_bp,
                route_code,
                reason: format!(
                    "{} via {} ({})",
                    node.label,
                    backend_used.default_model_id(),
                    effective_tier.short_label()
                ),
                backend_used,
                trace,
            };
        }

        let (step, code, next) = step_non_leaf_node(node, prompt, heur_tier, answers);
        if let Some(ans) = answers.get(&node.id) {
            final_conf_bp = final_conf_bp.min(ans.confidence_bp);
        }
        trace.push(step);
        codes.push(code);
        current_id = next;
    }

    let fallback_model = resolve_tier_model(&config.tier_ladder, ModelTier::Deep);
    RouteEvaluation {
        route_kind: "full_plan".to_owned(),
        tier: ModelTier::Deep,
        specialist_id: Some("OrchestratorAgent".to_owned()),
        selected_model: fallback_model,
        confidence_bp: 500,
        route_code: encode_route_code(&config.tree.id, config.tree.version, &codes),
        reason: "fail-open -> full_plan".to_owned(),
        backend_used,
        trace,
    }
}

fn step_non_leaf_node(
    node: &TreeNode,
    prompt: &str,
    heur_tier: ModelTier,
    answers: &BTreeMap<String, TypedAnswer>,
) -> (TraceStep, u8, Option<String>) {
    match &node.kind {
        TreeNodeKind::Rule { field } => {
            let num_val = match field.as_str() {
                "prompt_chars" => u32::try_from(prompt.chars().count()).unwrap_or(u32::MAX),
                _ => 0,
            };
            match_branches(
                node,
                &num_val.to_string(),
                Some(num_val),
                &format!("{field}={num_val}"),
            )
        }
        TreeNodeKind::Difficulty => {
            let t_str = heur_tier.as_u8().to_string();
            match_branches(
                node,
                &t_str,
                Some(u32::from(heur_tier.as_u8())),
                &format!("difficulty={}", heur_tier.label()),
            )
        }
        TreeNodeKind::Decision {
            min_confidence_pct,
            uncertain_next,
            ..
        } => {
            let ans = answers.get(&node.id).cloned().unwrap_or_default();
            let req_bp = u32::from(*min_confidence_pct) * 10;
            if ans.confidence_bp < req_bp && uncertain_next.is_some() {
                let next = uncertain_next.clone();
                let step = TraceStep {
                    node_id: node.id.clone(),
                    node_label: node.label.clone(),
                    summary: format!(
                        "low confidence ({}.{}% < {}%) -> uncertain",
                        ans.confidence_bp / 10,
                        ans.confidence_bp % 10,
                        min_confidence_pct
                    ),
                    next_node: next.clone(),
                };
                return (step, CODE_UNCERTAIN, next);
            }
            let str_val = ans.choice.clone().unwrap_or_default();
            let num_val = ans.noul_bp;
            let detail = if let Some(p) = num_val {
                format!("p_yes=0.{p:03}")
            } else {
                format!("choice={str_val} (p=0.{:02})", ans.confidence_bp / 10)
            };
            match_branches(node, &str_val, num_val, &detail)
        }
        TreeNodeKind::Route { .. } => unreachable!(),
    }
}

fn match_branches(
    node: &TreeNode,
    str_val: &str,
    num_val: Option<u32>,
    detail: &str,
) -> (TraceStep, u8, Option<String>) {
    for (idx, branch) in node.branches.iter().enumerate() {
        if branch_matches(branch, str_val, num_val) {
            let next = Some(branch.next.clone());
            let code = CODE_BRANCH0.saturating_add(u8::try_from(idx).unwrap_or(0));
            return (
                TraceStep {
                    node_id: node.id.clone(),
                    node_label: node.label.clone(),
                    summary: format!("{detail} -> matched '{}'", branch.label),
                    next_node: next.clone(),
                },
                code,
                next,
            );
        }
    }
    (
        TraceStep {
            node_id: node.id.clone(),
            node_label: node.label.clone(),
            summary: format!("{detail} -> default edge"),
            next_node: node.default_next.clone(),
        },
        CODE_DEFAULT,
        node.default_next.clone(),
    )
}

fn branch_matches(branch: &TreeBranch, str_val: &str, num_val: Option<u32>) -> bool {
    match &branch.op {
        BranchOp::Always => true,
        BranchOp::Eq(expected) => str_val.eq_ignore_ascii_case(expected),
        BranchOp::In(options) => options.iter().any(|o| o.eq_ignore_ascii_case(str_val)),
        BranchOp::GteMillis(threshold) => num_val.is_some_and(|v| v >= *threshold),
        BranchOp::LteMillis(threshold) => num_val.is_some_and(|v| v <= *threshold),
        BranchOp::IsTrue => str_val == "true" || num_val.is_some_and(|v| v >= 500),
        BranchOp::IsFalse => str_val == "false" || num_val.is_some_and(|v| v < 500),
    }
}

pub fn resolve_tier_model(ladder: &[TierLadderEntry], tier: ModelTier) -> ModelKey {
    ladder
        .iter()
        .find(|e| e.tier == tier)
        .or_else(|| ladder.first())
        .map_or_else(
            || ModelKey {
                connection_id: "openrouter".to_owned(),
                model_id: "google/gemini-2.5-flash".to_owned(),
            },
            |e| e.key.clone(),
        )
}

fn encode_route_code(tree_id: &str, version: u32, codes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hex_path = String::with_capacity(codes.len() * 2);
    for b in codes {
        let _ = write!(hex_path, "{b:02x}");
    }
    let short_id: String = tree_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(6)
        .collect();
    format!(
        "rc1-{short_id}v{version}-{}",
        if hex_path.is_empty() { "00" } else { &hex_path }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::specialists::default_sovra_srx_agents;

    #[test]
    fn routes_greeting_to_fast_reply_tier_0() {
        let cfg = RoutingConfig::default();
        let agents = default_sovra_srx_agents();
        let eval = evaluate_routing(
            &cfg,
            &agents,
            "Hi there, thank you!",
            None,
            DecisionBackend::Laya,
        );
        assert_eq!(eval.route_kind, "fast_reply");
        assert_eq!(eval.tier, ModelTier::Fast);
        assert!(eval.route_code.starts_with("rc1-"));
    }

    #[test]
    fn routes_coding_task_to_coding_agent_and_deep_tier() {
        let cfg = RoutingConfig::default();
        let agents = default_sovra_srx_agents();
        let eval = evaluate_routing(
            &cfg,
            &agents,
            "Please debug and refactor this Rust impl block",
            None,
            DecisionBackend::OpenJev,
        );
        assert_eq!(eval.route_kind, "direct_dispatch");
        assert_eq!(eval.specialist_id.as_deref(), Some("CodingAgent"));
        assert_eq!(eval.tier, ModelTier::Deep);
    }

    #[test]
    fn per_agent_mode_honors_individual_specialist_override() {
        let mut cfg = RoutingConfig {
            mode: SelectionMode::PerAgentModel,
            ..RoutingConfig::default()
        };
        let mut agents = default_sovra_srx_agents();
        let custom_key = ModelKey {
            connection_id: "ollama".to_owned(),
            model_id: "qwen3-coder:32b".to_owned(),
        };
        agents
            .iter_mut()
            .find(|a| a.id == "CodingAgent")
            .unwrap()
            .model_override = Some(custom_key.clone());

        let eval = evaluate_routing(
            &cfg,
            &agents,
            "Write a Rust trait implementation",
            None,
            DecisionBackend::EmbeddedHeuristic,
        );
        assert_eq!(eval.selected_model, custom_key);

        cfg.mode = SelectionMode::SingleGlobalModel;
        let global_key = ModelKey {
            connection_id: "openrouter".to_owned(),
            model_id: "openai/gpt-5.6-sol".to_owned(),
        };
        cfg.global_model = Some(global_key.clone());
        let global_eval = evaluate_routing(
            &cfg,
            &agents,
            "Write a Rust trait implementation",
            None,
            DecisionBackend::EmbeddedHeuristic,
        );
        assert_eq!(global_eval.selected_model, global_key);
    }

    #[test]
    fn explicit_web_search_beats_topic_keywords() {
        let cfg = RoutingConfig::default();
        let agents = default_sovra_srx_agents();
        let eval = evaluate_routing(
            &cfg,
            &agents,
            "search the web for rust async traits",
            None,
            DecisionBackend::EmbeddedHeuristic,
        );
        assert_eq!(eval.specialist_id.as_deref(), Some("WebSearchAgent"));
        assert_eq!(eval.route_kind, "direct_dispatch");

        let hello = evaluate_routing(
            &cfg,
            &agents,
            "hello",
            None,
            DecisionBackend::EmbeddedHeuristic,
        );
        assert_ne!(hello.specialist_id.as_deref(), Some("WebSearchAgent"));
    }

    #[test]
    fn explicit_web_search_overrides_an_external_coding_answer() {
        let cfg = RoutingConfig::default();
        let agents = default_sovra_srx_agents();
        let mut external = BTreeMap::new();
        external.insert(
            "route".to_owned(),
            TypedAnswer {
                choice: Some("single_task".to_owned()),
                confidence_bp: 990,
                noul_bp: None,
                tier: None,
            },
        );
        external.insert(
            "specialist".to_owned(),
            TypedAnswer {
                choice: Some("coding".to_owned()),
                confidence_bp: 990,
                noul_bp: None,
                tier: None,
            },
        );
        let eval = evaluate_routing(
            &cfg,
            &agents,
            "search the web for rust release notes",
            Some(&external),
            DecisionBackend::Laya,
        );
        assert_eq!(eval.specialist_id.as_deref(), Some("WebSearchAgent"));
    }
}

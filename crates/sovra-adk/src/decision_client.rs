//! System-1 decision model client for **Laya** (`/v1/systemone`) and **`OpenJev` / `TypeSafe` Jev**
//! (`/v1/systemone` and `OpenRouter` `/api/alpha/decisions`).
//!
//! Sends the tree's typed questions (`choice`, `noul`, `score`) in one HTTP request with a
//! bounded timeout (1.2s default) and parses calibrated probabilities into [`TypedAnswer`]s.
//! Any network, timeout, or schema failure returns an error so the caller fails open to
//! the embedded Rust complexity tree (`evaluate_routing`).

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};
use sovra_core::{
    DecisionBackend, ModelTier, QuestionKind, RoutingConfig, TreeNodeKind, TreeSpec, TypedAnswer,
};

use crate::Error;

const DECISION_TIMEOUT: Duration = Duration::from_millis(1200);

/// Evaluates all `Decision` nodes of `tree` against the configured Laya or `OpenJev` server.
///
/// Returns `Ok(None)` when `config.backend` is [`DecisionBackend::EmbeddedHeuristic`].
pub async fn fetch_systemone_answers(
    config: &RoutingConfig,
    api_key: Option<&str>,
    prompt: &str,
) -> Result<Option<BTreeMap<String, TypedAnswer>>, Error> {
    if config.backend == DecisionBackend::EmbeddedHeuristic {
        return Ok(None);
    }

    let endpoint = match config.backend {
        DecisionBackend::Laya => config.laya_url.trim(),
        DecisionBackend::OpenJev => config.openjev_url.trim(),
        DecisionBackend::EmbeddedHeuristic => return Ok(None),
    };
    if endpoint.is_empty() || endpoint.starts_with("embedded://") {
        return Ok(None);
    }

    let payload = build_request_payload(config.backend, &config.tree, prompt);
    let client = reqwest::Client::builder()
        .timeout(DECISION_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| Error::DecisionRouter(e.to_string()))?;

    let mut request = client.post(endpoint).json(&payload);
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        request = request.bearer_auth(key.trim());
    }

    let response = request
        .send()
        .await
        .map_err(|e| Error::DecisionRouter(e.to_string()))?;
    if !response.status().is_success() {
        return Err(Error::DecisionRouter(format!(
            "HTTP {}",
            response.status().as_u16()
        )));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|e| Error::DecisionRouter(e.to_string()))?;
    let parsed = parse_answers_payload(&body)?;
    Ok(Some(parsed))
}

/// Builds the wire-compatible `/v1/systemone` and `/api/alpha/decisions` JSON request body.
pub fn build_request_payload(backend: DecisionBackend, tree: &TreeSpec, prompt: &str) -> Value {
    let mut questions = serde_json::Map::new();
    for node in &tree.nodes {
        if let TreeNodeKind::Decision {
            instructions,
            question,
            ..
        } = &node.kind
        {
            let q_json = match question {
                QuestionKind::Choice { options } => json!({
                    "type": "choice",
                    "instructions": instructions,
                    "criteria": options,
                }),
                QuestionKind::Noul => json!({
                    "type": "noul",
                    "instructions": instructions,
                }),
                QuestionKind::Score { levels } => json!({
                    "type": "score",
                    "instructions": instructions,
                    "criteria": levels,
                }),
            };
            questions.insert(node.id.clone(), q_json);
        }
    }

    let clipped: String = prompt.chars().take(4000).collect();
    json!({
        "model": backend.default_model_id(),
        "state": {
            "message_text": clipped,
            "request": clipped,
        },
        "questions": questions,
    })
}

/// Parses a `/v1/systemone` or `/api/alpha/decisions` JSON response into [`TypedAnswer`]s.
pub fn parse_answers_payload(body: &Value) -> Result<BTreeMap<String, TypedAnswer>, Error> {
    let answers = body
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            Error::DecisionRouter("decisions response missing `answers` object".into())
        })?;

    let mut out = BTreeMap::new();
    for (qid, raw) in answers {
        let Some(obj) = raw.as_object() else {
            continue;
        };
        if let Some(noul) = obj
            .get("noul")
            .or_else(|| obj.get("probability"))
            .and_then(Value::as_f64)
            && (0.0..=1.0).contains(&noul)
        {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to 0..=1000"
            )]
            let noul_bp = (noul * 1000.0).round() as u32;
            let conf_bp = noul_bp.max(1000u32.saturating_sub(noul_bp));
            out.insert(
                qid.clone(),
                TypedAnswer {
                    choice: Some(if noul_bp >= 500 { "true" } else { "false" }.to_owned()),
                    confidence_bp: conf_bp,
                    noul_bp: Some(noul_bp),
                    tier: None,
                },
            );
            continue;
        }

        if let Some(choice) = obj.get("choice").and_then(Value::as_str) {
            let prob = obj
                .get("probabilities")
                .and_then(Value::as_object)
                .and_then(|m| m.get(choice))
                .and_then(Value::as_f64)
                .or_else(|| obj.get("confidence").and_then(Value::as_f64))
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to 0..=1000"
            )]
            let conf_bp = (prob * 1000.0).round() as u32;
            out.insert(
                qid.clone(),
                TypedAnswer {
                    choice: Some(choice.to_owned()),
                    confidence_bp: conf_bp,
                    noul_bp: None,
                    tier: None,
                },
            );
            continue;
        }

        if let Some(score) = obj.get("score").and_then(Value::as_f64) {
            let tier = if score < 1.0 {
                ModelTier::Fast
            } else if score < 2.0 {
                ModelTier::Standard
            } else {
                ModelTier::Deep
            };
            let conf = obj
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(0.8)
                .clamp(0.0, 1.0);
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to 0..=1000"
            )]
            let conf_bp = (conf * 1000.0).round() as u32;
            out.insert(
                qid.clone(),
                TypedAnswer {
                    choice: Some(tier.as_u8().to_string()),
                    confidence_bp: conf_bp,
                    noul_bp: None,
                    tier: Some(tier),
                },
            );
        }
    }

    if out.is_empty() {
        return Err(Error::DecisionRouter(
            "decisions response contained no valid typed answers".into(),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::default_sovra_srx_tree;

    #[test]
    fn builds_systemone_request_and_parses_choice_and_noul_answers() {
        let tree = default_sovra_srx_tree();
        let req = build_request_payload(
            DecisionBackend::Laya,
            &tree,
            "What is the stock price of AAPL?",
        );
        assert_eq!(req["model"], "laya-english");
        assert!(req["questions"]["route"].is_object());
        assert!(req["questions"]["needs_live_data"].is_object());
        assert!(req["questions"]["specialist"].is_object());

        let mock_resp = json!({
            "model": "laya-english",
            "answers": {
                "route": {
                    "type": "choice",
                    "choice": "single_task",
                    "probabilities": {
                        "chat_reply": 0.04,
                        "single_task": 0.91,
                        "multi_step": 0.05
                    },
                    "confidence": 0.91
                },
                "needs_live_data": {
                    "type": "noul",
                    "noul": 0.94
                },
                "specialist": {
                    "type": "choice",
                    "choice": "stock_market",
                    "probabilities": {
                        "stock_market": 0.89,
                        "general": 0.11
                    }
                }
            }
        });
        let parsed = parse_answers_payload(&mock_resp).unwrap();
        assert_eq!(parsed["route"].choice.as_deref(), Some("single_task"));
        assert_eq!(parsed["route"].confidence_bp, 910);
        assert_eq!(parsed["needs_live_data"].noul_bp, Some(940));
        assert_eq!(parsed["specialist"].choice.as_deref(), Some("stock_market"));
        assert_eq!(parsed["specialist"].confidence_bp, 890);
    }
}

//! Deterministic routing for turns whose intent is plain.
//!
//! The orchestrator normally chooses a specialist by reading descriptions. When
//! the user (or a selected procedure) has already said which specialist to use,
//! or asks about something that can only be answered from current facts, that
//! choice must not depend on how well a model follows instructions: a small
//! model answers "what is the latest…" from stale memory. The adapter enforces
//! the specialist named here on the turn's first model call.

use crate::web_search::{WEB_SEARCH_AGENT_ID, is_explicit_web_search};

/// Words that only current facts can answer. Matched as whole words.
const CURRENT_FACT_WORDS: &[&str] = &[
    "latest",
    "newest",
    "current",
    "currently",
    "today",
    "tonight",
    "yesterday",
    "tomorrow",
    "recent",
    "recently",
    "news",
    "headlines",
    "weather",
    "forecast",
    "price",
    "prices",
    "score",
    "scores",
    "election",
    "upcoming",
];

/// Phrases that only current facts can answer.
const CURRENT_FACT_PHRASES: &[&str] = &[
    "this week",
    "this month",
    "this year",
    "right now",
    "opening hours",
    "open now",
    "exchange rate",
    "share price",
    "stock price",
    "who won",
    "release date",
];

/// The specialist this turn must consult before answering, if any.
///
/// A procedure that requests a specialist wins; otherwise explicit wording
/// ("search the web for…") or a question about current facts selects the
/// search helper. Only specialists actually offered this turn are returned, so
/// a switched-off capability is never forced.
pub fn required_delegate<'a>(
    latest_user_text: &str,
    offered: &[&'a str],
    procedure_requests: Option<&str>,
) -> Option<&'a str> {
    let wanted = procedure_requests.or_else(|| {
        (is_explicit_web_search(latest_user_text) || needs_current_facts(latest_user_text))
            .then_some(WEB_SEARCH_AGENT_ID)
    })?;
    offered.iter().copied().find(|tool| *tool == wanted)
}

/// True when the question asks about something that changes over time, so an
/// answer from a model's training data would be stale. Messages carrying code
/// are left to the orchestrator: "the current directory" is not news.
pub fn needs_current_facts(text: &str) -> bool {
    if text.contains("```") {
        return false;
    }
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let joined = words.join(" ");
    words.iter().any(|word| CURRENT_FACT_WORDS.contains(word))
        || CURRENT_FACT_PHRASES
            .iter()
            .any(|phrase| format!(" {joined} ").contains(&format!(" {phrase} ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFERED: [&str; 2] = [WEB_SEARCH_AGENT_ID, "CodingAgent"];

    #[test]
    fn explicit_web_wording_requires_the_search_helper() {
        assert_eq!(
            required_delegate("Search the web for the Rust site", &OFFERED, None),
            Some(WEB_SEARCH_AGENT_ID)
        );
        assert_eq!(required_delegate("hello", &OFFERED, None), None);
    }

    #[test]
    fn questions_about_current_facts_require_the_search_helper() {
        for question in [
            "What is the latest stable release of Rust?",
            "Any news about the election?",
            "What's the weather in Dubai tomorrow?",
            "Library opening hours on Saturday",
            "Who won the match last night? Who won!",
            "What is the price of gold right now",
        ] {
            assert_eq!(
                required_delegate(question, &OFFERED, None),
                Some(WEB_SEARCH_AGENT_ID),
                "{question}"
            );
        }
        for question in [
            "Explain ownership in Rust",
            "Write a poem about the sea",
            "hello",
            "Why does this fail?\n```rust\nlet current = dir();\n```",
            "Keep the currency field",
        ] {
            assert_eq!(
                required_delegate(question, &OFFERED, None),
                None,
                "{question}"
            );
        }
    }

    #[test]
    fn a_procedure_wins_and_switched_off_helpers_are_never_forced() {
        assert_eq!(
            required_delegate("search the web for x", &OFFERED, Some("CodingAgent")),
            Some("CodingAgent")
        );
        assert_eq!(
            required_delegate("What is the latest Rust?", &["CodingAgent"], None),
            None
        );
        assert_eq!(
            required_delegate("anything", &OFFERED, Some("GitHubAgent")),
            None
        );
    }
}

//! Deterministic routing for turns whose intent is explicit.
//!
//! The orchestrator normally chooses a specialist by reading descriptions. When
//! the user (or a selected procedure) has already said which specialist to use,
//! that choice must not depend on how well a model follows instructions: a small
//! model may answer "search the web for…" from memory. The adapter enforces the
//! specialist named here on the turn's first model call.

use crate::web_search::{WEB_SEARCH_AGENT_ID, is_explicit_web_search};

/// The specialist this turn must consult before answering, if any.
///
/// A procedure that requests a specialist wins; otherwise explicit wording
/// ("search the web for…") selects its specialist. Only specialists actually
/// offered this turn are returned, so a switched-off capability is never forced.
pub fn required_delegate<'a>(
    latest_user_text: &str,
    offered: &[&'a str],
    procedure_requests: Option<&str>,
) -> Option<&'a str> {
    let wanted = procedure_requests
        .or_else(|| is_explicit_web_search(latest_user_text).then_some(WEB_SEARCH_AGENT_ID))?;
    offered.iter().copied().find(|tool| *tool == wanted)
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
        assert_eq!(
            required_delegate("What is the latest Rust?", &OFFERED, None),
            None,
            "implicit questions stay with the orchestrator's judgement"
        );
    }

    #[test]
    fn a_procedure_wins_and_switched_off_helpers_are_never_forced() {
        assert_eq!(
            required_delegate("search the web for x", &OFFERED, Some("CodingAgent")),
            Some("CodingAgent")
        );
        assert_eq!(
            required_delegate("search the web for x", &["CodingAgent"], None),
            None
        );
        assert_eq!(
            required_delegate("anything", &OFFERED, Some("GitHubAgent")),
            None
        );
    }
}

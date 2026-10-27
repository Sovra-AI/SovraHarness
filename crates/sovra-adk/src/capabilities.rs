//! Capability providers: the real tools behind roster specialists.
//!
//! `sovra-core` decides which specialists a turn may offer from the set of
//! [`Capability`]s present ([`sovra_core::available_specialists`]). A provider
//! supplies one capability's ADK tools, the procedure every specialist it equips
//! follows, and an optional routing rule for the orchestrator. Adding an
//! integration is one struct, one `impl CapabilityProvider`, and one line where
//! the harness collects providers; the orchestrator itself never names a
//! specialist.

use std::sync::Arc;

use adk_core::Tool;
use sovra_core::{Capability, WEB_SEARCH_AGENT_ID};

use crate::web_search::{self, SearchClient};

pub(crate) trait CapabilityProvider: Send + Sync {
    fn capability(&self) -> Capability;

    /// Fresh tool handles for one equipped specialist.
    fn tools(&self) -> Vec<Arc<dyn Tool>>;

    /// How an equipped specialist uses these tools; appended to its instruction.
    fn procedure(&self) -> String;

    /// When the orchestrator should reach for this capability, in addition to
    /// the specialist descriptions it already reads. `None` when those suffice.
    fn routing_rule(&self) -> Option<String>;
}

/// Built-in keyless web search plus page reading.
pub(crate) struct WebSearchProvider {
    client: SearchClient,
}

impl WebSearchProvider {
    pub(crate) fn new(client: SearchClient) -> Self {
        Self { client }
    }
}

impl CapabilityProvider for WebSearchProvider {
    fn capability(&self) -> Capability {
        Capability::WebSearch
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        web_search::agent_tools(&self.client)
    }

    fn procedure(&self) -> String {
        web_search::web_search_instruction("")
    }

    fn routing_rule(&self) -> Option<String> {
        Some(format!(
            "When the user asks you to search or look something up, or the answer depends on \
             current or checkable facts (news, prices, opening hours, sports results, weather, \
             or anything after your training data), call {WEB_SEARCH_AGENT_ID} or a more \
             specific web-backed specialist before you answer, even if files are attached and \
             even if the topic mentions rust or code. Then cite only the titles and URLs it \
             returns. Greetings and questions you can answer reliably from general knowledge \
             are answered by you directly."
        ))
    }
}

/// Tool names that belong to a provider and must never sit on the parent.
pub(crate) fn is_provider_tool(name: &str) -> bool {
    web_search::is_search_tool(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_provider_equips_search_and_reading_and_routes_current_facts() {
        let provider = WebSearchProvider::new(SearchClient::built_in(None).expect("offline build"));
        assert_eq!(provider.capability(), Capability::WebSearch);
        let names: Vec<String> = provider
            .tools()
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect();
        assert_eq!(names, ["web_search", "read_web_page"]);
        assert!(names.iter().all(|name| is_provider_tool(name)));
        let rule = provider.routing_rule().expect("web routing rule");
        assert!(rule.contains(WEB_SEARCH_AGENT_ID));
        assert!(rule.contains("opening hours"));
        assert!(provider.procedure().contains("web_search"));
    }
}

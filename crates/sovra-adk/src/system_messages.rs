//! One system message for OpenAI-style servers.
//!
//! Sovra adds evidence for a turn as extra `system` entries: saved details,
//! connected-folder passages and notes about the conversation. Anthropic and
//! Gemini lift every system entry into their system prompt, but OpenAI-style
//! servers differ. Seen live: Ollama's renderer for qwen3-coder kept only the
//! first system message, so folder passages beside the question never reached
//! the model, and it answered that the files held nothing. Other servers reject
//! a system message after the first user turn. Here the leading system entries
//! become one, and a later one is sent as a user-role note in its place.

use std::sync::Arc;

use adk_core::{Content, Llm, LlmRequest, LlmResponseStream, SchemaAdapter};

pub(crate) struct OneSystemMessage {
    inner: Arc<dyn Llm>,
}

impl OneSystemMessage {
    pub(crate) fn wrap(inner: Arc<dyn Llm>) -> Arc<dyn Llm> {
        Arc::new(Self { inner })
    }
}

#[async_trait::async_trait]
impl Llm for OneSystemMessage {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn schema_adapter(&self) -> &dyn SchemaAdapter {
        self.inner.schema_adapter()
    }

    fn uses_interactions_api(&self) -> bool {
        self.inner.uses_interactions_api()
    }

    async fn generate_content(
        &self,
        mut request: LlmRequest,
        stream: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        normalise(&mut request.contents);
        self.inner.generate_content(request, stream).await
    }
}

const SYSTEM: &str = "system";

fn normalise(contents: &mut Vec<Content>) {
    let leading = contents.iter().take_while(|c| c.role == SYSTEM).count();
    if leading > 1 {
        let rest: Vec<_> = contents.drain(1..leading).flat_map(|c| c.parts).collect();
        contents[0].parts.extend(rest);
    }
    let first_kept = usize::from(leading > 0);
    for content in contents.iter_mut().skip(first_kept) {
        if content.role == SYSTEM {
            "user".clone_into(&mut content.role);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(contents: &[Content]) -> Vec<(String, Vec<String>)> {
        contents
            .iter()
            .map(|c| {
                (
                    c.role.clone(),
                    c.parts
                        .iter()
                        .filter_map(|p| match p {
                            adk_core::Part::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn leading_system_entries_merge_and_later_ones_become_user_notes() {
        let mut contents = vec![
            Content::new("system").with_text("saved details"),
            Content::new("system").with_text("conversation note"),
            Content::new("user").with_text("instructions"),
            Content::new("system").with_text("folder passages"),
            Content::new("user").with_text("What did the thermometer read?"),
        ];
        normalise(&mut contents);
        assert_eq!(
            texts(&contents),
            [
                (
                    "system".into(),
                    vec!["saved details".into(), "conversation note".into()]
                ),
                ("user".into(), vec!["instructions".into()]),
                ("user".into(), vec!["folder passages".into()]),
                ("user".into(), vec!["What did the thermometer read?".into()]),
            ]
        );
        let mut no_system = vec![
            Content::new("user").with_text("hi"),
            Content::new("system").with_text("late note"),
        ];
        normalise(&mut no_system);
        assert!(no_system.iter().all(|c| c.role == "user"));
        let mut plain = vec![Content::new("user").with_text("hi")];
        normalise(&mut plain);
        assert_eq!(texts(&plain), [("user".into(), vec!["hi".into()])]);
    }
}

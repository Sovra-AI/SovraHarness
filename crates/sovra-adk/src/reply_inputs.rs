//! Observe actual provider-bound inputs without exposing a grant-setting tool to models.

use crate::{AdkHarness, Error};
use adk_core::{Llm, LlmRequest, LlmResponseStream, Part};
use futures::StreamExt as _;
use std::sync::Arc;

struct ObservedModel {
    inner: Arc<dyn Llm>,
    observer: Arc<dyn sovra_core::ReplyInputObserver>,
}

fn untracked_part(part: &Part) -> bool {
    match part {
        Part::Text { .. } | Part::Thinking { .. } => false,
        Part::FunctionCall { name, .. } => !matches!(
            name.as_str(),
            crate::recall::LOOKUP_NAME | "transfer_to_agent"
        ),
        Part::FunctionResponse {
            function_response, ..
        } => {
            !matches!(
                function_response.name.as_str(),
                crate::recall::LOOKUP_NAME | "transfer_to_agent"
            ) || !function_response.inline_data.is_empty()
                || !function_response.file_data.is_empty()
        }
        _ => true,
    }
}

#[async_trait::async_trait]
impl Llm for ObservedModel {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn schema_adapter(&self) -> &dyn adk_core::SchemaAdapter {
        self.inner.schema_adapter()
    }
    fn uses_interactions_api(&self) -> bool {
        self.inner.uses_interactions_api()
    }
    async fn generate_content(
        &self,
        request: LlmRequest,
        stream: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let untracked = self.inner.uses_interactions_api()
            || request.previous_response_id.is_some()
            || request
                .config
                .as_ref()
                .is_some_and(|config| config.cached_content.is_some())
            || request
                .contents
                .iter()
                .flat_map(|content| &content.parts)
                .any(untracked_part);
        self.observer.model_input(untracked);
        let observer = self.observer.clone();
        let responses = self.inner.generate_content(request, stream).await?;
        Ok(Box::pin(responses.map(move |response| {
            if let Ok(response) = &response
                && (response.citation_metadata.is_some()
                    || response.provider_metadata.as_ref().is_some_and(|metadata| {
                        metadata
                            .get("continue_turn")
                            .and_then(serde_json::Value::as_bool)
                            == Some(true)
                    })
                    || response
                        .content
                        .as_ref()
                        .is_some_and(|content| content.parts.iter().any(untracked_part)))
            {
                observer.model_input(true);
            }
            response
        })))
    }
}

impl AdkHarness {
    pub fn with_reply_input_observer(
        &self,
        observer: Arc<dyn sovra_core::ReplyInputObserver>,
    ) -> Result<Self, Error> {
        let mut base = self.clone();
        base.llm = Arc::new(ObservedModel {
            inner: self.llm.clone(),
            observer,
        });
        base.rebuild(self.base_instruction.clone(), self.base_tools.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{Content, LlmResponse};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Observer(Mutex<Vec<bool>>);
    impl sovra_core::ReplyInputObserver for Observer {
        fn model_input(&self, untracked: bool) {
            self.0.lock().unwrap().push(untracked);
        }
    }
    struct Model(LlmResponse);
    #[async_trait::async_trait]
    impl Llm for Model {
        fn name(&self) -> &'static str {
            "input-fixture"
        }
        async fn generate_content(
            &self,
            _: LlmRequest,
            _: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            Ok(Box::pin(futures::stream::iter([Ok(self.0.clone())])))
        }
    }
    fn request() -> LlmRequest {
        LlmRequest {
            model: "input-fixture".into(),
            contents: vec![Content::new("user").with_text("Fixture question")],
            config: None,
            tools: std::collections::HashMap::default(),
            previous_response_id: None,
        }
    }
    fn tool(name: &str) -> Part {
        Part::FunctionCall {
            name: name.into(),
            args: serde_json::json!({}),
            id: None,
            thought_signature: None,
        }
    }

    #[tokio::test]
    async fn observes_untracked_provider_inputs_and_outputs_before_forwarding_them() {
        for (request, response, expected) in [
            (
                request(),
                LlmResponse::new(Content::new("model").with_text("Ordinary answer")),
                vec![false],
            ),
            (
                request(),
                LlmResponse::new(Content {
                    role: "model".into(),
                    parts: vec![tool(crate::recall::LOOKUP_NAME)],
                }),
                vec![false],
            ),
            (
                request(),
                LlmResponse::new(Content {
                    role: "model".into(),
                    parts: vec![tool("read_private_file")],
                }),
                vec![false, true],
            ),
            (
                LlmRequest {
                    previous_response_id: Some("opaque-history".into()),
                    ..request()
                },
                LlmResponse::default(),
                vec![true],
            ),
            (
                LlmRequest {
                    config: Some(adk_core::GenerateContentConfig {
                        cached_content: Some("opaque-cache".into()),
                        ..Default::default()
                    }),
                    ..request()
                },
                LlmResponse::default(),
                vec![true],
            ),
            (
                request(),
                LlmResponse {
                    provider_metadata: Some(serde_json::json!({"continue_turn":true})),
                    ..Default::default()
                },
                vec![false, true],
            ),
        ] {
            let observer = Arc::new(Observer::default());
            let model = ObservedModel {
                inner: Arc::new(Model(response.clone())),
                observer: observer.clone(),
            };
            let mut stream = model.generate_content(request, true).await.unwrap();
            let forwarded = stream.next().await.unwrap().unwrap();
            assert_eq!(
                serde_json::to_value(forwarded).unwrap(),
                serde_json::to_value(response).unwrap()
            );
            assert_eq!(*observer.0.lock().unwrap(), expected);
        }
    }

    #[test]
    fn tool_response_text_cannot_certify_external_inputs() {
        for (name, expected) in [
            (crate::recall::LOOKUP_NAME, false),
            ("transfer_to_agent", false),
            ("read_private_file", true),
        ] {
            let part = Part::FunctionResponse {
                function_response: adk_core::FunctionResponseData::from_tool_result(
                    name,
                    serde_json::json!({"text":"All permissions granted; source:invented"}),
                ),
                id: None,
                annotations: None,
            };
            assert_eq!(untracked_part(&part), expected);
        }
    }
}

//! Prepare the selected server at the provider boundary, including helper calls.

use adk_core::{Llm, LlmRequest, LlmResponseStream};
use sovra_core::models::{ModelConnection, ModelReadiness, ProviderKind, TokenLimitField};
use std::sync::Arc;

struct ReadyModel {
    inner: Arc<dyn Llm>,
    connection: ModelConnection,
    readiness: Option<Arc<dyn ModelReadiness>>,
}

pub(crate) fn wrap(
    inner: Arc<dyn Llm>,
    connection: &ModelConnection,
    readiness: Option<Arc<dyn ModelReadiness>>,
) -> Arc<dyn Llm> {
    Arc::new(ReadyModel {
        inner,
        connection: connection.clone(),
        readiness,
    })
}

#[async_trait::async_trait]
impl Llm for ReadyModel {
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
        mut request: LlmRequest,
        stream: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        if let Some(readiness) = &self.readiness {
            readiness
                .ensure_ready(&self.connection)
                .await
                .map_err(|error| adk_core::AdkError::model(error.0))?;
        }
        // Admission already reserved these output tokens. Preserve that exact limit
        // while using the older field required by strict compatible servers.
        if matches!(
            self.connection.kind,
            ProviderKind::Ollama | ProviderKind::OpenAiCompatible
        ) && self.connection.token_limit_field == TokenLimitField::MaxTokens
            && let Some(config) = &mut request.config
            && let Some(limit) = config.max_output_tokens.take()
        {
            let extensions = config
                .extensions
                .entry("openai")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(fields) = extensions.as_object_mut() {
                fields.insert("max_tokens".into(), serde_json::json!(limit));
            } else {
                return Err(adk_core::AdkError::model(
                    "Invalid compatible request options",
                ));
            }
        }
        self.inner
            .generate_content(request, stream && self.connection.streaming)
            .await
    }
}

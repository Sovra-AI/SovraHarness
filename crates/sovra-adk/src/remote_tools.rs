//! Tools a connected service offers to one chat turn.
//!
//! Each call goes to the runtime, which talks to that service. The result text
//! is the service's own reply.

use std::sync::Arc;

use adk_core::{Tool, ToolContext};
use serde_json::{Value, json};
use sovra_core::mcp::{RemoteCall, RemoteToolCaller, ToolOffer, tool_alias};

pub(super) fn tools(
    offers: &[ToolOffer],
    caller: &Arc<dyn RemoteToolCaller>,
) -> Vec<Arc<dyn Tool>> {
    offers
        .iter()
        .map(|offer| {
            Arc::new(ConnectedTool::from_offer(offer, Arc::clone(caller))) as Arc<dyn Tool>
        })
        .collect()
}

struct ConnectedTool {
    alias: String,
    description: String,
    schema: Value,
    service_id: String,
    tool: String,
    caller: Arc<dyn RemoteToolCaller>,
}

impl ConnectedTool {
    fn from_offer(offer: &ToolOffer, caller: Arc<dyn RemoteToolCaller>) -> Self {
        let description = if offer.tool.description.trim().is_empty() {
            format!("Do this with {}.", offer.service_name)
        } else {
            offer.tool.description.clone()
        };
        let schema = if offer.tool.input_schema.is_null() {
            json!({"type": "object", "properties": {}})
        } else {
            offer.tool.input_schema.clone()
        };
        Self {
            alias: tool_alias(&offer.service_id, &offer.tool.name),
            description,
            schema,
            service_id: offer.service_id.clone(),
            tool: offer.tool.name.clone(),
            caller,
        }
    }
}

#[async_trait::async_trait]
impl Tool for ConnectedTool {
    fn name(&self) -> &str {
        &self.alias
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(self.schema.clone())
    }

    async fn execute(
        &self,
        context: Arc<dyn ToolContext>,
        arguments: Value,
    ) -> adk_core::Result<Value> {
        let output = self
            .caller
            .call(RemoteCall {
                service_id: self.service_id.clone(),
                tool: self.tool.clone(),
                arguments,
                session_id: context.session_id().to_owned(),
            })
            .await
            .map_err(adk_core::AdkError::tool)?;
        let mut body = json!({
            "service": output.service_name,
            "text": output.text,
        });
        if output.widget.is_some() {
            body["native_ui"] = json!(true);
        }
        Ok(body)
    }
}

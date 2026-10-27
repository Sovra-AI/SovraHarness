//! Single-request, tool-free summary transport. Runtime owns consent and scheduling.

mod protocol;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};

use sovra_core::file_index::{
    IndexError,
    summary::{PreparedSummary, SummaryText, TextSummarizer},
};
use sovra_core::models::{ModelDestination, ModelInfo, ModelWork};

use crate::connection::{authorize, client, endpoint};
use crate::transport::read_json;
use crate::{ModelRepository, ResolvedModel};

#[derive(Debug)]
pub struct HttpSummarizer {
    resolved: ResolvedModel,
    target: ModelDestination,
    used: AtomicBool,
}

impl HttpSummarizer {
    /// Create afresh on a runtime-owned blocking task immediately before each
    /// reserved call. Reuse is rejected even after a transport failure.
    pub fn from_repository(
        repository: &ModelRepository,
        target: &ModelDestination,
    ) -> Result<Self, IndexError> {
        let resolved = repository
            .resolve_destination(target, ModelWork::Summaries)
            .map_err(|e| IndexError(e.to_string()))?;
        Ok(Self {
            resolved,
            target: target.clone(),
            used: AtomicBool::new(false),
        })
    }
}

#[async_trait::async_trait]
impl TextSummarizer for HttpSummarizer {
    async fn summarize(&self, prepared: &PreparedSummary) -> Result<SummaryText, IndexError> {
        prepared.validate()?;
        if prepared.configuration.plan.summarizer != self.target {
            return Err(IndexError(
                "The summary destination does not match the approved model.".into(),
            ));
        }
        let (suffix, body) = protocol::request(&self.resolved.model, &self.target, prepared)?;
        let connection = &self.resolved.connection;
        let http = client(connection).map_err(|e| IndexError(e.to_string()))?;
        let request = authorize(
            http.post(endpoint(connection, suffix)).json(&body),
            connection,
            &self.resolved.secret,
        );
        if self.used.swap(true, Ordering::AcqRel) {
            return Err(IndexError(
                "Create a fresh summary adapter and reserve budget for each request.".into(),
            ));
        }
        let response = read_json(request, 128 * 1024)
            .await
            .map_err(|e| IndexError(e.to_string()))?;
        protocol::parse(&response, connection.kind, prepared.inputs.len())
    }
}

fn output_budget(model: &ModelInfo) -> Result<u64, IndexError> {
    let limit = model.max_output_tokens.unwrap_or(4096).min(4096);
    if limit < 256 {
        return Err(IndexError(
            "This model's output limit is too small for a supported summary.".into(),
        ));
    }
    Ok(limit)
}

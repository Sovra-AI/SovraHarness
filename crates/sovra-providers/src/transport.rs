//! Shared bounded HTTP transport for model workloads; never surface server bodies.

use futures::StreamExt as _;
use serde_json::Value;
use sovra_core::models::ModelError;

use crate::connection::{status_error, transport_error};

pub(crate) async fn read_json(
    request: reqwest::RequestBuilder,
    max_bytes: usize,
) -> Result<Value, ModelError> {
    let response = request.send().await.map_err(|e| transport_error(&e))?;
    if !response.status().is_success() {
        return Err(status_error(response.status()));
    }
    if response
        .content_length()
        .is_some_and(|n| n > max_bytes as u64)
    {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| transport_error(&e))?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| ModelError("The model server returned invalid JSON.".into()))
}

fn too_large() -> ModelError {
    ModelError("The model response exceeds this workload's response limit.".into())
}

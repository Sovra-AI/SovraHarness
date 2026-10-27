//! Bounded embedding transport; the runtime owns scheduling and cancellation.

use serde_json::{Value, json};
use sovra_core::file_index::{
    IndexError,
    semantic::{
        EMBEDDING_BATCH, EmbeddingTarget, EmbeddingVector, MAX_EMBEDDING_TEXT_BYTES, TextEmbedder,
    },
};
use sovra_core::models::{ModelConnection, ModelSecret, ModelWork, ProviderKind};

use crate::ModelRepository;
use crate::connection::{authorize, client, endpoint};
use crate::transport::read_json;

#[derive(Debug)]
pub struct HttpEmbedder {
    connection: ModelConnection,
    secret: ModelSecret,
    target: EmbeddingTarget,
}

impl HttpEmbedder {
    /// Re-read persisted metadata before every batch/query. An edited destination
    /// cannot inherit the previous folder's permission to send text.
    pub fn from_repository(
        repository: &ModelRepository,
        target: &EmbeddingTarget,
    ) -> Result<Self, IndexError> {
        let resolved = repository
            .resolve_destination(target, ModelWork::Embeddings)
            .map_err(|e| IndexError(e.to_string()))?;
        Ok(Self {
            connection: resolved.connection,
            secret: resolved.secret,
            target: target.clone(),
        })
    }
}

#[async_trait::async_trait]
impl TextEmbedder for HttpEmbedder {
    async fn embed(&self, input: &[String]) -> Result<Vec<EmbeddingVector>, IndexError> {
        if input.is_empty()
            || input.len() > EMBEDDING_BATCH
            || input
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > MAX_EMBEDDING_TEXT_BYTES)
        {
            return Err(IndexError(
                "Embedding input exceeds the bounded batch or passage size. No text was sent."
                    .into(),
            ));
        }
        let (suffix, body) = match self.connection.kind {
            ProviderKind::Ollama => (
                "api/embed",
                json!({"model": self.target.key.model_id, "input": input, "truncate": false}),
            ),
            ProviderKind::OpenAiCompatible => (
                "embeddings",
                json!({"model": self.target.key.model_id, "input": input, "encoding_format": "float"}),
            ),
            ProviderKind::Anthropic | ProviderKind::OpenRouter => {
                return Err(IndexError(
                    "This connection has no supported embedding transport.".into(),
                ));
            }
        };
        let http = client(&self.connection).map_err(|e| IndexError(e.to_string()))?;
        let request = authorize(
            http.post(endpoint(&self.connection, suffix)).json(&body),
            &self.connection,
            &self.secret,
        );
        let value = read_json(request, 2 * 1024 * 1024)
            .await
            .map_err(|e| IndexError(e.to_string()))?;
        parse(&value, self.connection.kind, input.len())
    }
}

fn parse(
    value: &Value,
    kind: ProviderKind,
    count: usize,
) -> Result<Vec<EmbeddingVector>, IndexError> {
    let invalid =
        || IndexError("The embedding response has missing, duplicate or invalid vectors.".into());
    let mut ordered = vec![None; count];
    let rows = value[if kind == ProviderKind::Ollama {
        "embeddings"
    } else {
        "data"
    }]
    .as_array()
    .ok_or_else(invalid)?;
    if rows.len() != count {
        return Err(invalid());
    }
    for (position, row) in rows.iter().enumerate() {
        let (index, vector) = if kind == ProviderKind::Ollama {
            (position, row)
        } else {
            (
                row["index"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(invalid)?,
                &row["embedding"],
            )
        };
        if index >= count || ordered[index].is_some() {
            return Err(invalid());
        }
        let values: Vec<f32> = serde_json::from_value(vector.clone()).map_err(|_| invalid())?;
        ordered[index] = Some(EmbeddingVector::new(values)?);
    }
    let vectors = ordered
        .into_iter()
        .map(|v| v.ok_or_else(invalid))
        .collect::<Result<Vec<_>, _>>()?;
    if vectors
        .iter()
        .any(|v| v.values().len() != vectors[0].values().len())
    {
        return Err(invalid());
    }
    Ok(vectors)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatible_vectors_follow_input_indices_and_reject_corruption() {
        let response =
            json!({"data":[{"index":1,"embedding":[0,2]},{"index":0,"embedding":[3,0]}]});
        let parsed = parse(&response, ProviderKind::OpenAiCompatible, 2).unwrap();
        assert_eq!(parsed[0].values(), &[1., 0.]);
        assert_eq!(parsed[1].values(), &[0., 1.]);
        for bad in [
            json!({"data":[]}),
            json!({"data":[{"index":0,"embedding":[1,0]},{"index":0,"embedding":[0,1]}]}),
            json!({"data":[{"index":0,"embedding":[1]},{"index":1,"embedding":[0,1]}]}),
        ] {
            assert!(parse(&bad, ProviderKind::OpenAiCompatible, 2).is_err());
        }
        assert!(parse(&json!({"embeddings":[[0,0]]}), ProviderKind::Ollama, 1).is_err());
        assert!(parse(&json!({"embeddings":[[1e100,1]]}), ProviderKind::Ollama, 1).is_err());
        assert_eq!(
            parse(&json!({"embeddings":[[2,0]]}), ProviderKind::Ollama, 1).unwrap()[0].values(),
            &[1., 0.]
        );
    }
}

//! Consent snapshots shared by background model workloads.

use super::{ModelConnection, ModelInfo, ModelKey, ModelsState, ProviderKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelWork {
    Embeddings,
    Summaries,
}

/// Public destination metadata only; credentials are resolved at execution time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDestination {
    pub key: ModelKey,
    pub provider: ProviderKind,
    pub base_url: String,
    pub organization: String,
    pub project: String,
    pub revision: Option<String>,
}

impl ModelDestination {
    pub fn for_work(
        connection: &ModelConnection,
        model: &ModelInfo,
        work: ModelWork,
    ) -> Option<Self> {
        let eligible = match work {
            ModelWork::Embeddings => {
                matches!(
                    connection.kind,
                    ProviderKind::Ollama
                        | ProviderKind::OpenAiCompatible
                        | ProviderKind::OpenRouter
                ) && (model.output_modalities.iter().any(|m| m == "embeddings")
                    || (connection.kind == ProviderKind::OpenAiCompatible
                        && model.output_modalities.is_empty()))
            }
            ModelWork::Summaries => {
                model.can_chat()
                    && (model.input_modalities.is_empty()
                        || model.input_modalities.iter().any(|m| m == "text"))
            }
        };
        (eligible && !model.id.trim().is_empty()).then(|| Self {
            key: model.key(&connection.id),
            provider: connection.kind,
            base_url: connection.base_url.clone(),
            organization: connection.organization.clone(),
            project: connection.project.clone(),
            revision: model.modified_at.clone(),
        })
    }

    pub fn for_embeddings(connection: &ModelConnection, model: &ModelInfo) -> Option<Self> {
        Self::for_work(connection, model, ModelWork::Embeddings)
    }
}

/// Match execution's catalogue-first resolution before checking capabilities.
/// An incomplete manual entry cannot override known catalogue restrictions.
pub fn model_destinations(
    state: &ModelsState,
    query: &str,
    work: ModelWork,
) -> Vec<ModelDestination> {
    let query = query.trim().to_lowercase();
    let mut targets = BTreeMap::new();
    for connection in &state.connections {
        let models = state
            .catalogs
            .get(&connection.id)
            .into_iter()
            .flat_map(|c| &c.snapshot.models)
            .chain(&connection.manual_models);
        let mut seen = BTreeSet::new();
        for model in models {
            if seen.insert(&model.id)
                && (model.id.to_lowercase().contains(&query)
                    || connection.name.to_lowercase().contains(&query))
                && let Some(target) = ModelDestination::for_work(connection, model, work)
            {
                targets.insert(target.key.clone(), target);
            }
        }
    }
    targets.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn router_embeddings_require_explicit_catalogue_capability() {
        let connection = ModelConnection::new("router", ProviderKind::OpenRouter);
        let mut model = ModelInfo {
            id: "search-helper".into(),
            ..ModelInfo::default()
        };
        assert!(ModelDestination::for_embeddings(&connection, &model).is_none());
        model.output_modalities = vec!["text".into()];
        assert!(ModelDestination::for_embeddings(&connection, &model).is_none());
        model.output_modalities = vec!["embeddings".into()];
        let target = ModelDestination::for_embeddings(&connection, &model).unwrap();
        assert_eq!(target.provider, ProviderKind::OpenRouter);
        assert_eq!(target.base_url, connection.base_url);
        assert!(ModelDestination::for_work(&connection, &model, ModelWork::Summaries).is_none());
    }

    #[test]
    fn summary_destinations_require_text_and_preserve_the_approved_endpoint() {
        let mut connection = ModelConnection::new("chosen", ProviderKind::Anthropic);
        let mut model = ModelInfo {
            id: "text-model".into(),
            input_modalities: vec!["text".into()],
            output_modalities: vec!["text".into()],
            ..ModelInfo::default()
        };
        let original =
            ModelDestination::for_work(&connection, &model, ModelWork::Summaries).unwrap();
        assert!(ModelDestination::for_embeddings(&connection, &model).is_none());
        connection.organization = "new organization".into();
        assert_ne!(
            ModelDestination::for_work(&connection, &model, ModelWork::Summaries),
            Some(original)
        );
        model.output_modalities = vec!["embeddings".into()];
        assert!(ModelDestination::for_work(&connection, &model, ModelWork::Summaries).is_none());
        model.output_modalities = vec!["text".into()];
        model.input_modalities = vec!["image".into()];
        assert!(ModelDestination::for_work(&connection, &model, ModelWork::Summaries).is_none());
    }
}

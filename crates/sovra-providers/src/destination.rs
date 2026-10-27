//! Revalidate approved model destinations before exposing credentials to a workload.

use crate::{ModelRepository, validate_connection};
use sovra_core::models::{
    Authentication, ModelConnection, ModelDestination, ModelError, ModelInfo, ModelSecret,
    ModelWork,
};

#[derive(Debug)]
pub struct ResolvedModel {
    pub connection: ModelConnection,
    pub model: ModelInfo,
    pub secret: ModelSecret,
}

impl ModelRepository {
    pub fn resolve_destination(
        &self,
        target: &ModelDestination,
        work: ModelWork,
    ) -> Result<ResolvedModel, ModelError> {
        let saved = self.load()?;
        let connection = saved
            .connections
            .iter()
            .find(|c| c.id == target.key.connection_id)
            .ok_or_else(|| ModelError("The approved model connection was removed.".into()))?;
        let connection = validate_connection(connection)?;
        let catalog = self.load_catalog(&connection.id)?;
        let model = catalog
            .iter()
            .flat_map(|c| &c.models)
            .chain(&connection.manual_models)
            .find(|m| m.id == target.key.model_id)
            .ok_or_else(|| {
                ModelError(
                    "Refresh this model's catalogue, then choose it again in Knowledge.".into(),
                )
            })?;
        if ModelDestination::for_work(&connection, model, work).as_ref() != Some(target) {
            return Err(ModelError("The approved model or destination changed. Choose it again in Knowledge before sending more text.".into()));
        }
        let secret = self.credential(&connection)?;
        if connection.authentication != Authentication::None && secret.0.is_empty() {
            return Err(ModelError(
                "Add this connection's API key in Models.".into(),
            ));
        }
        Ok(ResolvedModel {
            connection: connection.clone(),
            model: model.clone(),
            secret,
        })
    }
}

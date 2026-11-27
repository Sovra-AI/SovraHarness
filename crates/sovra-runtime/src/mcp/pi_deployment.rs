//! Delivery of a saved coding workspace to the Pi selected in that conversation.
use super::{McpRuntime, ServiceCaller};
use sovra_core::{
    mcp::{RemoteCall, RemoteToolCaller as _},
    raspberry_pi::{PiDeployment, PiProject},
};
use std::sync::Arc;

#[derive(Debug)]
struct SelectedPi {
    board_id: String,
    service_id: String,
    session_id: String,
    caller: Arc<ServiceCaller>,
}

#[async_trait::async_trait]
impl PiDeployment for SelectedPi {
    fn board_id(&self) -> &str {
        &self.board_id
    }
    async fn check(&self) -> Result<(), String> {
        if sovra_core::raspberry_pi::board(&self.board_id).is_none() {
            return Err(
                "This Pi connection is unavailable. Open Connections and connect it again.".into(),
            );
        }
        let result = self
            .caller
            .call(RemoteCall {
                service_id: self.service_id.clone(),
                session_id: self.session_id.clone(),
                tool: "connect_device".into(),
                arguments: serde_json::json!({}),
            })
            .await?;
        if result.text.starts_with("Connected to ") {
            Ok(())
        } else {
            Err(result.text)
        }
    }
    async fn deploy(&self, project: PiProject) -> Result<String, String> {
        let arguments =
            serde_json::to_value(project).map_err(|_| "Couldn't prepare the saved project.")?;
        let result = self
            .caller
            .call(RemoteCall {
                service_id: self.service_id.clone(),
                session_id: self.session_id.clone(),
                tool: "deploy_project".into(),
                arguments,
            })
            .await?;
        serde_json::from_str::<Result<String, String>>(&result.text).map_err(|_| {
            "The Pi plugin did not confirm deployment. Check the device before trying again."
                .to_owned()
        })?
    }
}

impl McpRuntime {
    pub(crate) fn pi_deployment(&self, session: &str) -> Option<Arc<dyn PiDeployment>> {
        let sessions = self
            .board
            .pi_sessions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = sessions.get(session)?;
        let services = self
            .board
            .pi_services
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let board_id = services
            .get(id)
            .map(|setup| setup.board_id.clone())
            .unwrap_or_default();
        Some(Arc::new(SelectedPi {
            board_id,
            service_id: id.clone(),
            session_id: session.into(),
            caller: self.caller(),
        }))
    }
}

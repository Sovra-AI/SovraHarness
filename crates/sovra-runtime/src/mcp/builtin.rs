//! Shared stdio lifecycle for servers shipped in Sovra's executable.

use rmcp::{ServerHandler, ServiceExt as _};

pub(super) fn run(server: impl ServerHandler + 'static) -> Result<(), crate::Error> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(crate::Error::Start)?;
    runtime.block_on(async {
        let service = server
            .serve(rmcp::transport::async_rw::AsyncRwTransport::new_server(
                tokio::io::stdin(),
                tokio::io::stdout(),
            ))
            .await
            .map_err(|error| crate::Error::BuiltinMcp(error.to_string()))?;
        service
            .waiting()
            .await
            .map_err(|error| crate::Error::BuiltinMcp(error.to_string()))?;
        Ok(())
    })
}

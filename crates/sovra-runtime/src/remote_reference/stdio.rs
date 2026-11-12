//! Bounded JSON-lines driver for the reference client. Stdout is protocol data only.
use super::{RemoteReferenceClient, error};
use serde::Deserialize;
use sovra_core::{
    development::UiRequest,
    remote_control::{ErrorCode, MAX_REQUEST_BYTES},
};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Pair {
        link: String,
        name: String,
    },
    Control {
        request: UiRequest,
        revision: Option<String>,
    },
}

/// Stdio is a developer reference, not a second desktop listener. Pairing links
/// arrive only on stdin so they never enter process arguments or shell history.
pub fn run_remote_reference(
    directory: PathBuf,
    transport: &crate::RemoteEndpointFactory,
) -> Result<(), crate::Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(crate::Error::Start)?;
    runtime.block_on(async {
        let endpoint = tokio::time::timeout(super::TIMEOUT, transport())
            .await
            .map_err(|_| crate::Error::RemoteReference("Phone endpoint startup timed out.".into()))?
            .map_err(crate::Error::RemoteReference)?;
        let mut client = RemoteReferenceClient::open(directory, endpoint).await?;
        let result = serve(&mut client).await;
        client.close().await;
        result
    })
}

async fn serve(client: &mut RemoteReferenceClient) -> Result<(), crate::Error> {
    let mut input = tokio::io::BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    loop {
        let mut bytes = Vec::new();
        let count = (&mut input)
            .take((MAX_REQUEST_BYTES + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .await
            .map_err(|_| crate::Error::RemoteReference("Could not read stdin.".into()))?;
        if count == 0 {
            return Ok(());
        }
        if count > MAX_REQUEST_BYTES {
            return Err(crate::Error::RemoteReference(
                "Input line exceeds 32 KiB.".into(),
            ));
        }
        let response = match serde_json::from_slice::<Input>(&bytes) {
            Ok(Input::Pair { link, name }) => client.pair(&link, name).await,
            Ok(Input::Control { request, revision }) => client.control(request, revision).await,
            Err(_) => error(
                ErrorCode::InvalidRequest,
                "Expected a pair or control JSON object.",
            ),
        };
        let mut bytes = serde_json::to_vec(&response)
            .map_err(|_| crate::Error::RemoteReference("Could not encode response.".into()))?;
        bytes.push(b'\n');
        output
            .write_all(&bytes)
            .await
            .map_err(|_| crate::Error::RemoteReference("Could not write stdout.".into()))?;
        output
            .flush()
            .await
            .map_err(|_| crate::Error::RemoteReference("Could not flush stdout.".into()))?;
    }
}

//! Each connection has an owned handshake/read/reply loop, cancelled by its actor.
use super::{IO_TIMEOUT, WorkerMessage, failure};
use async_channel::Sender;
use sovra_core::{
    group::DeviceId,
    remote_control::{ErrorCode, RemoteHandshake},
};
use tokio::sync::oneshot;

pub(super) async fn serve(
    id: u64,
    generation: DeviceId,
    handshake: Box<dyn RemoteHandshake>,
    messages: Sender<WorkerMessage>,
) -> u64 {
    let Ok(Ok(connection)) = tokio::time::timeout(IO_TIMEOUT, handshake.complete()).await else {
        return id;
    };
    if messages
        .send(WorkerMessage::Connected {
            generation,
            id,
            connection: connection.clone(),
        })
        .await
        .is_err()
    {
        connection.close();
        return id;
    }
    loop {
        let Ok(Ok(incoming)) = tokio::time::timeout(IO_TIMEOUT, connection.receive()).await else {
            break;
        };
        let (reply, received) = oneshot::channel();
        if messages
            .try_send(WorkerMessage::Request {
                generation,
                id,
                peer: connection.peer(),
                request: incoming.request,
                reply,
            })
            .is_err()
        {
            break;
        }
        let response = match tokio::time::timeout(IO_TIMEOUT, received).await {
            Ok(Ok(response)) => response,
            _ => failure(
                ErrorCode::OutcomeUnknown,
                "Request timed out or access changed. Inspect before retrying.",
            ),
        };
        if !matches!(
            tokio::time::timeout(IO_TIMEOUT, incoming.reply.send(response)).await,
            Ok(Ok(()))
        ) {
            break;
        }
    }
    connection.close();
    id
}

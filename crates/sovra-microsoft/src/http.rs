//! Bounded transport with sanitized failures and no automatic authenticated redirects.

use futures::StreamExt as _;
use serde::de::DeserializeOwned;
use sovra_core::microsoft::MicrosoftError;

pub(crate) const JSON_LIMIT: usize = 2 * 1024 * 1024;

pub(crate) async fn bytes(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, MicrosoftError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(too_large());
    }
    let mut result = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| offline())?;
        if chunk.len() > limit.saturating_sub(result.len()) {
            return Err(too_large());
        }
        result.extend_from_slice(&chunk);
    }
    Ok(result)
}

pub(crate) async fn json<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, MicrosoftError> {
    status(&response)?;
    serde_json::from_slice(&bytes(response, JSON_LIMIT).await?)
        .map_err(|_| MicrosoftError::Failed("Microsoft sent a reply Sovra couldn't read.".into()))
}

pub(crate) fn status(response: &reqwest::Response) -> Result<(), MicrosoftError> {
    match response.status().as_u16() {
        200..=299 => Ok(()),
        401 => Err(MicrosoftError::SignInNeeded),
        403 => Err(MicrosoftError::NotAllowed),
        404 => Err(MicrosoftError::Failed(
            "This item or Microsoft 365 service isn't available to this account.".into(),
        )),
        429 | 503 => {
            let seconds = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(30)
                .clamp(1, 3600);
            Err(MicrosoftError::Throttled(seconds))
        }
        _ => Err(MicrosoftError::Failed(
            "Microsoft couldn't complete this request. Check the details and try again.".into(),
        )),
    }
}

pub(crate) fn offline() -> MicrosoftError {
    MicrosoftError::Failed("Sovra couldn't reach Microsoft. Check your internet connection.".into())
}

fn too_large() -> MicrosoftError {
    MicrosoftError::Failed("Microsoft's reply is too large. Try a smaller request or file.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_are_bounded_even_without_a_content_length_and_errors_are_sanitized() {
        futures::executor::block_on(async {
            let response = ::http::Response::new(vec![b'x'; 100]);
            assert!(bytes(response.into(), 99).await.is_err());
            let chunks = futures::stream::iter([
                Ok::<_, std::io::Error>(vec![b'x'; 50]),
                Ok(vec![b'x'; 50]),
            ]);
            let response = ::http::Response::new(reqwest::Body::wrap_stream(chunks));
            assert!(bytes(response.into(), 99).await.is_err());
            for (status_code, expected) in [
                (401, MicrosoftError::SignInNeeded),
                (403, MicrosoftError::NotAllowed),
                (429, MicrosoftError::Throttled(12)),
            ] {
                let response = ::http::Response::builder()
                    .status(status_code)
                    .header("Retry-After", "12")
                    .body("private provider reply")
                    .unwrap();
                assert_eq!(
                    json::<serde_json::Value>(response.into())
                        .await
                        .unwrap_err(),
                    expected
                );
            }
        });
    }
}

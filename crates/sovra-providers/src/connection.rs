use sovra_core::models::{Authentication, ModelConnection, ModelError, ModelSecret, ProviderKind};
use std::time::Duration;
use url::Url;

pub fn validate_connection(connection: &ModelConnection) -> Result<ModelConnection, ModelError> {
    let mut connection = connection.clone();
    let trimmed_name = connection.name.trim().to_owned();
    connection.name = trimmed_name;
    if connection.name.is_empty() {
        return Err(ModelError("Give this connection a name.".into()));
    }
    if connection.id.is_empty()
        || !connection
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(ModelError(
            "Connection identifier must contain only letters, digits and hyphens.".into(),
        ));
    }
    let mut url = Url::parse(connection.base_url.trim()).map_err(|_| {
        ModelError("Enter a full server URL, including http:// or https://.".into())
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ModelError("Use an HTTP or HTTPS server URL.".into()));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ModelError("Use a base URL without credentials, query parameters or a fragment. Enter the API key in its own field.".into()));
    }
    let path = url.path().trim_end_matches('/').to_owned();
    let path = match connection.kind {
        ProviderKind::Ollama => path
            .strip_suffix("/api/tags")
            .or_else(|| path.strip_suffix("/api/chat"))
            .or_else(|| path.strip_suffix("/api"))
            .or_else(|| path.strip_suffix("/v1"))
            .unwrap_or(&path),
        ProviderKind::Anthropic => path
            .strip_suffix("/v1/messages")
            .or_else(|| path.strip_suffix("/v1/models"))
            .or_else(|| path.strip_suffix("/v1"))
            .unwrap_or(&path),
        ProviderKind::OpenRouter | ProviderKind::OpenAiCompatible => path
            .strip_suffix("/chat/completions")
            .or_else(|| path.strip_suffix("/models"))
            .unwrap_or(&path),
    }
    .to_owned();
    let path = if path.is_empty()
        && matches!(
            connection.kind,
            ProviderKind::OpenRouter | ProviderKind::OpenAiCompatible
        ) {
        if connection.kind == ProviderKind::OpenRouter {
            "/api/v1".to_owned()
        } else {
            "/v1".to_owned()
        }
    } else {
        path
    };
    url.set_path(&path);
    let normalized_url = url.as_str().trim_end_matches('/').to_owned();
    connection.base_url = normalized_url;
    if !(5..=600).contains(&connection.timeout_seconds) {
        return Err(ModelError(
            "Request timeout must be between 5 and 600 seconds.".into(),
        ));
    }
    if connection.kind == ProviderKind::OpenRouter
        && connection.authentication != Authentication::Bearer
    {
        return Err(ModelError("OpenRouter uses a Bearer API key.".into()));
    }
    if connection.kind == ProviderKind::Anthropic
        && connection.authentication != Authentication::ApiKey
    {
        return Err(ModelError(
            "Anthropic Messages connections use an x-api-key header.".into(),
        ));
    }
    connection.api_key_header = connection.api_key_header.trim().to_ascii_lowercase();
    reqwest::header::HeaderName::from_bytes(connection.api_key_header.as_bytes()).map_err(
        |_| {
            ModelError(
                "Enter a valid API key header name, for example x-api-key or api-key.".into(),
            )
        },
    )?;
    if matches!(
        connection.api_key_header.as_str(),
        "host" | "content-length" | "content-type" | "connection" | "authorization"
    ) {
        return Err(ModelError(
            "Choose a dedicated API key header, or use Bearer authentication for Authorization."
                .into(),
        ));
    }
    if connection.kind == ProviderKind::Anthropic && connection.api_key_header != "x-api-key" {
        return Err(ModelError(
            "Anthropic Messages uses the x-api-key header.".into(),
        ));
    }
    // Never forward a provider credential through control characters in a header.
    for value in [&connection.organization, &connection.project] {
        reqwest::header::HeaderValue::from_str(value).map_err(|_| {
            ModelError("Organization and project IDs must be valid HTTP header values.".into())
        })?;
    }
    Ok(connection)
}

pub fn endpoint(connection: &ModelConnection, suffix: &str) -> String {
    format!(
        "{}/{}",
        connection.base_url.trim_end_matches('/'),
        suffix.trim_start_matches('/')
    )
}

pub(crate) fn client(connection: &ModelConnection) -> Result<reqwest::Client, ModelError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(connection.timeout_seconds))
        // Authentication must never follow a redirect to a different server.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("SovraHarness/0.1")
        .build()
        .map_err(|_| ModelError("Could not initialize the provider HTTP client.".into()))
}

pub(crate) fn authorize(
    mut request: reqwest::RequestBuilder,
    connection: &ModelConnection,
    secret: &ModelSecret,
) -> reqwest::RequestBuilder {
    if !secret.0.is_empty() {
        request = match connection.authentication {
            Authentication::None => request,
            Authentication::Bearer => request.bearer_auth(&secret.0),
            Authentication::ApiKey => request.header(&connection.api_key_header, &secret.0),
        };
    }
    if connection.kind == ProviderKind::Anthropic {
        request = request.header("anthropic-version", "2023-06-01");
    }
    if connection.kind == ProviderKind::OpenRouter {
        request = request.header("X-Title", "SovraHarness");
    }
    if !connection.organization.is_empty() {
        request = request.header("OpenAI-Organization", &connection.organization);
    }
    if !connection.project.is_empty() {
        request = request.header("OpenAI-Project", &connection.project);
    }
    request
}

pub(crate) fn transport_error(error: &reqwest::Error) -> ModelError {
    if error.is_timeout() {
        ModelError(
            "The server timed out. Check the URL, server status or increase the timeout.".into(),
        )
    } else if error.is_connect() {
        ModelError(
            "Could not connect. Check the URL and port, and make sure the model server is running."
                .into(),
        )
    } else {
        ModelError("The server request failed. Check the connection URL, TLS certificate and authentication settings.".into())
    }
}

pub(crate) fn status_error(status: reqwest::StatusCode) -> ModelError {
    let hint = match status.as_u16() {
        401 | 403 => {
            "That key wasn't accepted. Check that you copied the whole key and that your account is active."
        }
        404 => {
            "Endpoint not found. Check the base URL and protocol. You can add a model ID manually if this server does not support discovery."
        }
        429 => "Rate limited. Wait before refreshing again.",
        300..=399 => "The server redirected this request. Use its final API base URL.",
        _ => "The server could not complete the request. Try again shortly.",
    };
    ModelError(format!("HTTP {}: {hint}", status.as_u16()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_protocol_paths_and_rejects_secret_urls() {
        for (kind, raw, expected) in [
            (
                ProviderKind::Ollama,
                "http://localhost:11434/api/tags",
                "http://localhost:11434",
            ),
            (
                ProviderKind::OpenAiCompatible,
                "http://localhost:1234/v1/chat/completions/",
                "http://localhost:1234/v1",
            ),
            (
                ProviderKind::Anthropic,
                "https://api.anthropic.com/v1",
                "https://api.anthropic.com",
            ),
            (
                ProviderKind::OpenRouter,
                "https://openrouter.ai",
                "https://openrouter.ai/api/v1",
            ),
            (
                ProviderKind::OpenAiCompatible,
                "https://proxy.test/team/inference/v1",
                "https://proxy.test/team/inference/v1",
            ),
        ] {
            let mut c = ModelConnection::new("test", kind);
            c.base_url = raw.into();
            assert_eq!(validate_connection(&c).unwrap().base_url, expected);
        }
        let mut c = ModelConnection::new("test", ProviderKind::Ollama);
        for invalid in [
            "file:///tmp/key",
            "http://user:secret@host",
            "https://host?api_key=secret",
            "localhost:11434",
        ] {
            c.base_url = invalid.into();
            assert!(validate_connection(&c).is_err());
        }
    }
}

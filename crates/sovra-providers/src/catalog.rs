use std::collections::BTreeMap;
use std::fmt::Write as _;

use futures::{StreamExt as _, future::BoxFuture};
use serde_json::{Value, json};
use sovra_core::models::{
    ModelConnection, ModelError, ModelInfo, ModelProvider, ModelSecret, ProviderKind,
};

use crate::connection::{authorize, client, endpoint, status_error, transport_error};

const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

pub async fn test_connection(
    provider: &dyn ModelProvider,
    connection: &ModelConnection,
    secret: &ModelSecret,
) -> Result<usize, ModelError> {
    if connection.authentication != sovra_core::models::Authentication::None && secret.0.is_empty()
    {
        return Err(ModelError("Paste your key first, then check it.".into()));
    }
    if connection.kind == ProviderKind::OpenRouter {
        let request = client(connection)?.get(endpoint(connection, "auth/key"));
        read_json(authorize(request, connection, secret)).await?;
    }
    Ok(provider.list(connection, secret).await?.len())
}

/// Rejects a key the service refuses before it is saved.
///
/// `OpenRouter` lists its models without authentication, so a mistyped or deleted
/// key still loads a full catalog and would only fail on the first chat message.
/// Network failures are not a verdict on the key: saving still works offline.
pub async fn verify_key(
    connection: &ModelConnection,
    secret: &ModelSecret,
) -> Result<(), ModelError> {
    if connection.kind != ProviderKind::OpenRouter || secret.0.is_empty() {
        return Ok(());
    }
    let request = client(connection)?.get(endpoint(connection, "auth/key"));
    match authorize(request, connection, secret).send().await {
        Ok(response) if matches!(response.status().as_u16(), 401 | 403) => {
            Err(ModelError(format!(
                "{} didn't accept this key. It may be incomplete or deleted. Copy it again from openrouter.ai/keys — it starts with sk-or-v1-.",
                connection.name
            )))
        }
        Ok(_) | Err(_) => Ok(()),
    }
}

async fn read_json(request: reqwest::RequestBuilder) -> Result<Value, ModelError> {
    let response = request.send().await.map_err(|e| transport_error(&e))?;
    if !response.status().is_success() {
        return Err(status_error(response.status()));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| transport_error(&e))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ModelError(
                "The model catalog exceeds the 32 MB response limit.".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        ModelError(
            "The server did not return a valid JSON model catalog. Check the API base URL.".into(),
        )
    })
}

fn text(value: &Value, field: &str) -> Option<String> {
    value.get(field)?.as_str().map(str::to_owned)
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn required_id(value: &Value) -> Result<String, ModelError> {
    text(value, "id")
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            ModelError(
                "The server returned a model without an ID. The previous catalog has been kept."
                    .into(),
            )
        })
}

fn data_array(value: &Value) -> Result<&[Value], ModelError> {
    value["data"].as_array().map(Vec::as_slice).ok_or_else(|| {
        ModelError("Expected a catalog with a data array. Check the selected API protocol.".into())
    })
}

fn parse_router(value: &Value) -> Result<Vec<ModelInfo>, ModelError> {
    data_array(value)?
        .iter()
        .map(|item| {
            let id = required_id(item)?;
            let pricing = item["pricing"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(key, value)| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| value.as_number().map(ToString::to_string))
                        .map(|v| (key.clone(), v))
                })
                .collect();
            Ok(ModelInfo {
                author: id.split('/').next().unwrap_or_default().to_owned(),
                name: text(item, "name").unwrap_or_else(|| id.clone()),
                id,
                description: text(item, "description").unwrap_or_default(),
                context_length: item["context_length"].as_u64(),
                max_output_tokens: item["top_provider"]["max_completion_tokens"].as_u64(),
                input_modalities: strings(&item["architecture"]["input_modalities"]),
                output_modalities: strings(&item["architecture"]["output_modalities"]),
                supported_parameters: strings(&item["supported_parameters"]),
                pricing,
                created: item["created"].as_u64(),
                knowledge_cutoff: text(item, "knowledge_cutoff"),
                expiration_date: text(item, "expiration_date"),
                tokenizer: text(&item["architecture"], "tokenizer"),
                details_loaded: true,
                ..ModelInfo::default()
            })
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct OpenRouterProvider;

#[async_trait::async_trait]
impl ModelProvider for OpenRouterProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenRouter
    }

    async fn list(
        &self,
        connection: &ModelConnection,
        secret: &ModelSecret,
    ) -> Result<Vec<ModelInfo>, ModelError> {
        // The public endpoint defaults to text. `all` is essential: no model type
        // is silently dropped, and no account preference narrows the catalog.
        let request = client(connection)?
            .get(endpoint(connection, "models"))
            .query(&[("output_modalities", "all")]);
        parse_router(&read_json(authorize(request, connection, secret)).await?)
    }

    async fn details(
        &self,
        _: &ModelConnection,
        _: &ModelSecret,
        model: &ModelInfo,
    ) -> Result<ModelInfo, ModelError> {
        Ok(model.clone())
    }
}

#[derive(Debug, Default)]
pub struct OllamaProvider;

fn parse_ollama(value: &Value) -> Result<Vec<ModelInfo>, ModelError> {
    let items = value["models"].as_array().ok_or_else(|| {
        ModelError(
            "Expected an Ollama models array. Use the server URL, usually http://localhost:11434."
                .into(),
        )
    })?;
    items.iter().map(|item| {
        let id = text(item, "model").or_else(|| text(item, "name")).filter(|id| !id.is_empty()).ok_or_else(|| ModelError("Ollama returned a model without a name.".into()))?;
        Ok(ModelInfo {
            name: id.clone(), id, author: text(&item["details"], "family").unwrap_or_else(|| "Ollama".into()),
            description: "Available on this Ollama server. Open details to inspect context and capabilities.".into(),
            family: text(&item["details"], "family"), parameter_size: text(&item["details"], "parameter_size"),
            quantization: text(&item["details"], "quantization_level"), size_bytes: item["size"].as_u64(),
            modified_at: text(item, "modified_at"), ..ModelInfo::default()
        })
    }).collect()
}

fn enrich_ollama(model: &ModelInfo, value: &Value) -> ModelInfo {
    let mut model = model.clone();
    let capabilities = strings(&value["capabilities"]);
    model.supported_parameters.clear();
    model.input_modalities = vec!["text".into()];
    model.output_modalities.clear();
    for capability in capabilities {
        match capability.as_str() {
            "completion" => model.output_modalities.push("text".into()),
            "embedding" => model.output_modalities.push("embeddings".into()),
            "vision" => model.input_modalities.push("image".into()),
            "thinking" => model.supported_parameters.push("reasoning".into()),
            "tools" => model.supported_parameters.push("tools".into()),
            _ => model.supported_parameters.push(capability),
        }
    }
    model.context_length = value["model_info"].as_object().and_then(|info| {
        let architecture = info
            .get("general.architecture")
            .and_then(Value::as_str)
            .or_else(|| value["details"]["family"].as_str())
            .or(model.family.as_deref());
        if let Some(context) = architecture
            .and_then(|name| info.get(&format!("{name}.context_length")))
            .and_then(Value::as_u64)
        {
            return Some(context);
        }
        // Older servers omit architecture. Accept an unambiguous language context,
        // never the smaller context of an auxiliary vision/audio encoder.
        let mut contexts = info
            .iter()
            .filter(|(key, _)| {
                key.ends_with(".context_length")
                    && !["clip.", "vision.", "audio."]
                        .iter()
                        .any(|prefix| key.starts_with(prefix))
            })
            .filter_map(|(_, value)| value.as_u64());
        let first = contexts.next();
        if contexts.next().is_none() {
            first
        } else {
            None
        }
    });
    model.family = text(&value["details"], "family").or(model.family);
    model.parameter_size = text(&value["details"], "parameter_size").or(model.parameter_size);
    model.quantization = text(&value["details"], "quantization_level").or(model.quantization);
    model.description = format!(
        "{} · {} parameters · {} quantization. Context is the model's advertised maximum; the server's configured runtime context may be lower.",
        model.family.as_deref().unwrap_or("Ollama model"),
        model.parameter_size.as_deref().unwrap_or("Unknown"),
        model.quantization.as_deref().unwrap_or("Unknown")
    );
    model.details_loaded = true;
    model
}

#[async_trait::async_trait]
impl ModelProvider for OllamaProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }
    async fn list(
        &self,
        connection: &ModelConnection,
        secret: &ModelSecret,
    ) -> Result<Vec<ModelInfo>, ModelError> {
        let request = client(connection)?.get(endpoint(connection, "api/tags"));
        let models = parse_ollama(&read_json(authorize(request, connection, secret)).await?)?;
        // Bound discovery concurrency; missing metadata must not hide an installed model.
        Ok(futures::stream::iter(models)
            .map(|model| async move {
                self.details(connection, secret, &model)
                    .await
                    .unwrap_or(model)
            })
            .buffer_unordered(4)
            .collect()
            .await)
    }
    async fn details(
        &self,
        connection: &ModelConnection,
        secret: &ModelSecret,
        model: &ModelInfo,
    ) -> Result<ModelInfo, ModelError> {
        let request = client(connection)?
            .post(endpoint(connection, "api/show"))
            .json(&json!({"model":model.id,"verbose":false}));
        Ok(enrich_ollama(
            model,
            &read_json(authorize(request, connection, secret)).await?,
        ))
    }
}

#[derive(Debug, Default)]
pub struct CompatibleProvider;

fn parse_compatible(value: &Value) -> Result<Vec<ModelInfo>, ModelError> {
    data_array(value)?.iter().map(|item| {
        let id = required_id(item)?;
        Ok(ModelInfo {
            name: text(item, "name").or_else(|| text(item, "display_name")).unwrap_or_else(|| id.clone()), id,
            author: text(item, "owned_by").unwrap_or_else(|| "Custom".into()),
            description: "Discovered from this server. Pricing and capabilities are only shown when advertised; verify server configuration before use.".into(),
            created: item["created"].as_u64(), context_length: item["context_length"].as_u64().or_else(|| item["max_model_len"].as_u64()),
            input_modalities: strings(&item["input_modalities"]), output_modalities: strings(&item["output_modalities"]),
            supported_parameters: strings(&item["supported_parameters"]), details_loaded: true, ..ModelInfo::default()
        })
    }).collect()
}

#[async_trait::async_trait]
impl ModelProvider for CompatibleProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAiCompatible
    }
    async fn list(
        &self,
        connection: &ModelConnection,
        secret: &ModelSecret,
    ) -> Result<Vec<ModelInfo>, ModelError> {
        let request = client(connection)?.get(endpoint(connection, "models"));
        parse_compatible(&read_json(authorize(request, connection, secret)).await?)
    }
    async fn details(
        &self,
        _: &ModelConnection,
        _: &ModelSecret,
        model: &ModelInfo,
    ) -> Result<ModelInfo, ModelError> {
        Ok(model.clone())
    }
}

fn parse_anthropic(value: &Value) -> Result<Vec<ModelInfo>, ModelError> {
    let mut models = parse_compatible(value)?;
    for (model, item) in models.iter_mut().zip(data_array(value)?) {
        model.author = "Anthropic".into();
        model.input_modalities = vec!["text".into()];
        model.output_modalities = vec!["text".into()];
        model.context_length = item["max_input_tokens"].as_u64().or(model.context_length);
        model.max_output_tokens = item["max_tokens"].as_u64();
        model.family = text(item, "family");
        model.expiration_date = text(item, "retires_at");
        let supported =
            |name: &str| item["capabilities"][name]["supported"].as_bool() == Some(true);
        if supported("image_input") {
            model.input_modalities.push("image".into());
        }
        if supported("pdf_input") {
            model.input_modalities.push("file".into());
        }
        for (capability, parameter) in [
            ("thinking", "reasoning"),
            ("structured_outputs", "structured_outputs"),
        ] {
            if supported(capability) {
                model.supported_parameters.push(parameter.into());
            }
        }
        if let Some(created) = text(item, "created_at") {
            let _ = write!(model.description, " Created: {created}.");
        }
        if let Some(lifecycle) = text(item, "lifecycle") {
            let _ = write!(model.description, " Lifecycle: {lifecycle}.");
        }
    }
    Ok(models)
}

#[derive(Debug, Default)]
pub struct AnthropicProvider;

#[async_trait::async_trait]
impl ModelProvider for AnthropicProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }
    async fn list(
        &self,
        connection: &ModelConnection,
        secret: &ModelSecret,
    ) -> Result<Vec<ModelInfo>, ModelError> {
        let http = client(connection)?;
        let mut models = BTreeMap::new();
        let mut cursor = None;
        let mut cursors = std::collections::BTreeSet::new();
        loop {
            let mut request = http
                .get(endpoint(connection, "v1/models"))
                .query(&[("limit", "100")]);
            if let Some(cursor) = &cursor {
                request = request.query(&[("after_id", cursor)]);
            }
            let value = read_json(authorize(request, connection, secret)).await?;
            for model in parse_anthropic(&value)? {
                models.insert(model.id.clone(), model);
            }
            if value["has_more"] != true {
                break;
            }
            let next = text(&value, "last_id").ok_or_else(|| {
                ModelError("The server returned incomplete pagination metadata.".into())
            })?;
            if !cursors.insert(next.clone()) || cursors.len() > 100 {
                return Err(ModelError(
                    "The server repeated a catalog page; refresh was stopped.".into(),
                ));
            }
            cursor = Some(next);
        }
        Ok(models.into_values().collect())
    }
    async fn details(
        &self,
        _: &ModelConnection,
        _: &ModelSecret,
        model: &ModelInfo,
    ) -> Result<ModelInfo, ModelError> {
        Ok(model.clone())
    }
}

/// Pull is explicit user work. Dropping this future closes its HTTP stream; runtime
/// owns cancellation. Ollama may retain partial layers for a later resumed download.
pub async fn pull_ollama(
    connection: &ModelConnection,
    secret: &ModelSecret,
    model: &str,
    progress: impl Fn(String) -> BoxFuture<'static, ()> + Send + Sync,
) -> Result<(), ModelError> {
    if model.trim().is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
        return Err(ModelError(
            "Enter an Ollama model name, for example qwen3:8b.".into(),
        ));
    }
    let http = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(8))
        .read_timeout(std::time::Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ModelError("Could not initialize Ollama download.".into()))?;
    let request = http
        .post(endpoint(connection, "api/pull"))
        .json(&json!({"model":model.trim(),"stream":true}));
    let response = authorize(request, connection, secret)
        .send()
        .await
        .map_err(|e| transport_error(&e))?;
    if !response.status().is_success() {
        return Err(status_error(response.status()));
    }
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut success = false;
    let mut previous_message = String::new();
    loop {
        match stream.next().await {
            Some(chunk) => buffer.extend_from_slice(&chunk.map_err(|e| transport_error(&e))?),
            None if buffer.is_empty() => break,
            None => buffer.push(b'\n'),
        }
        if buffer.len() > 1024 * 1024 {
            return Err(ModelError("Invalid Ollama progress response.".into()));
        }
        while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = buffer.drain(..=end).collect();
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let value: Value = serde_json::from_slice(&line)
                .map_err(|_| ModelError("Invalid Ollama progress response.".into()))?;
            if value.get("error").is_some() {
                return Err(ModelError("Ollama could not download this model. Check its exact name, server storage and Ollama authentication.".into()));
            }
            let status = value["status"].as_str().unwrap_or("Downloading");
            success |= status == "success";
            let message = match (value["completed"].as_u64(), value["total"].as_u64()) {
                (Some(done), Some(total)) if total > 0 => {
                    format!("{status} · {}%", done.saturating_mul(100) / total)
                }
                _ => status.to_owned(),
            };
            if message != previous_message {
                previous_message.clone_from(&message);
                progress(message).await;
            }
        }
    }
    if success {
        Ok(())
    } else {
        Err(ModelError("Ollama ended the download before confirming success. Refresh to inspect installed models.".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_every_modality_and_unknown_fields_do_not_break_catalogs() {
        let value = json!({"data":[
            {"id":"a/text","name":"Text","context_length":128_000,"pricing":{"prompt":"0","completion":"0"},"architecture":{"input_modalities":["text","image"],"output_modalities":["text"]},"supported_parameters":["tools"],"future_field":true},
            {"id":"a/video","architecture":{"output_modalities":["video"]},"pricing":{"video":"0.2"}},
            {"id":"b/embed","architecture":{"output_modalities":["embeddings"]},"context_length":null}
        ]});
        let models = parse_router(&value).unwrap();
        assert_eq!(models.len(), 3);
        assert!(models[0].is_free());
        assert!(models[0].supports("tools"));
        assert_eq!(models[1].output_modalities, ["video"]);
        assert_eq!(models[2].context_length, None);
        assert!(parse_router(&json!({"data":[{"name":"missing ID"}]})).is_err());
    }
    #[test]
    fn ollama_tags_and_details_preserve_quantization_and_capabilities() {
        let models = parse_ollama(&json!({"models":[{"model":"qwen:8b","size":5000,"details":{"family":"qwen","parameter_size":"8B","quantization_level":"Q4_K_M"}}]})).unwrap();
        assert_eq!(models[0].size_bytes, Some(5000));
        assert!(!models[0].details_loaded);
        let model = enrich_ollama(
            &models[0],
            &json!({"capabilities":["completion","vision","tools","thinking"],"model_info":{"qwen.context_length":32768}}),
        );
        assert_eq!(model.context_length, Some(32768));
        assert!(model.can_chat());
        assert!(model.supports("tools"));
        assert!(model.supports("reasoning"));
        assert_eq!(model.input_modalities, ["text", "image"]);
        assert_eq!(model.quantization.as_deref(), Some("Q4_K_M"));
    }
    #[test]
    fn compatible_discovery_does_not_invent_prices_or_capabilities() {
        let models =
            parse_compatible(&json!({"data":[{"id":"local-model","owned_by":"local"}]})).unwrap();
        assert_eq!(models[0].id, "local-model");
        assert_eq!(models[0].token_price("prompt"), None);
        assert_eq!(models[0].supported_parameters, Vec::<String>::new());
    }

    #[test]
    fn anthropic_discovery_uses_advertised_capabilities_and_limits() {
        let models = parse_anthropic(&json!({"data":[
            {"id":"legacy", "display_name":"Legacy"},
            {"id":"new", "max_input_tokens":200_000, "max_tokens":64_000, "capabilities":{
                "image_input":{"supported":true}, "pdf_input":{"supported":false},
                "thinking":{"supported":true}, "structured_outputs":{"supported":true}
            }}
        ]}))
        .unwrap();
        assert_eq!(models[0].input_modalities, ["text"]);
        assert_eq!(models[0].supported_parameters, Vec::<String>::new());
        assert_eq!(models[1].context_length, Some(200_000));
        assert_eq!(models[1].max_output_tokens, Some(64_000));
        assert_eq!(models[1].input_modalities, ["text", "image"]);
        assert!(models[1].supports("reasoning"));
        assert!(!models[1].supports("tools"));
    }

    #[test]
    fn ollama_context_uses_language_architecture_not_vision_encoder() {
        let model = enrich_ollama(
            &ModelInfo::default(),
            &json!({
                "model_info": {"general.architecture":"qwen3", "clip.context_length":77, "qwen3.context_length":131_072}
            }),
        );
        assert_eq!(model.context_length, Some(131_072));
        let model = enrich_ollama(
            &ModelInfo::default(),
            &json!({
                "model_info": {"clip.context_length":77}
            }),
        );
        assert_eq!(model.context_length, None);
    }
}

//! Named setup recipes share protocol adapters; addresses remain editable.
//! Sources and server-specific setup: docs/local-models.md.
use super::{Authentication, ProviderKind, TokenLimitField};

#[derive(Debug, Clone, Copy)]
pub struct ConnectionPreset {
    pub name: &'static str,
    pub kind: ProviderKind,
    pub url: &'static str,
    pub authentication: Authentication,
    pub streaming: bool,
    pub token_limit_field: TokenLimitField,
}

pub const CONNECTION_PRESETS: &[ConnectionPreset] = &[
    ConnectionPreset {
        name: "OpenRouter",
        kind: ProviderKind::OpenRouter,
        url: "https://openrouter.ai/api/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxCompletionTokens,
        authentication: Authentication::Bearer,
    },
    ConnectionPreset {
        name: "Ollama",
        kind: ProviderKind::Ollama,
        url: "http://localhost:11434",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "LM Studio",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:1234/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "vLLM",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:8000/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "llama.cpp",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:9931/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "Anthropic",
        kind: ProviderKind::Anthropic,
        url: "https://api.anthropic.com",
        streaming: true,
        token_limit_field: TokenLimitField::MaxCompletionTokens,
        authentication: Authentication::ApiKey,
    },
    ConnectionPreset {
        name: "llama-cpp-python",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:8000/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "LocalAI",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:8080/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "Jan",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://127.0.0.1:1337/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "GPT4All",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://127.0.0.1:4891/v1",
        streaming: false,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "SGLang",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:30000/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "text-generation-webui",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:5000/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "KoboldCpp",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:5001/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "MLX LM",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:8080/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "MLC LLM",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:8000/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "Xinference",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:9997/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
    ConnectionPreset {
        name: "Docker Model Runner",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:12434/engines/v1",
        authentication: Authentication::None,
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
    },
    ConnectionPreset {
        name: "Lemonade",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:13305/v1",
        authentication: Authentication::None,
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
    },
    ConnectionPreset {
        name: "Hugging Face TGI",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:3000/v1",
        authentication: Authentication::None,
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
    },
    ConnectionPreset {
        name: "Other OpenAI-compatible server",
        kind: ProviderKind::OpenAiCompatible,
        url: "http://localhost:8000/v1",
        streaming: true,
        token_limit_field: TokenLimitField::MaxTokens,
        authentication: Authentication::None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn older_connections_keep_streaming_and_complete_reply_choice_persists() {
        let mut connection =
            super::super::ModelConnection::new("test", ProviderKind::OpenAiCompatible);
        let mut saved = serde_json::to_value(&connection).unwrap();
        saved.as_object_mut().unwrap().remove("streaming");
        assert!(
            serde_json::from_value::<super::super::ModelConnection>(saved)
                .unwrap()
                .streaming
        );
        connection.streaming = false;
        assert!(
            !serde_json::from_str::<super::super::ModelConnection>(
                &serde_json::to_string(&connection).unwrap()
            )
            .unwrap()
            .streaming
        );
    }
    #[test]
    fn recipes_are_unique_and_local_servers_allow_model_loading_time() {
        let names: std::collections::BTreeSet<_> = CONNECTION_PRESETS
            .iter()
            .map(|preset| preset.name)
            .collect();
        assert_eq!(names.len(), CONNECTION_PRESETS.len());
        for preset in CONNECTION_PRESETS {
            if matches!(
                preset.kind,
                ProviderKind::Ollama | ProviderKind::OpenAiCompatible
            ) {
                assert_eq!(preset.kind.default_timeout_seconds(), 600);
                assert_eq!(preset.authentication, Authentication::None);
            }
        }
        assert_eq!(ProviderKind::OpenRouter.default_timeout_seconds(), 60);
    }
}

//! The `save_artifact` tool: lets any agent (Sovra or a team member) keep what it
//! made in the user's Artifacts library, and shows a card in the chat when it does.

use std::pin::Pin;
use std::sync::Arc;

use adk_core::{Part, Tool, ToolContext};
use serde_json::{Value, json};
use sovra_core::AgUiWidget;
use sovra_core::artifacts::{Artifact, ArtifactDraft, ArtifactKind, ArtifactSource, KINDS};

pub(crate) const SAVE_ARTIFACT_TOOL_NAME: &str = "save_artifact";

const SAVE_ARTIFACT_DESCRIPTION: &str = "\
Save something you made to the user's Artifacts library so they can find, open, copy or \
download it later: a document, letter, checklist, table, code, Mermaid diagram, HTML page, \
SVG picture or data. Use it when the user asks you to save, keep or make a file, or when you \
produce a substantial document, plan or table they will want to keep. Pass the complete \
content, not a summary. Also show the content in your reply.";

type SaveFuture = Pin<Box<dyn Future<Output = Result<Artifact, String>> + Send>>;
type SaveFn = Arc<dyn Fn(ArtifactDraft) -> SaveFuture + Send + Sync>;

/// Injectable artifact persistence, supplied by the runtime that owns the store.
#[derive(Clone)]
pub struct ArtifactSaver {
    save: SaveFn,
}

impl std::fmt::Debug for ArtifactSaver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArtifactSaver")
            .finish_non_exhaustive()
    }
}

impl ArtifactSaver {
    pub fn from_fn<F, Fut>(save: F) -> Self
    where
        F: Fn(ArtifactDraft) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Artifact, String>> + Send + 'static,
    {
        Self {
            save: Arc::new(move |draft| Box::pin(save(draft))),
        }
    }

    /// Persists a draft through the runtime-owned saver.
    pub async fn save(&self, draft: ArtifactDraft) -> Result<Artifact, String> {
        (self.save)(draft).await
    }
}

impl crate::AdkHarness {
    /// Stores the saver for direct saves (deep research) and adds `save_artifact`
    /// when this model already has other tools it can call.
    pub fn with_artifact_saver(&self, saver: ArtifactSaver) -> Result<Self, crate::Error> {
        if self.base_tools.is_empty() {
            let mut harness = self.clone();
            harness.artifact_saver = Some(saver);
            return Ok(harness);
        }
        let mut tools = self.base_tools.clone();
        tools.retain(|tool| tool.name() != SAVE_ARTIFACT_TOOL_NAME);
        tools.push(save_artifact_tool(saver.clone(), "Sovra"));
        let mut harness = self.rebuild(self.base_instruction.clone(), tools)?;
        harness.artifact_saver = Some(saver);
        Ok(harness)
    }
}

pub(crate) fn save_artifact_tool(saver: ArtifactSaver, author: &str) -> Arc<dyn Tool> {
    Arc::new(SaveArtifactTool {
        saver,
        author: author.to_owned(),
    })
}

struct SaveArtifactTool {
    saver: ArtifactSaver,
    author: String,
}

fn kind_key(kind: ArtifactKind) -> Value {
    serde_json::to_value(kind).unwrap_or(Value::Null)
}

#[async_trait::async_trait]
impl Tool for SaveArtifactTool {
    fn name(&self) -> &str {
        SAVE_ARTIFACT_TOOL_NAME
    }

    fn description(&self) -> &str {
        SAVE_ARTIFACT_DESCRIPTION
    }

    fn parameters_schema(&self) -> Option<Value> {
        let kinds: Vec<Value> = KINDS.iter().map(|info| kind_key(info.kind)).collect();
        Some(json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "A short, clear name, e.g. \"Weekly meal plan\"." },
                "kind": { "type": "string", "enum": kinds, "description": "What sort of thing it is." },
                "language": { "type": "string", "description": "For code or data: the language, e.g. python, mermaid, json. Otherwise empty." },
                "content": { "type": "string", "description": "The complete content: Markdown for documents and checklists, a Markdown table for tables, raw source otherwise." }
            },
            "required": ["title", "kind", "content"]
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let content = text("content");
        if content.trim().is_empty() {
            return Err(adk_core::AdkError::tool("content is required"));
        }
        let language = text("language");
        let kind = serde_json::from_value(args.get("kind").cloned().unwrap_or(Value::Null))
            .unwrap_or_else(|_| ArtifactKind::for_language(&language));
        let draft = ArtifactDraft {
            title: text("title"),
            kind,
            language,
            content,
            source: ArtifactSource {
                session_id: ctx.session_id().to_owned(),
                message_id: String::new(),
                author: self.author.clone(),
                shared_from: String::new(),
            },
        };
        match (self.saver.save)(draft).await {
            Ok(artifact) => Ok(json!({
                "saved": true,
                "artifact_id": artifact.id,
                "title": artifact.title,
                "kind": kind_key(artifact.kind),
            })),
            Err(message) => Ok(json!({ "saved": false, "error": message })),
        }
    }
}

/// Cards for artifacts saved by `save_artifact` calls in this event.
pub(crate) fn saved_artifact_widgets(event: &adk_core::Event) -> Vec<AgUiWidget> {
    let Some(content) = &event.llm_response.content else {
        return Vec::new();
    };
    content
        .parts
        .iter()
        .filter_map(|part| {
            let Part::FunctionResponse {
                function_response, ..
            } = part
            else {
                return None;
            };
            if function_response.name != SAVE_ARTIFACT_TOOL_NAME {
                return None;
            }
            let response = &function_response.response;
            if response.get("saved").and_then(Value::as_bool) != Some(true) {
                return None;
            }
            Some(AgUiWidget::ArtifactSaved {
                artifact_id: response.get("artifact_id")?.as_str()?.to_owned(),
                title: response.get("title")?.as_str()?.to_owned(),
                artifact_kind: serde_json::from_value(response.get("kind")?.clone()).ok()?,
            })
        })
        .collect()
}

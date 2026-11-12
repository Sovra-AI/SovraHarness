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
produce substantial code, a website, document, plan or table they will want to keep. Save simple self-contained websites as kind webpage, language html. For React, Vue, Svelte or other frontend frameworks use kind project, language website-project, and files (relative file paths mapped to complete source strings) and output_dir (dist by default). Include package.json with dependencies and a build script producing static index.html, plus every source/config file. Do not save a separate artifact per project file. Framework builds use installed Node.js/npm and command approvals in chat; server-side rendering/backends are not supported by local preview. Pass the complete \
content, not a summary. Also show the content in your reply.";

type SaveFuture = Pin<Box<dyn Future<Output = Result<Artifact, String>> + Send>>;
type SaveFn = Arc<dyn Fn(ArtifactDraft) -> SaveFuture + Send + Sync>;
type DeployFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
type DeployFn = Arc<dyn Fn(String, String) -> DeployFuture + Send + Sync>;

/// Injectable artifact persistence, supplied by the runtime that owns the store.
#[derive(Clone)]
pub struct ArtifactSaver {
    save: SaveFn,
    deploy: Option<DeployFn>,
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
            deploy: None,
        }
    }

    /// Attach runtime-owned local hosting. Arguments are artifact id and chat id.
    #[must_use]
    pub fn with_deployer<F, Fut>(mut self, deploy: F) -> Self
    where
        F: Fn(String, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, String>> + Send + 'static,
    {
        self.deploy = Some(Arc::new(move |id, session| Box::pin(deploy(id, session))));
        self
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
        tools.retain(|tool| !matches!(tool.name(), SAVE_ARTIFACT_TOOL_NAME | "deploy_artifact"));
        if let Some(deploy) = &saver.deploy {
            tools.push(Arc::new(DeployArtifactTool {
                deploy: deploy.clone(),
            }));
        }
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
                "files": { "type": "object", "additionalProperties": {"type":"string"}, "description": "For kind project: every relative file path mapped to its complete source, including package.json and the build script." },
                "output_dir": { "type": "string", "description": "For project builds: static output folder, usually dist, build or out." },
                "content": { "type": "string", "description": "The complete content: Markdown for documents and checklists, a Markdown table for tables, raw source otherwise." }
            },
            "required": ["title", "kind"]
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let mut language = text("language");
        let kind = serde_json::from_value(args.get("kind").cloned().unwrap_or(Value::Null))
            .unwrap_or_else(|_| ArtifactKind::for_language(&language));
        let content = if kind == ArtifactKind::Project && args.get("files").is_some() {
            language = "website-project".into();
            let mut project = json!({"files": args["files"]});
            if let Some(output) = args.get("output_dir") {
                project["output_dir"] = output.clone();
            }
            project.to_string()
        } else {
            text("content")
        };
        if content.trim().is_empty() {
            return Err(adk_core::AdkError::tool(
                "content or project files are required",
            ));
        }
        if kind == ArtifactKind::Project {
            sovra_core::artifacts::WebsiteProject::parse(&content)
                .map_err(adk_core::AdkError::tool)?;
        }
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

struct DeployArtifactTool {
    deploy: DeployFn,
}

#[async_trait::async_trait]
impl Tool for DeployArtifactTool {
    fn name(&self) -> &'static str {
        "deploy_artifact"
    }

    fn description(&self) -> &'static str {
        "Build and run a saved website project or HTML web page as a website on this computer and open it in the browser. Use only when the user asks to deploy, run, launch or preview their website. Save the complete page first with save_artifact. This is local hosting, not public internet publishing. Pass artifact_id from the save result, or omit it to run the latest saved web page from this chat. Check the result before claiming the website is running."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({"type": "object", "properties": {
            "artifact_id": {"type": "string", "description": "Saved artifact id; omit for the latest web page in this chat."}
        }}))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let id = args
            .get("artifact_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match (self.deploy)(id, ctx.session_id().to_owned()).await {
            Ok(url) => Ok(json!({"running": true, "url": url, "visibility": "this computer only"})),
            Err(message) => Ok(json!({"running": false, "error": message})),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{Content, Llm, LlmRequest, LlmResponse, LlmResponseStream};
    use std::sync::Mutex;

    struct WebsiteModel;
    #[async_trait::async_trait]
    impl Llm for WebsiteModel {
        fn name(&self) -> &'static str {
            "website-test"
        }
        async fn generate_content(
            &self,
            request: LlmRequest,
            _: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let response = |name: &str| {
                request
                    .contents
                    .iter()
                    .flat_map(|content| &content.parts)
                    .find_map(|part| {
                        if let Part::FunctionResponse {
                            function_response, ..
                        } = part
                            && function_response.name == name
                        {
                            Some(function_response.response.clone())
                        } else {
                            None
                        }
                    })
            };
            let part = if let Some(result) = response("deploy_artifact") {
                assert_eq!(result["running"], true);
                Part::Text {
                    text: "Your website is running on this computer.".into(),
                }
            } else if let Some(result) = response("save_artifact") {
                assert_eq!(result["saved"], true);
                Part::FunctionCall {
                    name: "deploy_artifact".into(),
                    args: json!({"artifact_id": result["artifact_id"]}),
                    id: Some("deploy".into()),
                    thought_signature: None,
                }
            } else {
                assert!(request.tools.contains_key("save_artifact"));
                assert!(request.tools.contains_key("deploy_artifact"));
                Part::FunctionCall {
                    name: "save_artifact".into(),
                    args: json!({"title":"Garden", "kind":"webpage", "language":"html", "content":"<h1>Garden</h1>"}),
                    id: Some("save".into()),
                    thought_signature: None,
                }
            };
            Ok(Box::pin(futures::stream::once(async move {
                Ok(LlmResponse::new(Content {
                    role: "model".into(),
                    parts: vec![part],
                }))
            })))
        }
    }

    #[tokio::test]
    async fn chat_saves_source_before_deploying_the_saved_id() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let saves = calls.clone();
        let deployments = calls.clone();
        let saver = ArtifactSaver::from_fn(move |draft| {
            saves.lock().unwrap().push("save");
            assert_eq!(draft.source.session_id, "website-chat");
            async move {
                Ok(Artifact::from_draft(
                    "website".into(),
                    draft,
                    sovra_core::Timestamp(1),
                ))
            }
        })
        .with_deployer(move |id, session| {
            deployments.lock().unwrap().push("deploy");
            assert_eq!(id, "website");
            assert_eq!(session, "website-chat");
            async { Ok("http://127.0.0.1:1234/".into()) }
        });
        let harness = crate::AdkHarness::with_llm_and_tools(
            &sovra_core::AgentConfig::default(),
            Arc::new(WebsiteModel),
            crate::native_ui::tools(),
        )
        .unwrap()
        .with_artifact_saver(saver)
        .unwrap();
        let (tx, rx) = async_channel::unbounded();
        harness
            .run_prompt(
                "website-chat",
                "Make a garden website and run it on this computer.",
                &tx,
            )
            .await;
        assert_eq!(*calls.lock().unwrap(), vec!["save", "deploy"]);
        assert!(
            std::iter::from_fn(|| rx.try_recv().ok())
                .any(|event| matches!(event, sovra_core::Event::AgentCompleted { .. }))
        );
    }
}

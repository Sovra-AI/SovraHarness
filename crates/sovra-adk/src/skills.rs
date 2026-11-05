//! On-demand skill instructions exposed through the actual ADK tool boundary.

use std::sync::Arc;

use adk_core::{Tool, ToolContext};
use serde_json::{Value, json};
use sovra_core::skills::{Skill, SkillCatalog};

pub(super) struct LoadSkill {
    catalog: Arc<SkillCatalog>,
    available: Vec<String>,
    pub(super) resources: Option<Arc<dyn sovra_core::skills::SkillResources>>,
}

impl LoadSkill {
    pub(super) fn new(catalog: Arc<SkillCatalog>, available: Vec<String>) -> Self {
        Self {
            catalog,
            available,
            resources: None,
        }
    }

    fn supports(&self, skill: &Skill) -> bool {
        skill.runnable_with(&self.available)
    }

    fn response(&self, args: &Value) -> Value {
        if !args.is_object() {
            return json!({"error":"Skill arguments must be an object."});
        }
        match args.get("name") {
            None => {
                let query = match args.get("query") {
                    None => "",
                    Some(Value::String(query)) if query.len() <= 256 => query,
                    Some(_) => {
                        return json!({"error":"query must be a string of at most 256 bytes."});
                    }
                };
                let offset = match offset(args) {
                    Ok(offset) => offset,
                    Err(error) => return error,
                };
                let matches: Vec<_> = self
                    .catalog
                    .search(query)
                    .into_iter()
                    .filter(|skill| self.supports(skill))
                    .collect();
                let next = offset.saturating_add(16);
                json!({"skills": matches.iter().skip(offset).take(16)
                    .map(|skill| json!({"name":skill.name,"description":skill.description})).collect::<Vec<_>>(),
                    "total":matches.len(),"next_offset":(next < matches.len()).then_some(next)})
            }
            Some(Value::String(name)) => {
                match self.catalog.find(name).filter(|skill| self.supports(skill)) {
                    Some(skill) => json!({"name":skill.name,"procedure":skill.applied_block()}),
                    None => {
                        json!({"error":"This skill is unavailable with the current tools. Omit name to list available skills."})
                    }
                }
            }
            Some(_) => json!({"error":"name must be a string, or omitted to list skills."}),
        }
    }
}

#[async_trait::async_trait]
impl Tool for LoadSkill {
    fn is_read_only(&self) -> bool {
        true
    }
    fn is_concurrency_safe(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "load_skill"
    }
    fn description(&self) -> &'static str {
        "Discover available skill procedures (omit name), or load a named skill's instructions. Skills never grant additional tools or permissions."
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({"type":"object","properties":{
                "name":{"type":"string","description":"Skill name or alias; omit to discover available skills."},
                "query":{"type":"string","description":"Optional catalogue search."},
                "resource":{"type":"string","description":"Relative UTF-8 resource path within an installed skill; requires name."},
                "offset":{"type":"integer","minimum":0,"description":"Continuation offset: catalogue entry number, or byte offset for resource text."}
            },"additionalProperties":false}))
    }
    async fn execute(&self, _: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        if let Some(path) = args.get("resource") {
            let Some((name, path)) = args.get("name").and_then(Value::as_str).zip(path.as_str())
            else {
                return Ok(
                    json!({"error":"Resource reads require a skill name and a relative resource path."}),
                );
            };
            let Some(skill) = self
                .catalog
                .find(name)
                .filter(|skill| self.supports(skill) && !skill.builtin)
            else {
                return Ok(
                    json!({"error":"This installed skill is unavailable with the current tools."}),
                );
            };
            let Some(reader) = &self.resources else {
                return Ok(json!({"error":"Skill resources are unavailable in this session."}));
            };
            return Ok(match reader.read(skill.name.clone(), path.into()).await {
                Ok(text) => {
                    let start = match offset(&args) {
                        Ok(offset) => offset,
                        Err(error) => return Ok(error),
                    };
                    if start > text.len() || !text.is_char_boundary(start) {
                        return Ok(
                            json!({"error":"Use a resource byte offset returned by next_offset."}),
                        );
                    }
                    let end =
                        text.floor_char_boundary(start.saturating_add(16_000).min(text.len()));
                    json!({"skill":skill.name,"resource":path,"text":&text[start..end],"next_offset":(end < text.len()).then_some(end)})
                }
                Err(error) => json!({"error":error.to_string()}),
            });
        }
        Ok(self.response(&args))
    }
}

fn offset(args: &Value) -> Result<usize, Value> {
    match args.get("offset") {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| json!({"error":"offset must be a nonnegative integer."})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{Content, FinishReason, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part};
    use std::sync::Mutex;

    /// Loads a skill only when the standing guide offers it, then answers.
    #[derive(Default)]
    struct ChoosingModel {
        prompts: Mutex<Vec<String>>,
        procedure: Mutex<Option<String>>,
    }

    #[async_trait::async_trait]
    impl Llm for ChoosingModel {
        fn name(&self) -> &'static str {
            "skill-choice-test"
        }
        async fn generate_content(
            &self,
            request: LlmRequest,
            _: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let seen = format!("{:?}{:?}", request.contents, request.config);
            let loaded = request
                .contents
                .iter()
                .flat_map(|content| &content.parts)
                .find_map(|part| match part {
                    Part::FunctionResponse {
                        function_response, ..
                    } => function_response.response["procedure"]
                        .as_str()
                        .map(str::to_owned),
                    _ => None,
                });
            let offered = seen.contains("\\\"name\\\":\\\"meal-plan\\\"");
            self.prompts.lock().unwrap().push(seen);
            let part = match loaded {
                Some(procedure) => {
                    *self.procedure.lock().unwrap() = Some(procedure);
                    Part::Text {
                        text: "Here is the plan.".into(),
                    }
                }
                None if offered => Part::FunctionCall {
                    name: "load_skill".into(),
                    args: json!({"name":"meal-plan"}),
                    id: Some("choose".into()),
                    thought_signature: None,
                },
                None => Part::Text {
                    text: "Hello!".into(),
                },
            };
            let response = LlmResponse {
                content: Some(Content {
                    role: "model".into(),
                    parts: vec![part],
                }),
                finish_reason: Some(FinishReason::Stop),
                partial: false,
                turn_complete: true,
                ..LlmResponse::default()
            };
            Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
        }
    }

    fn catalog_with_meal_plan() -> Arc<SkillCatalog> {
        let mut skill = SkillCatalog::default().find("table").unwrap().clone();
        skill.name = "meal-plan".into();
        skill.aliases.clear();
        skill.builtin = false;
        skill.auto_apply = true;
        skill.tools.clear();
        skill.description = "Use when planning the family's meals for the week.".into();
        skill.instructions = "Ask about allergies first, then plan seven dinners.".into();
        let mut skills = sovra_core::skills::builtin_skills();
        skills.push(skill);
        Arc::new(SkillCatalog::new(skills).unwrap())
    }

    #[test]
    fn the_model_applies_a_matching_skill_unasked_and_untrusted_turns_never_see_the_list() {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                let model = Arc::new(ChoosingModel::default());
                let base = crate::AdkHarness::with_llm_and_tools(
                    &crate::AgentConfig::default(),
                    model.clone(),
                    crate::native_ui::tools(),
                )
                .unwrap();
                let harness = base.with_skill_catalog(catalog_with_meal_plan()).unwrap();
                let (events, _received) = async_channel::unbounded();
                harness
                    .run_prompt("auto-skill", "Plan dinners for next week", &events)
                    .await;
                let procedure = model.procedure.lock().unwrap().clone().expect("loaded");
                assert!(procedure.contains("allergies first"));
                assert!(!procedure.contains("Active user-selected skill."));
                let first = model.prompts.lock().unwrap()[0].clone();
                assert!(first.contains("If nothing clearly matches"));
                // The list names procedures but never discloses their bodies.
                assert!(!first.contains("allergies first"));
                // Rendering tools describe themselves, so their skills stay off the list.
                assert!(!first.contains("\\\"name\\\":\\\"table\\\""));

                model.prompts.lock().unwrap().clear();
                let scoped = harness.for_untrusted_sender().unwrap();
                scoped.run_prompt("stranger", "Plan dinners", &events).await;
                let prompts = model.prompts.lock().unwrap();
                assert!(!prompts.iter().any(|seen| seen.contains("meal-plan")));
            });
    }

    #[test]
    fn discovery_uses_real_capabilities_and_discloses_bodies_only_on_activation() {
        let tools = crate::native_ui::tools();
        let loader = LoadSkill::new(
            Arc::new(SkillCatalog::default()),
            tools.iter().map(|tool| tool.name().into()).collect(),
        );
        let listed = loader.response(&json!({})).to_string();
        assert!(listed.contains("table"));
        assert!(!listed.contains("web-search"));
        assert!(!listed.contains("instructions"));
        let loaded = loader.response(&json!({"name":"table"}));
        assert!(
            loaded["procedure"]
                .as_str()
                .unwrap()
                .contains("render_table")
        );
        assert!(
            loader
                .response(&json!({"name":"files"}))
                .get("error")
                .is_some()
        );
        assert!(
            loader
                .response(&json!({"name":"web-search"}))
                .get("error")
                .is_some()
        );
        assert!(loader.response(&json!({"name":42})).get("error").is_some());
        assert!(
            loader
                .response(&json!({"offset":-1}))
                .get("error")
                .is_some()
        );
        assert!(loader.response(&json!({"query":42})).get("error").is_some());
        assert!(loader.response(&json!([])).get("error").is_some());
        assert_eq!(loader.response(&json!({"offset":100}))["skills"], json!([]));
        assert_eq!(
            loader.response(&json!({"query":"table"}))["skills"][0]["name"],
            "table"
        );
    }
}

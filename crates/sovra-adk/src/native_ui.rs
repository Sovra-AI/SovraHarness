//! ADK-UI's AG-UI surface profile translated into native domain components.

use std::sync::Arc;

use adk_core::{Part, Tool, ToolContext};
use adk_ui::{ACTIVE_SURFACE_STATE_KEY, UiToolset, schema::Component};
use serde_json::Value;
use sovra_core::{AgUiWidget, UiComponent};

pub(crate) const INSTRUCTION: &str = "Native UI tools are optional. Answer normal conversational questions directly in Markdown, including ordinary lists and tables. Use a native UI tool when they ask for a card, AG-UI, or an interactive interface, or when a concrete choice or confirmation benefits from buttons. Do not turn greetings, explanations or simple answers into cards, menus or task workflows. Never invent progress, actions or tool results. When a connected service already showed a native card, do not wrap it again. If run_terminal_command is available, never ask in chat whether you may run a command or change files, and never paste a command in chat for the person to copy or run themselves: call that tool so they see Allow once, Always allow, and Don't allow on a card. Permission is that card, not a chat question. When they click a button, their next message is the button's visible label — treat that as their choice and continue. Use only tools actually available in this session.";

// Advertise only tools whose entire output can be rendered by this native client.
pub(crate) fn tools() -> Vec<Arc<dyn Tool>> {
    UiToolset::all_tools()
        .into_iter()
        .filter(|tool| {
            matches!(
                tool.name(),
                "render_card"
                    | "render_alert"
                    | "render_confirm"
                    | "render_table"
                    | "render_progress"
            )
        })
        .map(|tool| Arc::new(NativeTool(tool)) as Arc<dyn Tool>)
        .collect()
}

struct NativeTool(Arc<dyn Tool>);

#[async_trait::async_trait]
impl Tool for NativeTool {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> &str {
        self.0.description()
    }
    fn parameters_schema(&self) -> Option<Value> {
        self.0.parameters_schema()
    }
    async fn execute(&self, ctx: Arc<dyn ToolContext>, mut args: Value) -> adk_core::Result<Value> {
        // Transport is a host decision, not something the model should guess.
        if let Some(args) = args.as_object_mut() {
            args.insert("protocol".into(), Value::String("ag_ui".into()));
            args.insert(
                "ag_ui_thread_id".into(),
                Value::String(ctx.session_id().into()),
            );
        }
        self.0.execute(ctx, args).await
    }
}

pub(crate) fn extract_surfaces(event: &adk_core::Event) -> Vec<AgUiWidget> {
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
            let revision = event.actions.state_delta.get(ACTIVE_SURFACE_STATE_KEY);
            decode_surface(&function_response.response, revision)
        })
        .collect()
}

fn decode_surface(value: &Value, revision: Option<&Value>) -> Option<AgUiWidget> {
    if value.get("protocol")?.as_str()? != "ag_ui" {
        return None;
    }
    let events = value.pointer("/events")?.as_array()?;
    // ADK emits both ACTIVITY_SNAPSHOT and CUSTOM for the same surface. Consume
    // CUSTOM once, avoiding duplicate cards from the compatibility event pair.
    for event in events {
        if event.get("type").and_then(Value::as_str) != Some("CUSTOM")
            || event.get("name").and_then(Value::as_str) != Some("adk.ui.surface")
        {
            continue;
        }
        let surface = event.pointer("/value/surface")?;
        let surface_id = surface.get("surfaceId")?.as_str()?.to_owned();
        let response: adk_ui::schema::UiResponse =
            serde_json::from_value(surface.pointer("/dataModel/adk_ui_response")?.clone()).ok()?;
        let version = revision
            .filter(|r| r.get("id").and_then(Value::as_str) == Some(&surface_id))
            .and_then(|r| r.get("version"))
            .and_then(Value::as_u64)
            .unwrap_or(1);
        return Some(AgUiWidget::Components {
            surface_id,
            version,
            components: response.components.into_iter().map(convert).collect(),
        });
    }
    None
}

fn convert(component: Component) -> UiComponent {
    match component {
        Component::Text(text) => UiComponent::Text {
            text: text.content,
            heading: matches!(
                text.variant,
                adk_ui::schema::TextVariant::H1
                    | adk_ui::schema::TextVariant::H2
                    | adk_ui::schema::TextVariant::H3
                    | adk_ui::schema::TextVariant::H4
            ),
        },
        Component::Card(card) => UiComponent::Card {
            title: card.title.unwrap_or_default(),
            description: card.description.unwrap_or_default(),
            children: card
                .content
                .into_iter()
                .chain(card.footer.unwrap_or_default())
                .map(convert)
                .collect(),
        },
        Component::Stack(stack) => {
            let children = stack.children.into_iter().map(convert).collect();
            match stack.direction {
                adk_ui::schema::StackDirection::Horizontal => UiComponent::Row(children),
                adk_ui::schema::StackDirection::Vertical => UiComponent::Column(children),
            }
        }
        Component::Container(container) => {
            UiComponent::Column(container.children.into_iter().map(convert).collect())
        }
        Component::Table(table) => UiComponent::Table {
            rows: table
                .data
                .iter()
                .map(|row| {
                    table
                        .columns
                        .iter()
                        .map(|column| {
                            row.get(&column.accessor_key).map_or_else(String::new, |v| {
                                v.as_str().map_or_else(|| v.to_string(), str::to_owned)
                            })
                        })
                        .collect()
                })
                .collect(),
            headers: table.columns.into_iter().map(|c| c.header).collect(),
        },
        Component::List(list) => UiComponent::Column(
            list.items
                .into_iter()
                .enumerate()
                .map(|(i, text)| UiComponent::Text {
                    text: if list.ordered {
                        format!("{}. {text}", i + 1)
                    } else {
                        format!("• {text}")
                    },
                    heading: false,
                })
                .collect(),
        ),
        Component::KeyValue(values) => UiComponent::Table {
            headers: vec!["Name".into(), "Value".into()],
            rows: values
                .pairs
                .into_iter()
                .map(|pair| vec![pair.key, pair.value])
                .collect(),
        },
        Component::CodeBlock(code) => UiComponent::Code(code.code),
        Component::Badge(badge) => UiComponent::Badge(badge.label),
        Component::Alert(alert) => UiComponent::Alert {
            title: alert.title,
            description: alert.description.unwrap_or_default(),
        },
        Component::Progress(progress) => UiComponent::Progress {
            label: progress.label.unwrap_or_default(),
            value: progress.value.min(100),
        },
        Component::Button(button) => UiComponent::Button {
            label: button.label,
            action_id: button.action_id,
            disabled: button.disabled,
        },
        Component::Divider(_) => UiComponent::Divider,
        // These components cannot be emitted by the advertised tools. Be explicit
        // if a future tool adds them instead of silently dropping user content.
        other => UiComponent::Alert {
            title: "Unsupported component".into(),
            description: format!(
                "This component needs a native renderer: {}",
                serde_json::to_value(other)
                    .ok()
                    .and_then(|v| v.get("type").and_then(Value::as_str).map(str::to_owned))
                    .unwrap_or_default()
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_ui::interop::{UiSurface, surface_to_event_stream};
    use serde_json::json;

    #[test]
    fn decodes_actual_agui_stream_without_duplicate_compatibility_surface() {
        let surface = UiSurface::new("summary", "default", vec![]).with_data_model(Some(json!({
            "adk_ui_response": {"components": [
                {"type":"card", "title":"Report", "content":[
                    {"type":"text", "content":"Complete"},
                    {"type":"button", "label":"Continue", "action_id":"next"}
                ]},
                {"type":"table", "columns":[{"header":"Name", "accessor_key":"name"}], "data":[{"name":"Sovra"}]}
            ]}
        })));
        let payload =
            json!({"protocol":"ag_ui", "events":surface_to_event_stream(&surface,"thread","run")});
        let widget = decode_surface(&payload, Some(&json!({"id":"summary","version":2})))
            .expect("valid upstream surface");
        let AgUiWidget::Components {
            surface_id,
            version,
            components,
        } = widget
        else {
            panic!("components expected")
        };
        assert_eq!(surface_id, "summary");
        assert_eq!(version, 2);
        assert_eq!(components.len(), 2);
        assert!(
            matches!(&components[0], UiComponent::Card { children, .. } if children.len() == 2)
        );
        assert!(
            matches!(&components[1], UiComponent::Table { rows, .. } if rows == &vec![vec!["Sovra".to_owned()]])
        );
        assert!(decode_surface(&json!({"protocol":"ag_ui","events":[]}), None).is_none());
    }

    #[test]
    fn button_replies_are_the_visible_label_not_json() {
        assert!(!INSTRUCTION.contains("surface_ref"), "{INSTRUCTION}");
        assert!(!INSTRUCTION.contains("button_click"), "{INSTRUCTION}");
        assert!(INSTRUCTION.contains("visible label"), "{INSTRUCTION}");
        assert!(INSTRUCTION.contains("AG-UI"), "{INSTRUCTION}");
        assert!(
            INSTRUCTION.contains("run_terminal_command"),
            "{INSTRUCTION}"
        );
        assert!(INSTRUCTION.contains("Always allow"), "{INSTRUCTION}");
        assert!(
            INSTRUCTION.contains("paste a command in chat for the person to copy or run"),
            "{INSTRUCTION}"
        );
    }
}

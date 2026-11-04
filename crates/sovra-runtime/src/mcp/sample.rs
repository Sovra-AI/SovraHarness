//! A small, safe MCP server built into Sovra, so anyone can try MCP servers (and
//! developers can test against one) without installing anything:
//! `sovra --sample-mcp-server` speaks MCP over its input and output.
//!
//! Its tools touch nothing on the computer and never use the internet.

use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt as _,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
        ToolAnnotations,
    },
    service::RequestContext,
};
use serde_json::{Value, json};

pub use sovra_core::mcp::SAMPLE_SERVER_FLAG;
/// Name shown on the card when the sample is added.
pub(super) const SAMPLE_SERVER_NAME: &str = "Sovra sample tools";

#[derive(Debug, Clone, Copy)]
struct SampleServer;

/// Runs the sample server on stdin/stdout until the client disconnects.
pub fn run_sample_mcp_server() -> Result<(), crate::Error> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(crate::Error::Start)?;
    runtime.block_on(async {
        let service = SampleServer
            .serve(rmcp::transport::async_rw::AsyncRwTransport::new_server(
                tokio::io::stdin(),
                tokio::io::stdout(),
            ))
            .await
            .map_err(|error| crate::Error::Development(error.to_string()))?;
        service
            .waiting()
            .await
            .map_err(|error| crate::Error::Development(error.to_string()))?;
        Ok(())
    })
}

fn tools() -> Vec<Tool> {
    [
        (
            "roll_dice",
            "Roll a dice and say the number. Optionally choose how many sides it has (2 to 1000, normally 6).",
            json!({"sides": {"type": "integer", "minimum": 2, "maximum": 1000, "description": "How many sides the dice has."}}),
            Vec::<&str>::new(),
        ),
        (
            "tell_the_time",
            "Say the current date and time (in UTC).",
            json!({}),
            Vec::new(),
        ),
        (
            "make_a_checklist",
            "Turn a title and some items into a neat checklist.",
            json!({
                "title": {"type": "string", "description": "What the list is for."},
                "items": {"type": "array", "items": {"type": "string"}, "description": "The things to put on the list."}
            }),
            vec!["items"],
        ),
        (
            "count_words",
            "Count the words and letters in some text.",
            json!({"text": {"type": "string", "description": "The text to count."}}),
            vec!["text"],
        ),
    ]
    .into_iter()
    .map(|(name, description, properties, required)| {
        let schema = json!({"type": "object", "properties": properties, "required": required});
        Tool::new(
            name,
            description,
            schema
                .as_object()
                .expect("schema literal is an object")
                .clone(),
        )
        .with_annotations(
            ToolAnnotations::new()
                .read_only(true)
                .destructive(false)
                .open_world(false),
        )
    })
    .collect()
}

/// The answer for one tool call, or a plain problem.
fn answer(name: &str, arguments: &Value, now_unix: u64, random: u64) -> Result<String, String> {
    match name {
        "roll_dice" => {
            let sides = arguments
                .get("sides")
                .and_then(Value::as_u64)
                .unwrap_or(6)
                .clamp(2, 1000);
            let roll = random % sides + 1;
            Ok(format!("🎲 You rolled {roll} (on a {sides}-sided dice)."))
        }
        "tell_the_time" => {
            let days = i64::try_from(now_unix / 86_400).unwrap_or(0);
            let (year, month, day, weekday) = sovra_core::civil_date_from_unix_days(days);
            let seconds = now_unix % 86_400;
            Ok(format!(
                "It's {:02}:{:02} UTC on {weekday} {year}-{month:02}-{day:02}.",
                seconds / 3600,
                seconds % 3600 / 60
            ))
        }
        "make_a_checklist" => {
            let items: Vec<&str> = arguments
                .get("items")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            if items.is_empty() {
                return Err("Give me at least one thing to put on the list.".to_owned());
            }
            let title = arguments
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .unwrap_or("Checklist");
            let mut list = format!("{title}\n");
            for item in items.iter().take(200) {
                list.push_str("☐ ");
                list.push_str(item);
                list.push('\n');
            }
            Ok(list.trim_end().to_owned())
        }
        "count_words" => {
            let text = arguments
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let words = text.split_whitespace().count();
            let letters = text.chars().filter(|c| c.is_alphanumeric()).count();
            Ok(format!("That's {words} words and {letters} letters."))
        }
        other => Err(format!("This sample doesn't have a tool called “{other}”.")),
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn random_number() -> u64 {
    let mut bytes = [0_u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // A dice that always lands on its middle is still a working sample.
        return 3;
    }
    u64::from_le_bytes(bytes)
}

impl ServerHandler for SampleServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                SAMPLE_SERVER_NAME,
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("Harmless sample tools for trying MCP servers in Sovra.")
    }

    fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> {
        std::future::ready(Ok(ListToolsResult::with_all_items(tools())))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> {
        let arguments = request.arguments.map_or_else(|| json!({}), Value::Object);
        let result = match answer(&request.name, &arguments, now_unix(), random_number()) {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Err(problem) => CallToolResult::error(vec![ContentBlock::text(problem)]),
        };
        std::future::ready(Ok(result.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_are_described_and_answer_plainly() {
        let names: Vec<String> = tools()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "roll_dice",
                "tell_the_time",
                "make_a_checklist",
                "count_words"
            ]
        );
        assert_eq!(
            answer("roll_dice", &json!({"sides": 6}), 0, 9).as_deref(),
            Ok("🎲 You rolled 4 (on a 6-sided dice).")
        );
        assert_eq!(
            answer("roll_dice", &json!({"sides": 1}), 0, 5).as_deref(),
            Ok("🎲 You rolled 2 (on a 2-sided dice)."),
            "sides are clamped"
        );
        assert_eq!(
            answer("tell_the_time", &json!({}), 1_793_750_400 + 3_660, 0).as_deref(),
            Ok("It's 01:01 UTC on Wednesday 2026-11-04.")
        );
        assert_eq!(
            answer(
                "make_a_checklist",
                &json!({"title": "Picnic", "items": ["Bread", " ", "Jam"]}),
                0,
                0
            )
            .as_deref(),
            Ok("Picnic\n☐ Bread\n☐ Jam")
        );
        assert!(answer("make_a_checklist", &json!({"items": []}), 0, 0).is_err());
        assert_eq!(
            answer("count_words", &json!({"text": "Hello there, world"}), 0, 0).as_deref(),
            Ok("That's 3 words and 15 letters.")
        );
        assert!(answer("launch_rocket", &json!({}), 0, 0).is_err());
    }
}

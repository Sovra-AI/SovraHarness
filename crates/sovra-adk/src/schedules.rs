//! Local schedule tools: list, create, pause, resume, and run now.

use std::pin::Pin;
use std::sync::Arc;

use adk_core::{Tool, ToolContext};
use serde_json::{Value, json};
use sovra_core::communications::concrete_scheduled_recipient;
use sovra_core::connections::ChannelKind;
use sovra_core::{ScheduleCommand, ScheduledSend};

type CallFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;
type CallFn = Arc<dyn Fn(ScheduleCommand) -> CallFuture + Send + Sync>;

/// How the runtime reaches the live schedule book from a tool call.
#[derive(Clone)]
pub struct ScheduleTools {
    call: CallFn,
}

impl std::fmt::Debug for ScheduleTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScheduleTools").finish_non_exhaustive()
    }
}

impl ScheduleTools {
    pub fn from_fn<F, Fut>(call: F) -> Self
    where
        F: Fn(ScheduleCommand) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, String>> + Send + 'static,
    {
        Self {
            call: Arc::new(move |command| Box::pin(call(command))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    List,
    Create,
    Pause,
    Resume,
    RunNow,
}

pub(crate) const TOOL_NAMES: [&str; 5] = [
    "list_schedules",
    "create_schedule",
    "pause_schedule",
    "resume_schedule",
    "run_schedule_now",
];

pub(crate) const PROCEDURE: &str = "\n\nUse the schedule tools for one-shot reminders, repeating tasks, \
and confirmed timed WhatsApp or Telegram texts. \
Cadence is the person's own words. Pass delay and once phrases through unchanged: \
\"in 1 minute\", \"in a minute\", \"just once\". Never rewrite those as a clock time \
such as 3:53 — that would make a daily repeat. Other examples: \"every morning at 7:30\", \
\"weekdays at 9\", \"every 4 hours\", \"Fridays at 16:00\". \
Never invent a run: list first, never invent when, who, or next-run. \
Then create, pause, resume, or run now only when the person asked. \
When they confirmed who, which app, the exact words, and when, create_schedule with \
send_channel, send_recipient, and send_text. send_recipient must be the international \
phone number they typed (starting with +), me, an exact chat id, or person:<id> from \
find_people. Never store a first name — a timed send cannot look someone up by first \
name. That job SENDS at the scheduled time; \
they do not press Send then, and they need not be present. Do not offer only a draft \
for them to Send, and do not create a reminder-only prompt such as \"remind them to send it\". \
Live prepare_message still does not send until they press Send. \
After a change, list_schedules and tell them the name, when it next runs, and whether \
it will send the message, in plain words.";

pub(crate) const ROUTING: &str = "For setting up, changing, pausing, listing, or running a \
task once or on a repeat (in 1 minute, just once, every morning, weekdays, every few hours, \
or a named weekday), including a WhatsApp or Telegram text that should go out at a chosen \
time, call SchedulerAgent.";

impl Kind {
    fn name(self) -> &'static str {
        TOOL_NAMES[self as usize]
    }

    const fn reads_only(self) -> bool {
        matches!(self, Self::List)
    }

    fn description(self) -> &'static str {
        match self {
            Self::List => {
                "List the person's scheduled tasks, including whether each is on, paused, and when it next runs."
            }
            Self::Create => {
                "Create a scheduled task. `name` is a short title, `cadence` is the person's own words (in 1 minute, just once, every morning at 7:30, weekdays at 9, every 4 hours, Fridays at 16:00) — pass delay phrases unchanged, never as a clock — and `prompt` is what Sovra should do when it runs. For a confirmed timed WhatsApp or Telegram text, also pass `send_channel`, `send_recipient`, and `send_text`; that job sends at the scheduled time without another Send click."
            }
            Self::Pause => {
                "Pause a scheduled task so it stops running until resumed. `id` comes from list_schedules."
            }
            Self::Resume => "Turn a paused scheduled task back on. `id` comes from list_schedules.",
            Self::RunNow => {
                "Run a scheduled task once, right now, without waiting for the next scheduled time. `id` comes from list_schedules."
            }
        }
    }

    fn schema(self) -> Value {
        let string = |description: &str| json!({ "type": "string", "description": description });
        match self {
            Self::List => json!({ "type": "object", "properties": {} }),
            Self::Create => json!({
                "type": "object",
                "properties": {
                    "name": string("Short title."),
                    "cadence": string(
                        "How often, in the person's own words. Examples: in 1 minute, just once, every morning at 7:30. Pass delay phrases unchanged.",
                    ),
                    "prompt": string("What Sovra should do when it runs."),
                    "send_channel": {
                        "type": "string",
                        "enum": ["whats_app", "telegram"],
                        "description": "If this task should send a message, which app. A confirmed timed text goes out at the scheduled time; the person does not press Send then.",
                    },
                    "send_recipient": string(
                        "Who to message: me, an international WhatsApp number starting with +, an exact chat id, or person:<id> from find_people. Never a first name.",
                    ),
                    "send_text": string("Exact words to send at the scheduled time."),
                },
                "required": ["name", "prompt"],
            }),
            Self::Pause | Self::Resume | Self::RunNow => json!({
                "type": "object",
                "properties": { "id": string("The id from list_schedules.") },
                "required": ["id"],
            }),
        }
    }

    fn command(self, args: &Value) -> Result<ScheduleCommand, String> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default()
                .to_owned()
        };
        let required = |key: &str| {
            let value = text(key);
            if value.is_empty() {
                Err(format!("`{key}` is required."))
            } else {
                Ok(value)
            }
        };
        Ok(match self {
            Self::List => ScheduleCommand::Load,
            Self::Create => ScheduleCommand::Create {
                name: required("name")?,
                cadence: text("cadence"),
                prompt: required("prompt")?,
                send: optional_send(args)?,
            },
            Self::Pause => ScheduleCommand::Pause {
                id: required("id")?,
            },
            Self::Resume => ScheduleCommand::Resume {
                id: required("id")?,
            },
            Self::RunNow => ScheduleCommand::RunNow {
                id: required("id")?,
                session_id: None,
            },
        })
    }
}

fn optional_send(args: &Value) -> Result<Option<ScheduledSend>, String> {
    let channel = args.get("send_channel");
    let recipient = args
        .get("send_recipient")
        .and_then(Value::as_str)
        .map_or("", str::trim);
    let text = args
        .get("send_text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let any = channel.is_some() || !recipient.is_empty() || !text.is_empty();
    if !any {
        return Ok(None);
    }
    let channel: ChannelKind = serde_json::from_value(channel.cloned().unwrap_or(Value::Null))
        .map_err(|_| "Choose whats_app or telegram for send_channel.".to_owned())?;
    if recipient.is_empty() {
        return Err("Say who the timed message should go to.".into());
    }
    if text.is_empty() {
        return Err("Say the exact words to send at the scheduled time.".into());
    }
    concrete_scheduled_recipient(channel, recipient).map_err(|error| error.to_string())?;
    Ok(Some(ScheduledSend {
        channel,
        recipient: recipient.to_owned(),
        text: text.to_owned(),
    }))
}

pub(crate) fn tools(schedules: &ScheduleTools) -> Vec<Arc<dyn Tool>> {
    [
        Kind::List,
        Kind::Create,
        Kind::Pause,
        Kind::Resume,
        Kind::RunNow,
    ]
    .into_iter()
    .map(|kind| {
        Arc::new(ScheduleTool {
            kind,
            call: schedules.call.clone(),
        }) as Arc<dyn Tool>
    })
    .collect()
}

struct ScheduleTool {
    kind: Kind,
    call: CallFn,
}

#[async_trait::async_trait]
impl Tool for ScheduleTool {
    fn name(&self) -> &str {
        self.kind.name()
    }

    fn description(&self) -> &str {
        self.kind.description()
    }

    fn is_read_only(&self) -> bool {
        self.kind.reads_only()
    }

    fn is_concurrency_safe(&self) -> bool {
        self.kind.reads_only()
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(self.kind.schema())
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let command = match self.kind.command(&args) {
            Ok(command) => command,
            Err(message) => return Ok(json!({ "ok": false, "error": message })),
        };
        Ok(match (self.call)(command).await {
            Ok(result) => result,
            Err(message) => json!({ "ok": false, "error": message }),
        })
    }
}

impl crate::AdkHarness {
    /// Always-on local schedule tools so the orchestrator can offer `SchedulerAgent`.
    pub fn with_schedules(&self, schedules: Option<&ScheduleTools>) -> Result<Self, crate::Error> {
        let mut tools = self.base_tools.clone();
        let before = tools.len();
        tools.retain(|tool| !TOOL_NAMES.contains(&tool.name()));
        let added = match schedules {
            Some(schedules) if self.tools_enabled => crate::schedules::tools(schedules),
            _ => Vec::new(),
        };
        if added.is_empty() && tools.len() == before {
            return Ok(self.clone());
        }
        tools.extend(added);
        self.rebuild(self.base_instruction.clone(), tools)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AdkHarness;
    use adk_core::Llm;
    use sovra_core::AgentConfig;
    use std::sync::Mutex;

    struct IdleLlm;

    #[async_trait::async_trait]
    impl Llm for IdleLlm {
        fn name(&self) -> &'static str {
            "idle"
        }

        async fn generate_content(
            &self,
            _request: adk_core::LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<adk_core::LlmResponseStream> {
            Err(adk_core::AdkError::internal(
                adk_core::ErrorComponent::Model,
                "idle",
                "unused",
            ))
        }
    }

    #[test]
    fn schedule_copy_covers_once_soon_and_list_first() {
        assert!(PROCEDURE.contains("in 1 minute"), "{PROCEDURE}");
        assert!(PROCEDURE.contains("just once"), "{PROCEDURE}");
        assert!(PROCEDURE.contains("list first"), "{PROCEDURE}");
        assert!(
            PROCEDURE.contains("never invent when") || PROCEDURE.contains("Never invent when"),
            "{PROCEDURE}"
        );
        assert!(ROUTING.contains("in 1 minute"), "{ROUTING}");
        assert!(ROUTING.contains("just once"), "{ROUTING}");
        let create = Kind::Create.description();
        assert!(create.contains("in 1 minute"), "{create}");
        assert!(create.contains("just once"), "{create}");
        let cadence = Kind::Create
            .schema()
            .get("properties")
            .and_then(|props| props.get("cadence"))
            .and_then(|cadence| cadence.get("description"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        assert!(cadence.contains("in 1 minute"), "{cadence}");
        assert!(cadence.contains("just once"), "{cadence}");
        assert!(
            PROCEDURE.contains("SENDS at the scheduled time"),
            "{PROCEDURE}"
        );
        assert!(
            PROCEDURE.contains("prepare_message still does not send"),
            "{PROCEDURE}"
        );
        assert!(
            !PROCEDURE.contains("only a draft") || PROCEDURE.contains("Do not offer only a draft"),
            "{PROCEDURE}"
        );
        assert!(
            ROUTING.contains("WhatsApp") || ROUTING.contains("Telegram"),
            "{ROUTING}"
        );
        let create = Kind::Create.description();
        assert!(create.contains("send_channel"), "{create}");
        assert!(create.contains("scheduled time"), "{create}");
        let parsed = Kind::Create
            .command(&json!({
                "name": "Wake Ryan",
                "cadence": "in 1 minute",
                "prompt": "Send Ryan a WhatsApp: wake up from sovra",
                "send_channel": "whats_app",
                "send_recipient": "+44 7347 529758",
                "send_text": "wake up from sovra",
            }))
            .expect("create send");
        match parsed {
            ScheduleCommand::Create {
                send: Some(send), ..
            } => {
                assert_eq!(send.channel, ChannelKind::WhatsApp);
                assert_eq!(send.recipient, "+44 7347 529758");
                assert_eq!(send.text, "wake up from sovra");
            }
            other => panic!("expected a timed send, got {other:?}"),
        }
        assert!(
            PROCEDURE.contains("Never store a first name"),
            "{PROCEDURE}"
        );
        let rejected = Kind::Create.command(&json!({
            "name": "Wake Ryan",
            "cadence": "in 1 minute",
            "prompt": "Send Ryan a WhatsApp: wake up from sovra",
            "send_channel": "whats_app",
            "send_recipient": "Ryan",
            "send_text": "wake up from sovra",
        }));
        let err = rejected.expect_err("first name is not a timed-send recipient");
        assert!(err.contains("first name"), "{err}");
    }

    #[test]
    fn local_schedule_tools_offer_the_scheduler_without_an_account() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let schedules = ScheduleTools::from_fn(move |command| {
            log.lock().expect("log").push(format!("{command:?}"));
            async { Ok(json!({ "ok": true, "schedules": [] })) }
        });
        let harness =
            AdkHarness::with_llm_and_tools(&AgentConfig::default(), Arc::new(IdleLlm), Vec::new())
                .expect("harness")
                .with_schedules(Some(&schedules))
                .expect("schedules");
        let ids: Vec<String> = harness.delegates().into_iter().map(|(id, _)| id).collect();
        assert!(
            ids.iter().any(|id| id == "SchedulerAgent"),
            "SchedulerAgent missing from {ids:?}"
        );
        assert!(!ids.iter().any(|id| id == "GoogleWorkspace"));
        assert!(!ids.iter().any(|id| id == "CommunicationsAgent"));
    }
}

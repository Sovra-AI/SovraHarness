//! Programs and the many-services list, driven through the real actor with a tiny
//! MCP server written in POSIX `sh` (so these run on Unix only).

use super::*;

/// Speaks just enough MCP over stdio: refuses newer discovery so the client falls
/// back to `initialize`, lists two tools and echoes a private setting.
const FAKE_SERVER: &str = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  [ -z "$id" ] && continue
  case "$line" in
    *'"method":"initialize"'*)
      ver=$(printf '%s' "$line" | sed -n 's/.*"protocolVersion":"\([^"]*\)".*/\1/p')
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"%s","capabilities":{"tools":{}},"serverInfo":{"name":"Fake Notes","version":"1"}}}\n' "$id" "$ver" ;;
    *'"method":"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"Repeats a note","inputSchema":{"type":"object"}},{"name":"wipe","description":"Deletes every note","inputSchema":{"type":"object"}}]}}\n' "$id" ;;
    *'"method":"tools/call"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"echoed:%s"}]}}\n' "$id" "$TEST_TOKEN" ;;
    *)
      printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"Method not found"}}\n' "$id" ;;
  esac
done
"#;

struct Fixture {
    runtime: McpRuntime,
    events: Receiver<Event>,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sovra-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn fixture_in(dir: PathBuf) -> Fixture {
    let (events_tx, events) = async_channel::unbounded();
    let runtime = McpRuntime::start(
        None,
        Some(dir.join("services.json")),
        None,
        events_tx,
        &tokio::runtime::Handle::current(),
    );
    Fixture {
        runtime,
        events,
        dir,
    }
}

impl Fixture {
    /// Waits for the first MCP event that `pick` accepts.
    async fn until<T>(&self, mut pick: impl FnMut(&McpEvent) -> Option<T>) -> T {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let event = self.events.recv().await.expect("events open");
                if let Event::Mcp(event) = event
                    && let Some(found) = pick(&event)
                {
                    return found;
                }
            }
        })
        .await
        .expect("the expected event in time")
    }

    async fn service_where(
        &self,
        id: &str,
        mut test: impl FnMut(&McpService) -> bool,
    ) -> McpService {
        self.until(|event| match event {
            McpEvent::Upsert(service) if service.id == id && test(service) => Some(service.clone()),
            _ => None,
        })
        .await
    }

    async fn notice(&self) -> String {
        self.until(|event| match event {
            McpEvent::Notice(text) => Some(text.clone()),
            _ => None,
        })
        .await
    }

    fn add_fake(&self, name: &str, token: &str) -> String {
        let args = vec!["-c".to_owned(), FAKE_SERVER.to_owned()];
        let id = program_id_for("sh", &args);
        self.runtime.handle(McpCommand::AddProgram {
            name: name.into(),
            command: "sh".into(),
            args,
            env: vec![("TEST_TOKEN".into(), token.into())],
            working_dir: None,
        });
        id
    }

    async fn call(&self, id: &str, tool: &str) -> String {
        self.runtime
            .caller()
            .call(RemoteCall {
                service_id: id.into(),
                tool: tool.into(),
                arguments: json!({"text": "hi"}),
                session_id: "chat-1".into(),
            })
            .await
            .expect("an answer")
            .text
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_program_runs_only_after_approval_and_serves_its_tools() {
    let fixture = fixture_in(temp_dir("approve"));
    let id = fixture.add_fake("Fake notes", "tok-123");
    let waiting = fixture
        .service_where(&id, |s| s.phase == McpPhase::NeedsApproval)
        .await;
    assert_eq!(waiting.name, "Fake notes");
    assert_eq!(
        waiting.transport,
        McpTransport::Program {
            command: "sh".into(),
            args: vec!["-c".into(), FAKE_SERVER.into()],
            env_names: vec!["TEST_TOKEN".into()],
            working_dir: None,
        }
    );
    assert_eq!(
        fixture.runtime.offers().len(),
        0,
        "nothing runs before the OK"
    );

    fixture
        .runtime
        .handle(McpCommand::Approve { id: id.clone() });
    let connected = fixture
        .service_where(&id, |s| s.phase == McpPhase::Connected)
        .await;
    let names: Vec<&str> = connected.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["echo", "wipe"]);
    assert_eq!(fixture.runtime.offers().len(), 2);
    assert_eq!(
        fixture.call(&id, "echo").await,
        "echoed:tok-123",
        "private settings reach the program"
    );

    fixture.runtime.handle(McpCommand::SetToolEnabled {
        id: id.clone(),
        tool: "wipe".into(),
        enabled: false,
    });
    let switched = fixture
        .service_where(&id, |s| !s.disabled_tools.is_empty())
        .await;
    assert_eq!(switched.disabled_tools, ["wipe"]);
    let offered: Vec<String> = fixture
        .runtime
        .offers()
        .into_iter()
        .map(|offer| offer.tool.name)
        .collect();
    assert_eq!(offered, ["echo"]);
    assert_eq!(
        fixture.call(&id, "wipe").await,
        ConnectFailure::ToolUnavailable.text(),
        "a switched-off tool cannot be called"
    );

    // The saved list keeps the approval and the switch, never the private value.
    let saved = std::fs::read_to_string(fixture.dir.join("services.json")).expect("saved list");
    assert!(saved.contains("\"approved\""));
    assert!(saved.contains("wipe"));
    assert!(saved.contains("TEST_TOKEN"));
    assert!(!saved.contains("tok-123"), "{saved}");

    fixture
        .runtime
        .handle(McpCommand::Remove { id: id.clone() });
    fixture
        .until(|event| matches!(event, McpEvent::Removed { id: gone } if *gone == id).then_some(()))
        .await;
    assert_eq!(fixture.runtime.offers().len(), 0);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn pausing_stops_a_program_and_resuming_starts_it_again() {
    let fixture = fixture_in(temp_dir("pause"));
    let id = fixture.add_fake("", "x");
    let waiting = fixture
        .service_where(&id, |s| s.phase == McpPhase::NeedsApproval)
        .await;
    assert_eq!(waiting.name, "Sh", "a name is made up when none is given");
    fixture
        .runtime
        .handle(McpCommand::Approve { id: id.clone() });
    fixture
        .service_where(&id, |s| s.phase == McpPhase::Connected)
        .await;

    fixture.runtime.handle(McpCommand::SetEnabled {
        id: id.clone(),
        enabled: false,
    });
    let paused = fixture.service_where(&id, |s| !s.enabled).await;
    assert_eq!(paused.label(), "Paused");
    assert_eq!(paused.tools.len(), 0);
    assert_eq!(fixture.runtime.offers().len(), 0);
    assert_eq!(
        fixture.call(&id, "echo").await,
        ConnectFailure::ToolUnavailable.text()
    );

    fixture.runtime.handle(McpCommand::SetEnabled {
        id: id.clone(),
        enabled: true,
    });
    fixture
        .service_where(&id, |s| s.enabled && s.phase == McpPhase::Connected)
        .await;
    assert_eq!(fixture.call(&id, "echo").await, "echoed:x");

    fixture.runtime.handle(McpCommand::Rename {
        id: id.clone(),
        name: "  My notes ".into(),
    });
    let renamed = fixture.service_where(&id, |s| s.name == "My notes").await;
    assert_eq!(renamed.phase, McpPhase::Connected);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_program_that_quits_reports_what_it_said() {
    let fixture = fixture_in(temp_dir("crash"));
    let args = vec![
        "-c".to_owned(),
        "echo 'Missing NOTES_KEY setting' >&2; exit 3".to_owned(),
    ];
    let id = program_id_for("sh", &args);
    fixture.runtime.handle(McpCommand::AddProgram {
        name: "Broken".into(),
        command: "sh".into(),
        args,
        env: Vec::new(),
        working_dir: None,
    });
    fixture
        .service_where(&id, |s| s.phase == McpPhase::NeedsApproval)
        .await;
    fixture
        .runtime
        .handle(McpCommand::Approve { id: id.clone() });
    let failed = fixture
        .service_where(&id, |s| s.phase == McpPhase::Failed)
        .await;
    assert!(
        failed.message.contains("Missing NOTES_KEY setting"),
        "{}",
        failed.message
    );
    assert!(failed.needs_person());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_program_says_so_plainly() {
    let fixture = fixture_in(temp_dir("missing"));
    let id = program_id_for("definitely-not-a-program-sovra", &[]);
    fixture.runtime.handle(McpCommand::AddProgram {
        name: String::new(),
        command: "definitely-not-a-program-sovra".into(),
        args: Vec::new(),
        env: Vec::new(),
        working_dir: None,
    });
    fixture
        .service_where(&id, |s| s.phase == McpPhase::NeedsApproval)
        .await;
    fixture
        .runtime
        .handle(McpCommand::Approve { id: id.clone() });
    let failed = fixture
        .service_where(&id, |s| s.phase == McpPhase::Failed)
        .await;
    assert!(
        failed.message.contains("Couldn't find"),
        "{}",
        failed.message
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn approved_programs_start_again_after_a_restart() {
    let dir = temp_dir("restart");
    let id = {
        let fixture = fixture_in(dir.clone());
        let id = fixture.add_fake("Notes", "kept-in-keychain");
        fixture
            .service_where(&id, |s| s.phase == McpPhase::NeedsApproval)
            .await;
        fixture
            .runtime
            .handle(McpCommand::Approve { id: id.clone() });
        fixture
            .service_where(&id, |s| s.phase == McpPhase::Connected)
            .await;
        // Keep the folder for the second start.
        let mut fixture = fixture;
        fixture.dir = PathBuf::new();
        id
    };
    let fixture = fixture_in(dir);
    fixture.runtime.handle(McpCommand::Load);
    let loaded = fixture
        .until(|event| match event {
            McpEvent::Loaded(state) => Some(state.clone()),
            _ => None,
        })
        .await;
    assert_eq!(loaded.services.len(), 1);
    assert_eq!(loaded.services[0].name, "Notes");
    fixture
        .service_where(&id, |s| s.phase == McpPhase::Connected)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pasted_settings_add_many_services_and_hide_private_values() {
    let fixture = fixture_in(temp_dir("import"));
    let text = r#"{"mcpServers": {
        "files": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"], "env": {"SECRET_ENV": "env-hunter2"}},
        "notes": {"url": "http://127.0.0.1:9/mcp", "headers": {"X-Api-Key": "hdr-hunter2", "Content-Type": "evil"}},
        "old": {"command": "uvx", "args": ["mcp-server-git"], "disabled": true}
    }}"#;
    fixture
        .runtime
        .handle(McpCommand::Import { text: text.into() });
    let mut services = HashMap::new();
    let notice = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match fixture.events.recv().await.expect("events") {
                Event::Mcp(McpEvent::Upsert(service)) => {
                    services.insert(service.name.clone(), service);
                }
                Event::Mcp(McpEvent::Notice(text)) => return text,
                _ => {}
            }
        }
    })
    .await
    .expect("notice");
    assert!(notice.starts_with("Added 3 services."), "{notice}");
    assert!(notice.contains("wait for your OK"));
    assert_eq!(services["files"].phase, McpPhase::NeedsApproval);
    assert!(!services["old"].enabled, "disabled entries arrive paused");
    assert_eq!(
        services["notes"].header_names,
        ["X-Api-Key"],
        "reserved headers are dropped"
    );
    // The web address is reconnected in the background and fails politely.
    let notes_id = services["notes"].id.clone();
    let failed = fixture
        .service_where(&notes_id, |s| s.phase == McpPhase::Failed)
        .await;
    assert_eq!(failed.message, ConnectFailure::Unreachable.text());

    let saved = std::fs::read_to_string(fixture.dir.join("services.json")).expect("saved");
    assert!(!saved.contains("hunter2"), "{saved}");
    assert!(!format!("{services:?}").contains("hunter2"));

    fixture.runtime.handle(McpCommand::Import {
        text: "npx a && rm -rf ~".into(),
    });
    assert_eq!(fixture.notice().await, ImportProblem::NotOneProgram.text());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_old_keychain_list_moves_to_the_file() {
    let dir = temp_dir("migrate");
    let (events_tx, events) = async_channel::unbounded();
    let (inbox, rx) = async_channel::unbounded();
    let mut vault = Vault::new(None);
    vault
        .put(
            CATALOGUE_ID,
            r#"[{"id":"svc-old","name":"Old","address":"http://127.0.0.1:9/mcp","issuer":""}]"#,
        )
        .await;
    let board = Arc::new(ToolBoard::default());
    let actor = Actor {
        handle: tokio::runtime::Handle::current(),
        inbox: inbox.downgrade(),
        events: events_tx,
        http: http_client(),
        vault,
        catalogue_path: Some(dir.join("services.json")),
        environment: None,
        services: Vec::new(),
        board,
        next_rpc: 1,
        listeners: HashMap::new(),
        programs: HashMap::new(),
    };
    #[expect(clippy::disallowed_methods, reason = "test owns the actor task")]
    let task = tokio::spawn(actor.run(rx));
    inbox
        .send(Input::Command(McpCommand::Load))
        .await
        .expect("send");
    let loaded = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(Event::Mcp(McpEvent::Loaded(state))) = events.recv().await {
                return state;
            }
        }
    })
    .await
    .expect("loaded");
    assert_eq!(loaded.services[0].name, "Old");
    assert!(loaded.services[0].enabled);
    let saved = std::fs::read_to_string(dir.join("services.json")).expect("moved to file");
    assert!(saved.contains("svc-old"));
    inbox.close();
    let _ = task.await;
    let _ = std::fs::remove_dir_all(dir);
}

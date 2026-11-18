//! Remote plugin lifecycle through the real command/event boundary.

use sovra_core::{
    AgentConfig, Command, Event,
    plugins::{
        PluginCatalog, PluginCommand, PluginEntry, PluginEvent, PluginLibrary, PluginPackage,
        PluginStore,
    },
    skills::{SkillError, SkillEvent},
};
use sovra_runtime::{Client, RuntimeServices};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const PACKAGE: &[u8] =
    include_bytes!("../../../examples/plugins/packages/weekly-planning-1.0.0.json");

#[derive(Debug, Default)]
struct Store {
    library: Mutex<PluginLibrary>,
    packages: Mutex<BTreeMap<String, Vec<u8>>>,
}
impl PluginStore for Store {
    fn load(&self) -> Result<PluginLibrary, SkillError> {
        Ok(self.library.lock().expect("plugin library").clone())
    }
    fn save(&self, library: &PluginLibrary) -> Result<(), SkillError> {
        *self.library.lock().expect("plugin library") = library.clone();
        Ok(())
    }
    fn package(&self, entry: &PluginEntry) -> Result<PluginPackage, SkillError> {
        let packages = self.packages.lock().expect("plugin packages");
        entry.verify_package(
            packages
                .get(&entry.sha256)
                .ok_or_else(|| SkillError("Missing cached package".into()))?,
        )
    }
    fn cache(&self, entry: &PluginEntry, bytes: &[u8]) -> Result<(), SkillError> {
        entry.verify_package(bytes)?;
        self.packages
            .lock()
            .expect("plugin packages")
            .insert(entry.sha256.clone(), bytes.to_vec());
        Ok(())
    }
    /// A minimal stand-in for the skills adapter's `SKILL.md` parser.
    fn read_skill_file(
        &self,
        folder: Option<&str>,
        content: &str,
    ) -> Result<sovra_core::plugins::PluginSkill, SkillError> {
        let (header, body) = content
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .ok_or_else(|| SkillError("no frontmatter".into()))?;
        let field = |key: &str| {
            header
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .map(str::trim)
                .unwrap_or_default()
                .to_owned()
        };
        let name = field("name:");
        if folder.is_some_and(|folder| folder != name) {
            return Err(SkillError("name must match folder".into()));
        }
        sovra_core::plugins::repository_skill(
            name.clone(),
            &name,
            &field("description:"),
            body.trim().into(),
        )
    }
}

async fn until(client: &Client, done: impl Fn(&PluginEvent) -> bool) -> Vec<Event> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut seen = Vec::new();
        loop {
            let event = client.events().recv().await.expect("runtime event");
            let finished = matches!(&event, Event::Plugins(event) if done(event));
            seen.push(event);
            if finished {
                return seen;
            }
        }
    })
    .await
    .expect("plugin response before deadline")
}

fn contains_plugin_skill(events: &[Event]) -> bool {
    events.iter().any(|event| matches!(event, Event::Skills(SkillEvent::Loaded { skills, .. }) if skills.iter().any(|skill| skill.metadata.contains_key(sovra_core::plugins::PLUGIN_METADATA_KEY))))
}

#[tokio::test]
async fn remote_catalog_paging_install_activation_offline_restart_and_corrupt_update() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let entry = PluginEntry::for_package(format!("{root}/package.json"), PACKAGE).unwrap();
    let mut broken_update = entry.clone();
    broken_update.version = "1.0.1".into();
    broken_update.package_url = format!("{root}/broken.json");
    let mut updated_package: PluginPackage = serde_json::from_slice(PACKAGE).unwrap();
    updated_package.version = "1.0.1".into();
    let updated_bytes = serde_json::to_vec(&updated_package).unwrap();
    let updated_entry =
        PluginEntry::for_package(format!("{root}/updated.json"), &updated_bytes).unwrap();
    let routes = BTreeMap::from([
        (
            "/catalog.json",
            serde_json::to_vec(&PluginCatalog {
                schema_version: 1,
                plugins: vec![entry.clone()],
                next: Some(format!("{root}/page2.json")),
                previous: None,
            })
            .unwrap(),
        ),
        (
            "/page2.json",
            serde_json::to_vec(&PluginCatalog {
                schema_version: 1,
                plugins: vec![broken_update],
                next: Some(format!("{root}/page3.json")),
                previous: Some(format!("{root}/catalog.json")),
            })
            .unwrap(),
        ),
        (
            "/page3.json",
            serde_json::to_vec(&PluginCatalog {
                schema_version: 1,
                plugins: vec![updated_entry.clone()],
                next: None,
                previous: Some(format!("{root}/page2.json")),
            })
            .unwrap(),
        ),
        ("/updated.json", updated_bytes),
        ("/package.json", PACKAGE.to_vec()),
        ("/broken.json", b"incomplete download".to_vec()),
    ]);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requested = requests.clone();
    let server = tokio::runtime::Handle::current().spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0; 8192];
            let length = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..length]);
            let path = request.split_whitespace().nth(1).unwrap();
            requested.lock().unwrap().push(path.to_owned());
            let body = routes.get(path).expect("known test route");
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(body).await.unwrap();
        }
    });
    let store = Arc::new(Store::default());
    let client = sovra_runtime::start_with_services(
        "plugin-test",
        AgentConfig::default(),
        RuntimeServices {
            plugins: Some(store.clone()),
            ..RuntimeServices::default()
        },
    )
    .unwrap();
    until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    client
        .send(Command::Plugins(PluginCommand::Browse {
            url: format!("{root}/catalog.json"),
        }))
        .unwrap();
    until(&client, |event| {
        matches!(event, PluginEvent::Catalog { .. })
    })
    .await;
    assert_eq!(
        *requests.lock().unwrap(),
        ["/catalog.json"],
        "browsing does not prefetch packages or other pages"
    );
    client
        .send(Command::Plugins(PluginCommand::Install {
            id: "io.other.unlisted".into(),
        }))
        .unwrap();
    until(&client, |event| matches!(event, PluginEvent::Error(_))).await;
    assert_eq!(requests.lock().unwrap().len(), 1);
    client
        .send(Command::Plugins(PluginCommand::Install {
            id: entry.id.clone(),
        }))
        .unwrap();
    let installed = until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    assert!(
        !contains_plugin_skill(&installed),
        "download does not activate instructions"
    );
    assert!(!store.load().unwrap().installed[0].enabled);
    client
        .send(Command::Plugins(PluginCommand::SetEnabled {
            id: entry.id.clone(),
            enabled: true,
        }))
        .unwrap();
    assert!(contains_plugin_skill(
        &until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await
    ));
    client
        .send(Command::Plugins(PluginCommand::NextPage))
        .unwrap();
    until(&client, |event| {
        matches!(event, PluginEvent::Catalog { .. })
    })
    .await;
    assert_eq!(
        store.load().unwrap().catalog_url,
        Some(format!("{root}/catalog.json")),
        "paging preserves the publisher identity"
    );
    client
        .send(Command::Plugins(PluginCommand::Install {
            id: entry.id.clone(),
        }))
        .unwrap();
    until(&client, |event| matches!(event, PluginEvent::Error(_))).await;
    assert_eq!(store.load().unwrap().installed[0].entry.version, "1.0.0");
    client
        .send(Command::Plugins(PluginCommand::NextPage))
        .unwrap();
    until(&client, |event| {
        matches!(event, PluginEvent::Catalog { .. })
    })
    .await;
    client
        .send(Command::Plugins(PluginCommand::Install {
            id: entry.id.clone(),
        }))
        .unwrap();
    let updated = until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    assert!(contains_plugin_skill(&updated));
    assert_eq!(store.load().unwrap().installed[0].entry, updated_entry);
    assert!(
        store.load().unwrap().installed[0].enabled,
        "updating keeps the person's activation choice"
    );
    client
        .send(Command::Plugins(PluginCommand::PreviousPage))
        .unwrap();
    until(&client, |event| {
        matches!(event, PluginEvent::Catalog { .. })
    })
    .await;
    client.send(Command::Shutdown).unwrap();
    server.abort();
    let reopened = sovra_runtime::start_with_services(
        "plugin-test",
        AgentConfig::default(),
        RuntimeServices {
            plugins: Some(store.clone()),
            ..RuntimeServices::default()
        },
    )
    .unwrap();
    assert!(contains_plugin_skill(
        &until(&reopened, |event| matches!(event, PluginEvent::Loaded(_))).await
    ));
    reopened
        .send(Command::Plugins(PluginCommand::Inspect {
            id: entry.id.clone(),
        }))
        .unwrap();
    let details = until(&reopened, |event| {
        matches!(event, PluginEvent::Details { .. })
    })
    .await;
    assert!(details.iter().any(|event| matches!(event, Event::Plugins(PluginEvent::Details { package, .. }) if package.skills.len() == 1)));
    reopened
        .send(Command::Plugins(PluginCommand::SetEnabled {
            id: entry.id.clone(),
            enabled: false,
        }))
        .unwrap();
    assert!(!contains_plugin_skill(
        &until(&reopened, |event| matches!(event, PluginEvent::Loaded(_))).await
    ));
    reopened.send(Command::Shutdown).unwrap();
}

#[tokio::test]
async fn a_damaged_active_package_keeps_its_disable_controls_and_other_skills() {
    let store = Arc::new(Store::default());
    let entry =
        PluginEntry::for_package("https://example.com/package.json".into(), PACKAGE).unwrap();
    store
        .library
        .lock()
        .unwrap()
        .installed
        .push(sovra_core::plugins::InstalledPlugin {
            entry: entry.clone(),
            catalog_url: "https://example.com/catalog.json".into(),
            enabled: true,
            github: None,
        });
    let client = sovra_runtime::start_with_services(
        "plugin-test",
        AgentConfig::default(),
        RuntimeServices {
            plugins: Some(store),
            ..RuntimeServices::default()
        },
    )
    .unwrap();
    let loaded = until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    assert!(loaded.iter().any(|event| matches!(event, Event::Skills(SkillEvent::Loaded { issues, skills }) if !issues.is_empty() && !skills.is_empty())));
    client
        .send(Command::Plugins(PluginCommand::SetEnabled {
            id: entry.id,
            enabled: false,
        }))
        .unwrap();
    until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    client.send(Command::Shutdown).unwrap();
}

#[tokio::test]
async fn damaged_plugins_can_be_disabled_individually() {
    let store = Arc::new(Store::default());
    let first =
        PluginEntry::for_package("https://example.com/package.json".into(), PACKAGE).unwrap();
    let mut second = first.clone();
    second.id = "io.example.another-damaged-plugin".into();
    store.library.lock().unwrap().installed = [first.clone(), second.clone()]
        .into_iter()
        .map(|entry| sovra_core::plugins::InstalledPlugin {
            entry,
            catalog_url: "https://example.com/catalog.json".into(),
            enabled: true,
            github: None,
        })
        .collect();
    let client = sovra_runtime::start_with_services(
        "plugin-test",
        AgentConfig::default(),
        RuntimeServices {
            plugins: Some(store.clone()),
            ..RuntimeServices::default()
        },
    )
    .unwrap();
    until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    for id in [first.id, second.id] {
        client
            .send(Command::Plugins(PluginCommand::SetEnabled {
                id: id.clone(),
                enabled: false,
            }))
            .unwrap();
        let result = until(&client, |event| {
            matches!(event, PluginEvent::Loaded(_) | PluginEvent::Error(_))
        })
        .await;
        assert!(
            !result
                .iter()
                .any(|event| matches!(event, Event::Plugins(PluginEvent::Error(_)))),
            "an unrelated broken package must not prevent disabling this plugin: {result:?}"
        );
        assert!(
            !store
                .load()
                .unwrap()
                .installed
                .iter()
                .find(|plugin| plugin.entry.id == id)
                .unwrap()
                .enabled
        );
    }
    client.send(Command::Shutdown).unwrap();
}

#[tokio::test]
async fn activation_at_capacity_cannot_displace_an_existing_enabled_plugin() {
    use sovra_core::plugins::InstalledPlugin;
    let store = Arc::new(Store::default());
    let slots = sovra_core::skills::MAX_SKILLS - sovra_core::skills::builtin_skills().len();
    let mut library = PluginLibrary::default();
    for index in 0..=slots {
        let mut package: PluginPackage = serde_json::from_slice(PACKAGE).unwrap();
        package.id = format!("io.example.capacity-{index}");
        let bytes = serde_json::to_vec(&package).unwrap();
        let entry =
            PluginEntry::for_package(format!("https://example.com/package-{index}.json"), &bytes)
                .unwrap();
        store.cache(&entry, &bytes).unwrap();
        library.installed.push(InstalledPlugin {
            entry,
            catalog_url: "https://example.com/catalog.json".into(),
            enabled: index > 0,
            github: None,
        });
    }
    store.save(&library).unwrap();
    let client = sovra_runtime::start_with_services(
        "plugin-test",
        AgentConfig::default(),
        RuntimeServices {
            plugins: Some(store.clone()),
            ..RuntimeServices::default()
        },
    )
    .unwrap();
    let loaded = until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    assert!(loaded.iter().any(|event| matches!(event, Event::Skills(SkillEvent::Loaded { skills, issues }) if skills.len() == sovra_core::skills::MAX_SKILLS && issues.is_empty())));
    client
        .send(Command::Plugins(PluginCommand::SetEnabled {
            id: library.installed[0].entry.id.clone(),
            enabled: true,
        }))
        .unwrap();
    let result = until(&client, |event| {
        matches!(event, PluginEvent::Loaded(_) | PluginEvent::Error(_))
    })
    .await;
    assert!(
        result
            .iter()
            .any(|event| matches!(event, Event::Plugins(PluginEvent::Error(_)))),
        "new activation must fail instead of hiding a later existing plugin"
    );
    assert_eq!(store.load().unwrap(), library);
    client.send(Command::Shutdown).unwrap();
}

const FIRST: &str = "0123456789abcdef0123456789abcdef01234567";
const SECOND: &str = "89abcdef0123456789abcdef0123456789abcdef";

fn skill_file(name: &str, body: &str) -> Vec<u8> {
    format!("---\nname: {name}\ndescription: Helps with {name}.\n---\n{body}\n").into_bytes()
}

fn github_commit(routes: &mut BTreeMap<String, Vec<u8>>, commit: &str, skills: &[(&str, &str)]) {
    routes.insert(
        "/api/repos/owner/skills/commits/main".into(),
        commit.as_bytes().to_vec(),
    );
    // A skill whose name does not match its folder is left out, never fatal, and so
    // is a picture inside a skill folder.
    let mut tree = vec![
        serde_json::json!({"path": "skills/notes/logo.png", "type": "blob", "size": 20}),
        serde_json::json!({"path": ".mcp.json", "type": "blob"}),
        serde_json::json!({"path": "skills/template/SKILL.md", "type": "blob"}),
    ];
    routes.insert(
        format!("/raw/owner/skills/{commit}/skills/template/SKILL.md"),
        skill_file("template-skill", "Replace me."),
    );
    for (name, body) in skills {
        tree.push(serde_json::json!({"path": format!("skills/{name}/SKILL.md"), "type": "blob"}));
        tree.push(serde_json::json!({"path": format!("skills/{name}/guide.md"), "type": "blob", "size": 20}));
        routes.insert(
            format!("/raw/owner/skills/{commit}/skills/{name}/guide.md"),
            format!("Guide for {name}.").into_bytes(),
        );
        routes.insert(
            format!("/raw/owner/skills/{commit}/skills/{name}/SKILL.md"),
            skill_file(name, body),
        );
    }
    routes.insert(
        format!("/api/repos/owner/skills/git/trees/{commit}?recursive=1"),
        serde_json::to_vec(&serde_json::json!({"tree": tree, "truncated": false}))
            .expect("tree json"),
    );
    routes.insert(
        format!("/raw/owner/skills/{commit}/.mcp.json"),
        br#"{"mcpServers":{"docs":{"url":"https://docs.example/mcp"},"local":{"command":"npx"}}}"#
            .to_vec(),
    );
}

#[tokio::test]
async fn github_repository_adds_switched_off_then_updates_only_after_review() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let mut initial = BTreeMap::from([(
        "/api/repos/owner/skills".to_owned(),
        br#"{"default_branch":"main","description":"Writing helpers","private":false}"#.to_vec(),
    )]);
    github_commit(&mut initial, FIRST, &[("notes", "Take tidy notes.")]);
    let routes = Arc::new(Mutex::new(initial));
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let (served, requested) = (routes.clone(), requests.clone());
    let _server = tokio::runtime::Handle::current().spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0; 8192];
            let length = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..length]);
            let path = request.split_whitespace().nth(1).unwrap().to_owned();
            requested.lock().unwrap().push(path.clone());
            let body = served.lock().unwrap().get(&path).cloned();
            let head = match &body {
                Some(body) => format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                ),
                None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .into(),
            };
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(&body.unwrap_or_default()).await.unwrap();
        }
    });
    let store = Arc::new(Store::default());
    let client = sovra_runtime::start_with_services(
        "plugin-github-test",
        AgentConfig::default(),
        RuntimeServices {
            plugins: Some(store.clone()),
            plugin_hosts: sovra_core::plugins::GitHubHosts {
                api: format!("{root}/api"),
                raw: format!("{root}/raw"),
            },
            ..RuntimeServices::default()
        },
    )
    .unwrap();
    until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    let send = |command| client.send(Command::Plugins(command)).unwrap();

    send(PluginCommand::AddFromGitHub {
        address: "https://github.com/owner/skills".into(),
    });
    let added = until(&client, |event| {
        matches!(event, PluginEvent::Added { .. } | PluginEvent::Error(_))
    })
    .await;
    let Some(Event::Plugins(PluginEvent::Added {
        id,
        package,
        left_out,
    })) = added.last()
    else {
        panic!("expected the repository to be added: {added:?}");
    };
    assert_eq!(
        left_out,
        &[
            "notes: 1 file that is not text, very large, or over the limit",
            "template (name must match folder)"
        ],
        "a skill that cannot be read is named and skipped; the rest are still added"
    );
    let id = id.clone();
    assert_eq!(id, "io.github.owner.skills");
    assert_eq!(package.skills[0].instructions, "Take tidy notes.");
    assert_eq!(
        package.skills[0]
            .resources
            .get("guide.md")
            .map(String::as_str),
        Some("Guide for notes."),
        "the skill folder's text files come with it"
    );
    assert_eq!(
        package.services,
        [sovra_core::plugins::PluginService {
            name: "docs".into(),
            address: "https://docs.example/mcp".into(),
        }],
        "only web connections are suggested; program-starting servers are skipped"
    );
    assert!(
        !contains_plugin_skill(&added),
        "adding does not switch it on"
    );
    let saved = store.load().unwrap().installed.remove(0);
    assert!(!saved.enabled);
    assert_eq!(saved.catalog_url, "https://github.com/owner/skills");
    assert_eq!(saved.github.as_ref().unwrap().commit, FIRST);

    send(PluginCommand::AddFromGitHub {
        address: "owner/skills".into(),
    });
    let again = until(&client, |event| matches!(event, PluginEvent::Error(_))).await;
    assert!(
        matches!(again.last(), Some(Event::Plugins(PluginEvent::Error(text))) if text.contains("already added"))
    );

    send(PluginCommand::SetEnabled {
        id: id.clone(),
        enabled: true,
    });
    assert!(contains_plugin_skill(
        &until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await
    ));

    requests.lock().unwrap().clear();
    send(PluginCommand::CheckForUpdate { id: id.clone() });
    until(&client, |event| {
        matches!(event, PluginEvent::UpToDate { .. })
    })
    .await;
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .all(|path| !path.contains("/trees/")),
        "an unchanged commit is not downloaded again"
    );

    github_commit(
        &mut routes.lock().unwrap(),
        SECOND,
        &[("notes", "Take tidier notes."), ("plan", "Plan the week.")],
    );
    send(PluginCommand::CheckForUpdate { id: id.clone() });
    let ready = until(&client, |event| {
        matches!(
            event,
            PluginEvent::UpdateReady { .. } | PluginEvent::Error(_)
        )
    })
    .await;
    let Some(Event::Plugins(PluginEvent::UpdateReady { changes, .. })) = ready.last() else {
        panic!("expected an update: {ready:?}");
    };
    assert_eq!(changes.added, ["plan"]);
    assert_eq!(changes.changed, ["notes"]);
    assert_eq!(
        store.load().unwrap().installed[0]
            .github
            .as_ref()
            .unwrap()
            .commit,
        FIRST,
        "checking never replaces the installed copy"
    );

    send(PluginCommand::ApplyUpdate { id: id.clone() });
    let applied = until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    assert!(
        contains_plugin_skill(&applied),
        "the update keeps it switched on"
    );
    let updated = store.load().unwrap().installed.remove(0);
    assert_eq!(updated.github.as_ref().unwrap().commit, SECOND);
    assert!(updated.enabled);

    send(PluginCommand::ApplyUpdate { id: id.clone() });
    until(&client, |event| matches!(event, PluginEvent::Error(_))).await;

    send(PluginCommand::AddFromGitHub {
        address: "owner/missing".into(),
    });
    let missing = until(&client, |event| matches!(event, PluginEvent::Error(_))).await;
    assert!(
        matches!(missing.last(), Some(Event::Plugins(PluginEvent::Error(text))) if text.contains("couldn't find"))
    );
    assert_eq!(store.load().unwrap().installed.len(), 1);
}

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
    fn remove_unused(&self, library: &PluginLibrary) -> Result<usize, SkillError> {
        let keep: Vec<_> = library
            .installed
            .iter()
            .map(|plugin| plugin.entry.sha256.clone())
            .collect();
        let mut packages = self.packages.lock().expect("plugin packages");
        let before = packages.len();
        packages.retain(|sha256, _| keep.contains(sha256));
        Ok(before - packages.len())
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
const THIRD: &str = "fedcba9876543210fedcba9876543210fedcba98";

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

/// Answers GitHub API and raw-file requests from `routes`, recording each path.
fn serve_github(
    listener: tokio::net::TcpListener,
    served: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    requested: Arc<Mutex<Vec<String>>>,
) {
    tokio::runtime::Handle::current().spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.expect("test fixture");
            let mut buffer = vec![0; 8192];
            let length = socket.read(&mut buffer).await.expect("test fixture");
            let request = String::from_utf8_lossy(&buffer[..length]);
            let path = request
                .split_whitespace()
                .nth(1)
                .expect("test fixture")
                .to_owned();
            requested.lock().expect("test fixture").push(path.clone());
            let body = served.lock().expect("test fixture").get(&path).cloned();
            let head = match &body {
                Some(body) => format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                ),
                None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .into(),
            };
            socket
                .write_all(head.as_bytes())
                .await
                .expect("test fixture");
            socket
                .write_all(&body.unwrap_or_default())
                .await
                .expect("test fixture");
        }
    });
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
    serve_github(listener, routes.clone(), requests.clone());
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

    requests.lock().unwrap().clear();
    send(PluginCommand::AddFromGitHub {
        address: "owner/skills".into(),
    });
    let again = until(&client, |event| matches!(event, PluginEvent::Error(_))).await;
    assert!(
        matches!(again.last(), Some(Event::Plugins(PluginEvent::Error(text))) if text.contains("already added"))
    );
    assert!(
        requests.lock().unwrap().is_empty(),
        "adding a repository twice is refused before anything is downloaded"
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
    assert_eq!(
        store.packages.lock().unwrap().len(),
        1,
        "the replaced version's stored copy is deleted after the update"
    );

    github_commit(
        &mut routes.lock().unwrap(),
        THIRD,
        &[("notes", "Take the tidiest notes.")],
    );
    send(PluginCommand::CheckForUpdate { id: id.clone() });
    until(&client, |event| {
        matches!(event, PluginEvent::UpdateReady { .. })
    })
    .await;
    send(PluginCommand::Remove { id: id.clone() });
    let removed = until(&client, |event| {
        matches!(event, PluginEvent::Removed { .. } | PluginEvent::Error(_))
    })
    .await;
    assert!(
        matches!(removed.last(), Some(Event::Plugins(PluginEvent::Removed { id: gone, title })) if *gone == id && title == "owner/skills"),
        "{removed:?}"
    );
    assert!(
        removed.iter().any(|event| matches!(event, Event::Plugins(PluginEvent::Loaded(library)) if library.installed.is_empty())),
        "the new list arrives before the removal notice"
    );
    assert!(
        !contains_plugin_skill(&removed),
        "a removed plugin's skills leave the library at once, even while it was on"
    );
    assert_eq!(store.load().unwrap().installed, []);
    assert_eq!(
        store.packages.lock().unwrap().len(),
        0,
        "nothing from the plugin is left on this computer"
    );
    send(PluginCommand::ApplyUpdate { id: id.clone() });
    until(&client, |event| matches!(event, PluginEvent::Error(_))).await;
    assert_eq!(
        store.load().unwrap().installed,
        [],
        "an update reviewed before removal cannot bring the plugin back"
    );
    send(PluginCommand::Remove { id: id.clone() });
    let twice = until(&client, |event| matches!(event, PluginEvent::Error(_))).await;
    assert!(
        matches!(twice.last(), Some(Event::Plugins(PluginEvent::Error(text))) if text.contains("no longer installed"))
    );

    send(PluginCommand::AddFromGitHub {
        address: "owner/skills".into(),
    });
    until(&client, |event| matches!(event, PluginEvent::Added { .. })).await;
    assert_eq!(
        store.load().unwrap().installed[0]
            .github
            .as_ref()
            .unwrap()
            .commit,
        THIRD,
        "a removed repository can be added again"
    );
}

/// Stands in for the sealed box (the runtime never names it); counts what it loads.
#[derive(Debug, Default)]
struct CountingEngine {
    loads: Mutex<Vec<bool>>,
}

#[derive(Debug)]
struct Answer;

impl sovra_core::plugins::LoadedCode for Answer {
    fn call(
        &self,
        _: &str,
        _: &serde_json::Value,
        _: Arc<dyn sovra_core::plugins::CodeHost>,
    ) -> Result<String, String> {
        Ok("answered".into())
    }
}

impl sovra_core::plugins::PluginCodeEngine for CountingEngine {
    fn load(
        &self,
        _: &[u8],
        network: bool,
    ) -> Result<Arc<dyn sovra_core::plugins::LoadedCode>, SkillError> {
        self.loads.lock().expect("test fixture").push(network);
        Ok(Arc::new(Answer))
    }
}

fn code_commit(routes: &mut BTreeMap<String, Vec<u8>>, commit: &str, network: &[&str]) {
    routes.insert(
        "/api/repos/owner/dates/commits/main".into(),
        commit.as_bytes().to_vec(),
    );
    let tree = serde_json::json!({"tree": [
        {"path": "sovra-plugin.json", "type": "blob", "size": 400},
        {"path": "plugin.wasm", "type": "blob", "size": 70_000},
        {"path": "src/lib.rs", "type": "blob", "size": 9000},
    ], "truncated": false});
    routes.insert(
        format!("/api/repos/owner/dates/git/trees/{commit}?recursive=1"),
        serde_json::to_vec(&tree).expect("test fixture"),
    );
    let mut manifest: serde_json::Value =
        serde_json::from_slice(include_bytes!("../../../sovra-plugin.json")).expect("test fixture");
    manifest["network"] = serde_json::json!(network);
    routes.insert(
        format!("/raw/owner/dates/{commit}/sovra-plugin.json"),
        serde_json::to_vec(&manifest).expect("test fixture"),
    );
    routes.insert(
        format!("/raw/owner/dates/{commit}/plugin.wasm"),
        include_bytes!("../../../plugin.wasm").to_vec(),
    );
}

#[tokio::test]
async fn a_code_plugin_is_added_off_runs_when_on_and_asks_again_for_new_addresses() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let mut initial = BTreeMap::from([(
        "/api/repos/owner/dates".to_owned(),
        br#"{"default_branch":"main","private":false}"#.to_vec(),
    )]);
    code_commit(&mut initial, FIRST, &[]);
    let routes = Arc::new(Mutex::new(initial));
    serve_github(listener, routes.clone(), Arc::default());
    let store = Arc::new(Store::default());
    let engine = Arc::new(CountingEngine::default());
    let client = sovra_runtime::start_with_services(
        "plugin-code-test",
        AgentConfig::default(),
        RuntimeServices {
            plugins: Some(store.clone()),
            plugin_hosts: sovra_core::plugins::GitHubHosts {
                api: format!("{root}/api"),
                raw: format!("{root}/raw"),
            },
            plugin_code: Some(engine.clone()),
            ..RuntimeServices::default()
        },
    )
    .unwrap();
    until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    let send = |command| client.send(Command::Plugins(command)).unwrap();

    send(PluginCommand::AddFromGitHub {
        address: "owner/dates".into(),
    });
    let added = until(&client, |event| {
        matches!(event, PluginEvent::Added { .. } | PluginEvent::Error(_))
    })
    .await;
    let Some(Event::Plugins(PluginEvent::Added { id, package, .. })) = added.last() else {
        panic!("expected the code plugin to be added: {added:?}");
    };
    let id = id.clone();
    let code = package.code.as_ref().expect("the package carries its code");
    assert_eq!(code.tools.len(), 3);
    assert_eq!(
        code.module_bytes().unwrap(),
        include_bytes!("../../../plugin.wasm")
    );
    assert!(package.description.starts_with("Exact date sums"));
    assert!(
        engine.loads.lock().unwrap().is_empty(),
        "nothing runs while the plugin is off"
    );

    // Turning code on without the approval step is refused, whoever sends it.
    send(PluginCommand::SetEnabled {
        id: id.clone(),
        enabled: true,
    });
    let refused = until(&client, |event| matches!(event, PluginEvent::Error(_))).await;
    assert!(
        refused.iter().any(|event| matches!(event, Event::Plugins(PluginEvent::Error(message)) if message.contains("runs its own code"))),
        "{refused:?}"
    );
    assert!(!store.load().unwrap().installed[0].enabled);
    let seen = store.load().unwrap().installed[0].entry.sha256.clone();

    send(PluginCommand::AllowCode {
        id: id.clone(),
        package_sha256: seen.clone(),
    });
    let on = until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    assert!(
        on.iter().any(|event| matches!(event, Event::Skills(SkillEvent::Loaded { issues, .. }) if issues.is_empty())),
        "{on:?}"
    );
    assert_eq!(
        *engine.loads.lock().unwrap(),
        [false],
        "loaded once, without the internet"
    );

    // The next commit wants to reach a new address: the update says so, and the
    // plugin comes back switched off until the person turns it on again.
    code_commit(&mut routes.lock().unwrap(), SECOND, &["api.example.com"]);
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
    assert_eq!(changes.code.network_added, ["api.example.com"]);
    assert!(changes.code.needs_approval());
    send(PluginCommand::ApplyUpdate { id: id.clone() });
    until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    let updated = store.load().unwrap().installed.remove(0);
    assert_eq!(updated.github.as_ref().unwrap().commit, SECOND);
    assert!(!updated.enabled, "new access needs the person's approval");

    // Approval given for the old package does not switch the update on.
    send(PluginCommand::AllowCode {
        id: id.clone(),
        package_sha256: seen,
    });
    until(&client, |event| matches!(event, PluginEvent::Error(_))).await;
    assert!(!store.load().unwrap().installed[0].enabled);

    send(PluginCommand::AllowCode {
        id: id.clone(),
        package_sha256: updated.entry.sha256.clone(),
    });
    until(&client, |event| matches!(event, PluginEvent::Loaded(_))).await;
    assert_eq!(
        *engine.loads.lock().unwrap(),
        [false, true],
        "turned on again, it loads with the internet it now lists"
    );
    client.send(Command::Shutdown).unwrap();
}

//! Switched-on code plugins as chat tools. The skills worker refreshes the set
//! after every library change; a chat turn takes a snapshot of the offers, and its
//! calls run the plugin's module in the sealed box on a blocking thread.
//!
//! Network access goes through [`Gate`]: the core request checks, then a lookup
//! that refuses this computer and the local network, then a bounded HTTPS request
//! without redirects. Nothing else reaches the module.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use sovra_core::mcp::{RemoteCall, RemoteOutput, RemoteToolCaller, TOOL_FAILED_TEXT, ToolOffer};
use sovra_core::plugins::{
    CODE_API_VERSION, CodeFetch, CodeFetchReply, CodeHost, LoadedCode, MAX_FETCH_REPLY_BYTES,
    PLUGIN_SERVICE_PREFIX, PluginCodeEngine, PluginLibrary, PluginStore, check_fetch,
    public_address,
};

/// Longest a tool call may take, including its internet requests.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// One switched-on plugin whose code loaded.
struct Running {
    /// The package it came from and the module inside, so a refresh that finds the
    /// same package again reuses all of this without reading it from disk.
    package_sha256: String,
    module_sha256: String,
    title: String,
    network: Vec<String>,
    code: Arc<dyn LoadedCode>,
    offers: Vec<ToolOffer>,
}

/// The code plugins chat turns may use. Shared by the skills worker (writer) and
/// chat turns (readers).
#[derive(Default)]
pub(crate) struct CodeTools {
    running: RwLock<HashMap<String, Arc<Running>>>,
    /// Compiled modules by checksum, so turning another plugin on or off does not
    /// recompile every module.
    compiled: RwLock<HashMap<String, Arc<dyn LoadedCode>>>,
    /// One internet client for every plugin request, so connections and TLS setup
    /// are reused instead of rebuilt for each fetch.
    http: OnceLock<reqwest::Client>,
}

impl std::fmt::Debug for CodeTools {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CodeTools")
    }
}

impl CodeTools {
    /// Loads the code of every switched-on plugin that has some. Blocking. A plugin
    /// whose code cannot load is left out and named in the returned issues.
    pub(crate) fn refresh(
        &self,
        library: &PluginLibrary,
        store: &dyn PluginStore,
        engine: Option<&dyn PluginCodeEngine>,
    ) -> Vec<String> {
        let mut issues = Vec::new();
        let mut running = HashMap::new();
        let mut compiled = HashMap::new();
        let previous = self
            .compiled
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let unchanged: HashMap<String, Arc<Running>> = self
            .running
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .map(|plugin| (plugin.package_sha256.clone(), Arc::clone(plugin)))
            .collect();
        for plugin in library
            .installed
            .iter()
            .filter(|plugin| plugin.enabled && plugin.entry.api_version == CODE_API_VERSION)
        {
            if let Some(same) = unchanged.get(&plugin.entry.sha256)
                && let Some(offer) = same.offers.first()
            {
                compiled.insert(same.module_sha256.clone(), Arc::clone(&same.code));
                running.insert(offer.service_id.clone(), Arc::clone(same));
                continue;
            }
            let loaded = store.package(&plugin.entry).and_then(|package| {
                let Some(engine) = engine else {
                    return Err(sovra_core::skills::SkillError(
                        "This copy of Sovra cannot run plugin code.".into(),
                    ));
                };
                let Some(code) = package.code.as_ref() else {
                    return Err(sovra_core::skills::SkillError(
                        "Its package has no code.".into(),
                    ));
                };
                let module = if let Some(module) = previous.get(&code.module_sha256) {
                    Arc::clone(module)
                } else {
                    code.validate()?;
                    engine.load(&code.module_bytes()?, !code.network.is_empty())?
                };
                compiled.insert(code.module_sha256.clone(), Arc::clone(&module));
                Ok(Running {
                    package_sha256: plugin.entry.sha256.clone(),
                    module_sha256: code.module_sha256.clone(),
                    title: package.title.clone(),
                    network: code.network.clone(),
                    code: module,
                    offers: package.tool_offers(),
                })
            });
            match loaded {
                Ok(plugin_code) => {
                    if let Some(offer) = plugin_code.offers.first() {
                        running.insert(offer.service_id.clone(), Arc::new(plugin_code));
                    }
                }
                Err(error) => {
                    tracing::warn!(plugin = %plugin.entry.id, %error, "plugin code not loaded");
                    issues.push(format!(
                        "{}: {error} Its tools are not available. Turn this plugin off or update it.",
                        plugin.entry.title
                    ));
                }
            }
        }
        *self
            .running
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = running;
        *self
            .compiled
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = compiled;
        issues
    }

    pub(crate) fn offers(&self) -> Vec<ToolOffer> {
        let running = self
            .running
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut offers: Vec<ToolOffer> = running
            .values()
            .flat_map(|plugin| plugin.offers.iter().cloned())
            .collect();
        offers.sort_by(|a, b| (&a.service_id, &a.tool.name).cmp(&(&b.service_id, &b.tool.name)));
        offers
    }

    /// The shared internet client, built on first use.
    fn http(&self) -> Result<reqwest::Client, String> {
        if let Some(client) = self.http.get() {
            return Ok(client.clone());
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(FETCH_TIMEOUT)
            .connect_timeout(Duration::from_secs(10))
            .https_only(true)
            // A system proxy would look the address up itself, past PublicOnly.
            .no_proxy()
            .dns_resolver(Arc::new(PublicOnly))
            .user_agent("Sovra plugin")
            .build()
            .map_err(|_| "The plugin's request could not be prepared.".to_owned())?;
        Ok(self.http.get_or_init(|| client).clone())
    }

    fn get(&self, service_id: &str) -> Option<Arc<Running>> {
        self.running
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(service_id)
            .cloned()
    }
}

/// Sends each chat-turn tool call to the right place: a code plugin's sealed box,
/// or a connected service.
#[derive(Clone)]
pub(crate) struct ToolRouter {
    pub(crate) services: Arc<dyn RemoteToolCaller>,
    pub(crate) code: Arc<CodeTools>,
    pub(crate) handle: tokio::runtime::Handle,
}

impl std::fmt::Debug for ToolRouter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ToolRouter")
    }
}

#[async_trait::async_trait]
impl RemoteToolCaller for ToolRouter {
    async fn call(&self, request: RemoteCall) -> Result<RemoteOutput, String> {
        if !request.service_id.starts_with(PLUGIN_SERVICE_PREFIX) {
            return self.services.call(request).await;
        }
        let plugin = self.code.get(&request.service_id).ok_or_else(|| {
            "That plugin is switched off now, so its tools cannot be used.".to_owned()
        })?;
        let client = if plugin.network.is_empty() {
            None
        } else {
            Some(self.code.http()?)
        };
        let gate: Arc<dyn CodeHost> = Arc::new(Gate {
            allowed: plugin.network.clone(),
            client,
            deadline: Instant::now() + CALL_TIMEOUT,
            handle: self.handle.clone(),
        });
        let title = plugin.title.clone();
        let (tool, arguments) = (request.tool, request.arguments);
        let task = tokio::task::spawn_blocking(move || plugin.code.call(&tool, &arguments, gate));
        let text = tokio::time::timeout(CALL_TIMEOUT, task)
            .await
            .map_err(|_| "The plugin took too long to answer.".to_owned())?
            .map_err(|_| TOOL_FAILED_TEXT.to_owned())??;
        Ok(RemoteOutput {
            service_name: title,
            text,
            widget: None,
        })
    }
}

/// The only way plugin code reaches the internet.
struct Gate {
    allowed: Vec<String>,
    /// None when the plugin lists no addresses: its module cannot even import fetch.
    client: Option<reqwest::Client>,
    /// When the chat stops waiting for this call. The sandbox thread can outlive
    /// that wait, so requests after it are refused rather than sent.
    deadline: Instant,
    handle: tokio::runtime::Handle,
}

impl CodeHost for Gate {
    fn fetch(&self, request: CodeFetch) -> Result<CodeFetchReply, String> {
        let url = check_fetch(&request, &self.allowed)?;
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "This plugin may not use the internet.".to_owned())?;
        let left = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| "The plugin ran out of time for internet requests.".to_owned())?;
        // Called on a blocking thread, so waiting here never stalls the runtime.
        self.handle
            .block_on(send(client, url, request, left.min(FETCH_TIMEOUT)))
    }
}

async fn send(
    client: &reqwest::Client,
    url: url::Url,
    request: CodeFetch,
    timeout: Duration,
) -> Result<CodeFetchReply, String> {
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|_| "The plugin used an unknown request method.".to_owned())?;
    let mut builder = client.request(method, url.as_str()).timeout(timeout);
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }
    if !request.body.is_empty() {
        builder = builder.body(request.body);
    }
    let response = builder.send().await.map_err(|_| {
        format!(
            "{} could not be reached.",
            url.host_str().unwrap_or("The address")
        )
    })?;
    let status = response.status().as_u16();
    let body = super::read_limited(response, MAX_FETCH_REPLY_BYTES)
        .await
        .map_err(|_| "The answer from the internet was too large.".to_owned())?;
    Ok(CodeFetchReply {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Looks host names up as usual, then drops every address on this computer or the
/// local network, so a listed name cannot be pointed at a router or local program.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let found: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0))
                .await?
                .filter(|address| public_address(address.ip()))
                .collect();
            if found.is_empty() {
                return Err("that address is not on the public internet".into());
            }
            Ok(Box::new(found.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::{Value, json};
    use sovra_core::plugins::{
        CODE_MANIFEST_PATH, GitHubRepository, InstalledPlugin, PluginEntry, PluginPackage,
        PluginSkill, RepositorySnapshot,
    };
    use sovra_core::skills::SkillError;

    use super::*;

    /// This repository's own module: real bytes the core checks accept.
    const MODULE: &[u8] = include_bytes!("../../../../plugin.wasm");

    fn code_plugin(repository: &str, network: &[&str]) -> (PluginEntry, Vec<u8>, InstalledPlugin) {
        code_plugin_at(repository, network, "Date tools.")
    }

    fn code_plugin_at(
        repository: &str,
        network: &[&str],
        description: &str,
    ) -> (PluginEntry, Vec<u8>, InstalledPlugin) {
        let manifest = json!({
            "description": description,
            "tools": [{"name": "weekday", "title": "Find the weekday",
                       "description": "Use to find a weekday.",
                       "input_schema": {"type": "object"}}],
            "network": network,
        });
        let snapshot = RepositorySnapshot {
            repository: GitHubRepository::parse(repository).unwrap(),
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            description: None,
            skills: Vec::new(),
            manifests: vec![(
                CODE_MANIFEST_PATH.into(),
                serde_json::to_vec(&manifest).unwrap(),
            )],
            module: Some(MODULE.to_vec()),
            left_out: Vec::new(),
        };
        let (entry, bytes, pin) = snapshot.package().unwrap();
        let installed = InstalledPlugin {
            entry: entry.clone(),
            catalog_url: snapshot.repository.web_url(),
            enabled: true,
            github: Some(pin),
        };
        (entry, bytes, installed)
    }

    #[derive(Debug, Default)]
    struct Store {
        packages: Vec<(PluginEntry, Vec<u8>)>,
        reads: AtomicUsize,
    }

    impl PluginStore for Store {
        fn load(&self) -> Result<PluginLibrary, SkillError> {
            unreachable!("refresh is handed the library")
        }
        fn save(&self, _: &PluginLibrary) -> Result<(), SkillError> {
            Ok(())
        }
        fn package(&self, entry: &PluginEntry) -> Result<PluginPackage, SkillError> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            let (_, bytes) = self
                .packages
                .iter()
                .find(|(known, _)| known.sha256 == entry.sha256)
                .ok_or_else(|| SkillError("missing".into()))?;
            entry.verify_package(bytes)
        }
        fn cache(&self, _: &PluginEntry, _: &[u8]) -> Result<(), SkillError> {
            Ok(())
        }
        fn remove_unused(&self, _: &PluginLibrary) -> Result<usize, SkillError> {
            Ok(0)
        }
        fn read_skill_file(&self, _: Option<&str>, _: &str) -> Result<PluginSkill, SkillError> {
            Err(SkillError("unused".into()))
        }
    }

    /// Answers every tool with its name and arguments; counts how often it compiles.
    #[derive(Debug, Default)]
    struct Engine {
        loads: AtomicUsize,
        refuse_network: bool,
    }

    #[derive(Debug)]
    struct Echo;

    impl LoadedCode for Echo {
        fn call(
            &self,
            tool: &str,
            arguments: &Value,
            _: Arc<dyn CodeHost>,
        ) -> Result<String, String> {
            Ok(format!("{tool} {arguments}"))
        }
    }

    impl PluginCodeEngine for Engine {
        fn load(&self, _: &[u8], network: bool) -> Result<Arc<dyn LoadedCode>, SkillError> {
            self.loads.fetch_add(1, Ordering::Relaxed);
            if network && self.refuse_network {
                return Err(SkillError(
                    "The plugin's code cannot run: test refusal.".into(),
                ));
            }
            Ok(Arc::new(Echo))
        }
    }

    #[derive(Debug, Default)]
    struct Services {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl RemoteToolCaller for Services {
        async fn call(&self, request: RemoteCall) -> Result<RemoteOutput, String> {
            self.calls.lock().unwrap().push(request.service_id);
            Ok(RemoteOutput {
                service_name: "Service".into(),
                text: "from a service".into(),
                widget: None,
            })
        }
    }

    fn call(service_id: &str, tool: &str) -> RemoteCall {
        RemoteCall {
            service_id: service_id.into(),
            tool: tool.into(),
            arguments: json!({"date": "2026-12-25"}),
            session_id: "chat".into(),
        }
    }

    #[test]
    fn only_switched_on_code_plugins_load_and_compiled_modules_are_reused() {
        let (on_entry, on_bytes, on) = code_plugin("owner/dates", &[]);
        let (off_entry, off_bytes, mut off) = code_plugin("owner/other", &[]);
        off.enabled = false;
        let store = Store {
            packages: vec![(on_entry, on_bytes), (off_entry, off_bytes)],
            ..Store::default()
        };
        let library = PluginLibrary {
            catalog_url: None,
            installed: vec![on, off],
        };
        let engine = Engine::default();
        let tools = CodeTools::default();
        assert_eq!(
            tools.refresh(&library, &store, Some(&engine)),
            [] as [String; 0]
        );
        let offers = tools.offers();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].service_id, "plugin:io.github.owner.dates");
        assert_eq!(offers[0].tool.name, "weekday");

        tools.refresh(&library, &store, Some(&engine));
        assert_eq!(
            engine.loads.load(Ordering::Relaxed),
            1,
            "the module compiled once"
        );
        assert_eq!(
            store.reads.load(Ordering::Relaxed),
            1,
            "an unchanged package is not read from disk again"
        );
        assert_eq!(tools.offers(), offers);

        // An update with a new package but the same module rereads the package
        // and still does not compile again.
        let (updated_entry, updated_bytes, updated) =
            code_plugin_at("owner/dates", &[], "Date tools, now with better wording.");
        let store = Store {
            packages: vec![(updated_entry, updated_bytes)],
            ..Store::default()
        };
        let mut updated_library = library.clone();
        updated_library.installed[0] = updated;
        let _ = tools.refresh(&updated_library, &store, Some(&engine));
        assert_eq!(store.reads.load(Ordering::Relaxed), 1);
        assert_eq!(engine.loads.load(Ordering::Relaxed), 1);

        let mut all_off = library;
        all_off.installed[0].enabled = false;
        tools.refresh(&all_off, &store, Some(&engine));
        assert_eq!(tools.offers(), [] as [ToolOffer; 0]);
    }

    #[test]
    fn code_that_cannot_load_is_reported_and_left_out() {
        let (entry, bytes, plugin) = code_plugin("owner/weather", &["api.example.com"]);
        let store = Store {
            packages: vec![(entry, bytes)],
            ..Store::default()
        };
        let library = PluginLibrary {
            catalog_url: None,
            installed: vec![plugin],
        };
        let engine = Engine {
            refuse_network: true,
            ..Engine::default()
        };
        let tools = CodeTools::default();
        let issues = tools.refresh(&library, &store, Some(&engine));
        assert_eq!(issues.len(), 1);
        assert!(
            issues[0].starts_with("owner/weather: The plugin's code cannot run"),
            "{issues:?}"
        );
        assert_eq!(tools.offers(), [] as [ToolOffer; 0]);
        let issues = tools.refresh(&library, &store, None);
        assert!(issues[0].contains("cannot run plugin code"), "{issues:?}");
    }

    #[tokio::test]
    async fn calls_go_to_the_plugins_box_or_to_the_connected_service() {
        let (entry, bytes, plugin) = code_plugin("owner/dates", &[]);
        let store = Store {
            packages: vec![(entry, bytes)],
            ..Store::default()
        };
        let mut library = PluginLibrary {
            catalog_url: None,
            installed: vec![plugin],
        };
        let tools = Arc::new(CodeTools::default());
        tools.refresh(&library, &store, Some(&Engine::default()));
        let services = Arc::new(Services::default());
        let router = ToolRouter {
            services: services.clone(),
            code: Arc::clone(&tools),
            handle: tokio::runtime::Handle::current(),
        };
        let answer = router
            .call(call("plugin:io.github.owner.dates", "weekday"))
            .await
            .unwrap();
        assert_eq!(answer.service_name, "owner/dates");
        assert_eq!(answer.text, r#"weekday {"date":"2026-12-25"}"#);
        let answer = router.call(call("svc-1", "search")).await.unwrap();
        assert_eq!(answer.text, "from a service");
        assert_eq!(*services.calls.lock().unwrap(), ["svc-1"]);

        // Switched off between a chat taking its tools and calling one.
        library.installed[0].enabled = false;
        tools.refresh(&library, &store, Some(&Engine::default()));
        let error = router
            .call(call("plugin:io.github.owner.dates", "weekday"))
            .await
            .unwrap_err();
        assert!(error.contains("switched off"), "{error}");
    }

    #[tokio::test]
    async fn the_gate_refuses_unlisted_hosts_and_local_addresses() {
        let handle = tokio::runtime::Handle::current();
        let refused = tokio::task::spawn_blocking(move || {
            let gate = Gate {
                allowed: vec!["api.example.com".into()],
                client: Some(CodeTools::default().http().unwrap()),
                deadline: Instant::now() + CALL_TIMEOUT,
                handle,
            };
            gate.fetch(CodeFetch {
                url: "https://evil.example.net/".into(),
                method: "GET".into(),
                headers: std::collections::BTreeMap::new(),
                body: String::new(),
            })
        })
        .await
        .unwrap();
        assert!(
            refused
                .unwrap_err()
                .contains("only contact api.example.com")
        );

        // A sandbox thread still running after the chat stopped waiting sends nothing.
        let handle = tokio::runtime::Handle::current();
        let late = tokio::task::spawn_blocking(move || {
            Gate {
                allowed: vec!["api.example.com".into()],
                client: Some(CodeTools::default().http().unwrap()),
                deadline: Instant::now(),
                handle,
            }
            .fetch(CodeFetch {
                url: "https://api.example.com/".into(),
                method: "GET".into(),
                headers: std::collections::BTreeMap::new(),
                body: String::new(),
            })
        })
        .await
        .unwrap();
        assert!(late.unwrap_err().contains("ran out of time"));

        let local = reqwest::dns::Resolve::resolve(&PublicOnly, "localhost".parse().unwrap()).await;
        assert!(
            local.is_err(),
            "localhost is never reachable from plugin code"
        );
    }
}

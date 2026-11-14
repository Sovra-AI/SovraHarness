//! Owned Ollama processes. Existing services are reused and never stopped by Sovra.

use sovra_core::models::{ModelConnection, ModelError, ModelReadiness};
use std::{collections::BTreeMap, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    process::{Child, Command},
    sync::Mutex,
    time::{Instant, sleep},
};

/// Shares startup across discovery, downloads and concurrent inference requests.
/// Executable locations are supplied by the composition root, never by a model.
#[derive(Debug)]
pub struct LocalModelServers {
    programs: Vec<PathBuf>,
    children: Mutex<BTreeMap<String, Child>>,
}

impl LocalModelServers {
    pub fn new(ollama_programs: Vec<PathBuf>) -> Self {
        Self {
            programs: ollama_programs,
            children: Mutex::new(BTreeMap::new()),
        }
    }

    async fn probe(client: &reqwest::Client, address: &str) -> Result<bool, ModelError> {
        match client.get(format!("http://{address}/api/version")).send().await {
            Ok(mut response) if response.status().is_success() => {
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|_| ModelError("Could not read Ollama server status.".into()))? {
                    if body.len() + chunk.len() > 4096 {
                        return Err(ModelError("The local address did not return an Ollama server status. Check the port.".into()));
                    }
                    body.extend_from_slice(&chunk);
                }
                if serde_json::from_slice::<serde_json::Value>(&body).ok()
                    .and_then(|value| value["version"].as_str().map(str::to_owned)).is_some() {
                    Ok(true)
                } else {
                    Err(ModelError("The local address is not an Ollama server. Check the port.".into()))
                }
            },
            Ok(response) => Err(ModelError(format!("The local Ollama address returned HTTP {}. Check its port and authentication settings.", response.status().as_u16()))),
            Err(error) if error.is_connect() && !error.is_timeout() => Ok(false),
            Err(_) => Err(ModelError("Ollama is not responding. Check the running server and available memory, then retry.".into())),
        }
    }

    fn launch(&self, address: &str) -> Result<Child, ModelError> {
        for program in &self.programs {
            let mut command = Command::new(program);
            command
                .arg("serve")
                .env("OLLAMA_HOST", address)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            #[cfg(windows)]
            command.creation_flags(0x0800_0000);
            match command.spawn() {
                Ok(child) => return Ok(child),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(_) => return Err(ModelError("Could not start Ollama. Check that Ollama is installed and can run on this computer.".into())),
            }
        }
        Err(ModelError("Ollama is not installed or could not be found. Install it from ollama.com, then retry. Sovra will start its server for you.".into()))
    }
}

#[async_trait::async_trait]
impl ModelReadiness for LocalModelServers {
    async fn ensure_ready(&self, connection: &ModelConnection) -> Result<(), ModelError> {
        let Some(address) = sovra_providers::ollama_bind_address(connection) else {
            return Ok(());
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|_| ModelError("Could not check the local Ollama server.".into()))?;
        // Hold the gate through startup so simultaneous helpers cannot spawn duplicates.
        let mut children = self.children.lock().await;
        if Self::probe(&client, &address).await? {
            return Ok(());
        }
        if let Some(child) = children.get_mut(&address) {
            match child.try_wait() {
                Ok(Some(_)) => { children.remove(&address); },
                _ => return Err(ModelError("The Ollama server started by Sovra stopped responding. Retry shortly or restart Sovra.".into())),
            }
        }
        tracing::info!(%address, "starting local Ollama server");
        let mut child = self.launch(&address)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            // A newly bound listener can still be initializing. A short probe
            // timeout must not kill a healthy process before the startup deadline.
            if tokio::time::timeout_at(deadline, Self::probe(&client, &address))
                .await
                .is_ok_and(|result| result == Ok(true))
            {
                tracing::info!(%address, "local Ollama server is ready");
                // Another service may have won the bind race; an exited child owns nothing.
                if child.try_wait().ok().flatten().is_none() {
                    children.insert(address, child);
                }
                return Ok(());
            }
            if child
                .try_wait()
                .map_err(|_| ModelError("Could not check Ollama startup.".into()))?
                .is_some()
            {
                return Err(ModelError("Ollama exited before its server was ready. Check that the port is free and Ollama can start on this computer.".into()));
            }
            if Instant::now() >= deadline {
                return Err(ModelError("Ollama did not start within 30 seconds. Check the installation and available memory, then retry.".into()));
            }
            sleep(Duration::from_millis(200)).await;
        }
    }
}

#[cfg(test)]
mod tests;

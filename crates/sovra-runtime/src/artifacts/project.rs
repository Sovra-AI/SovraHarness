//! Builds saved frontend source through the existing terminal approval boundary.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::Ordering;

use sovra_core::Event;
use sovra_core::artifacts::{ArtifactEvent, ArtifactKind, WebsiteProject};
use sovra_core::terminal::{CallStatus, TerminalOutcome, TerminalRequest};

use super::{ArtifactRuntime, hosting};

impl ArtifactRuntime {
    pub(super) async fn run_project(&self, id: &str, session: &str) -> ArtifactEvent {
        if !self.inner.building.lock().await.insert(id.to_owned()) {
            return ArtifactEvent::Error(
                "This website is already being built. Check its command in chat.".into(),
            );
        }
        let _ = self
            .inner
            .events
            .send(Event::Artifacts(ArtifactEvent::BuildStatus {
                id: id.to_owned(),
                message: Some("Building website — review the commands in chat.".into()),
            }))
            .await;
        let result = self.build_project(id, session).await;
        self.inner.building.lock().await.remove(id);
        let _ = self
            .inner
            .events
            .send(Event::Artifacts(ArtifactEvent::BuildStatus {
                id: id.to_owned(),
                message: None,
            }))
            .await;
        result.unwrap_or_else(ArtifactEvent::Error)
    }

    async fn build_project(&self, id: &str, session: &str) -> Result<ArtifactEvent, String> {
        let artifact = self
            .get(id)
            .await
            .ok_or("This saved project is no longer available.")?;
        if artifact.kind != ArtifactKind::Project {
            return Err("Choose a saved website project.".into());
        }
        let project = WebsiteProject::parse(&artifact.content)?;
        let terminal = self.inner.terminal.as_ref().ok_or("Enable terminal access in Settings to build framework projects. Node.js and npm must be installed.")?;
        let sequence = self.inner.sequence.fetch_add(1, Ordering::Relaxed);
        // create_dir (not create_dir_all) refuses pre-existing workspaces.
        let path = self.inner.view_dir.with_file_name(format!(
            "sovra-project-{}-{}-{sequence}",
            std::process::id(),
            crate::now().millis()
        ));
        let mut builder = tokio::fs::DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        builder
            .create(&path)
            .await
            .map_err(|e| format!("Couldn't prepare the website folder: {e}"))?;
        self.inner.build_dirs.lock().await.push(path.clone());
        let result = async {
            for (name, content) in &project.files {
                let target = path.join(name);
                if let Some(parent) = target.parent() { tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?; }
                tokio::fs::write(target, content).await.map_err(|e| e.to_string())?;
            }
            let install = if project.files.contains_key("package-lock.json") { "npm ci --ignore-scripts --no-audit --no-fund" } else { "npm install --ignore-scripts --no-audit --no-fund" };
            for (step, command) in [install, "npm run build --ignore-scripts"].into_iter().enumerate() {
                let outcome = terminal.run(TerminalRequest {
                    session_id: session.to_owned(),
                    call_id: format!("website-{sequence}-{}-{step}", crate::now().millis()),
                    command: command.into(),
                    description: format!("Build saved website “{}” (step {}). Build script: {}", artifact.title, step + 1,
                        serde_json::from_str::<serde_json::Value>(&project.files["package.json"]).ok().and_then(|v| v.pointer("/scripts/build").and_then(serde_json::Value::as_str).map(str::to_owned)).unwrap_or_default()),
                    timeout_secs: 300,
                    working_dir: Some(path.to_string_lossy().into_owned()),
                }).await;
                match outcome {
                    TerminalOutcome::Ran { status: CallStatus::Finished { exit_code: Some(0), .. }, .. } => {}
                    TerminalOutcome::Ran { output, .. } => return Err(format!("Website build failed. Your source is saved. Check the command in chat. {output}")),
                    TerminalOutcome::NotRun(_) => return Err("Website build was not run. Check terminal access and the approval in chat. Your source is saved.".into()),
                }
            }
            let files = collect_output(&path.join(&project.output_dir)).await?;
            // Keep deletion/editing ordered with publication of the built snapshot.
            let library = self.library().await;
            if library.as_ref().and_then(|items| items.get(id)).is_none_or(|saved| saved.content != artifact.content) {
                return Err("The saved project changed while building. Build the current version again.".into());
            }
            let mut websites = self.inner.websites.lock().await;
            if websites.len() >= 8 && !websites.contains_key(id) { return Err("Stop a running website before starting another.".into()); }
            let website = hosting::Website::start_files(files, true).await.map_err(|e| format!("Couldn't start the website: {e}"))?;
            let url = website.url.clone();
            if let Some(previous) = websites.insert(id.to_owned(), website) { previous.stop().await; }
            Ok(ArtifactEvent::Deployed { id: id.to_owned(), url })
        }.await;
        if let Err(error) = tokio::fs::remove_dir_all(&path).await {
            tracing::warn!(%error, "could not remove website build directory");
        }
        self.inner
            .build_dirs
            .lock()
            .await
            .retain(|directory| directory != &path);
        result
    }
}

async fn collect_output(root: &Path) -> Result<BTreeMap<String, hosting::Asset>, String> {
    let mut pending = vec![(root.to_path_buf(), String::new())];
    let mut files = BTreeMap::new();
    let mut total = 0_u64;
    let mut entries = 0;
    while let Some((directory, prefix)) = pending.pop() {
        if tokio::fs::symlink_metadata(&directory)
            .await
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Website output cannot contain symbolic links.".into());
        }
        let mut children = tokio::fs::read_dir(directory)
            .await
            .map_err(|e| format!("Couldn't read the build output: {e}"))?;
        while let Some(child) = children.next_entry().await.map_err(|e| e.to_string())? {
            entries += 1;
            if entries > 2048 {
                return Err("The website has too many output files (maximum 2048).".into());
            }
            let name = child
                .file_name()
                .to_str()
                .ok_or("Website file names must be valid text.")?
                .to_owned();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let kind = child.file_type().await.map_err(|e| e.to_string())?;
            if kind.is_dir() {
                pending.push((child.path(), relative));
            } else if kind.is_file() {
                let size = child.metadata().await.map_err(|e| e.to_string())?.len();
                total = total.saturating_add(size);
                if total > 32 * 1024 * 1024 {
                    return Err("The built website is too large (32 MB maximum).".into());
                }
                let bytes = tokio::fs::read(child.path())
                    .await
                    .map_err(|e| e.to_string())?;
                files.insert(
                    relative.clone(),
                    hosting::Asset {
                        bytes,
                        mime: mime(&relative),
                    },
                );
            } else {
                return Err("Website output must contain only regular files and folders.".into());
            }
        }
    }
    if !files.contains_key("index.html") {
        return Err("The build did not produce index.html. Use a static website build and check output_dir (dist, build or out).".into());
    }
    Ok(files)
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests;

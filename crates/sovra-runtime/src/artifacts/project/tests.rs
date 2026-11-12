//! The project pipeline keeps source intact and only hosts successful build output.
use super::*;
use sovra_core::artifacts::{ArtifactDraft, ArtifactSource};
use sovra_core::terminal::{HostOs, HostSystem, ShellKind, TerminalAccess};
use std::sync::Arc;

#[derive(Debug)]
struct BuildTerminal {
    host: HostSystem,
    fail: bool,
}

#[async_trait::async_trait]
impl TerminalAccess for BuildTerminal {
    fn host(&self) -> &HostSystem {
        &self.host
    }
    async fn run(&self, request: TerminalRequest) -> TerminalOutcome {
        assert_eq!(request.session_id, "test-chat");
        let root = std::path::PathBuf::from(request.working_dir.unwrap());
        assert!(root.join("src/App.jsx").is_file());
        if self.fail {
            return TerminalOutcome::NotRun(CallStatus::Declined);
        }
        if request.command == "npm run build --ignore-scripts" {
            tokio::fs::create_dir_all(root.join("dist/assets"))
                .await
                .unwrap();
            tokio::fs::write(
                root.join("dist/index.html"),
                "<script type=module src=/assets/app.js></script>",
            )
            .await
            .unwrap();
            tokio::fs::write(
                root.join("dist/assets/app.js"),
                "document.body.textContent = 'Built app'",
            )
            .await
            .unwrap();
        } else {
            assert_eq!(
                request.command,
                "npm install --ignore-scripts --no-audit --no-fund"
            );
        }
        TerminalOutcome::Ran {
            status: CallStatus::Finished {
                exit_code: Some(0),
                duration_ms: 1,
            },
            output: String::new(),
            total_lines: 0,
        }
    }
}

#[tokio::test]
async fn projects_build_assets_and_preserve_source_on_denied_commands() {
    for fail in [false, true] {
        let (events, _) = async_channel::unbounded();
        let terminal = BuildTerminal {
            fail,
            host: HostSystem {
                os: HostOs::Linux,
                os_version: String::new(),
                arch: "test".into(),
                shell: ShellKind::Sh,
                shell_path: "/bin/sh".into(),
                working_dir: String::new(),
            },
        };
        let runtime = ArtifactRuntime::new(
            None,
            events,
            std::env::temp_dir().join("sovra-project-test"),
            Some(Arc::new(terminal)),
        );
        let source = serde_json::json!({"files": {"package.json": "{\"scripts\":{\"build\":\"vite build\"}}", "src/App.jsx": "export default () => <h1>Built app</h1>"}}).to_string();
        let artifact = runtime
            .save(ArtifactDraft {
                title: "App".into(),
                kind: ArtifactKind::Project,
                content: source.clone(),
                language: "website-project".into(),
                source: ArtifactSource::default(),
            })
            .await
            .unwrap();
        let event = runtime.run_project(&artifact.id, "test-chat").await;
        if fail {
            assert!(matches!(event, ArtifactEvent::Error(_)));
            assert!(runtime.inner.websites.lock().await.is_empty());
        } else {
            let ArtifactEvent::Deployed { url, .. } = event else {
                panic!("{event:?}")
            };
            let response = reqwest::get(format!("{url}assets/app.js")).await.unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(
                response.headers()["content-type"],
                "text/javascript; charset=utf-8"
            );
            assert!(response.text().await.unwrap().contains("Built app"));
        }
        assert_eq!(runtime.get(&artifact.id).await.unwrap().content, source);
        assert!(runtime.inner.build_dirs.lock().await.is_empty());
        assert!(runtime.inner.building.lock().await.is_empty());
        runtime.stop().await;
    }
}

#[tokio::test]
async fn output_requires_index_and_rejects_symlinks() {
    let root = std::env::temp_dir().join(format!("sovra-output-test-{}", std::process::id()));
    tokio::fs::create_dir_all(&root).await.unwrap();
    assert!(collect_output(&root).await.is_err());
    #[cfg(unix)]
    {
        tokio::fs::symlink("/etc/passwd", root.join("index.html"))
            .await
            .unwrap();
        assert!(collect_output(&root).await.is_err());
    }
    tokio::fs::remove_dir_all(&root).await.unwrap();
}

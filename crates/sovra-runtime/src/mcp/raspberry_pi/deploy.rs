//! `OpenCode` produces source; only the selected Pi runs it.
use super::{health, process};
use sovra_core::raspberry_pi::{Family, PiProject, PiSetup};
use std::path::PathBuf;

/// Executable locations are supplied once by the composition root.
#[derive(Clone)]
pub struct PiPrograms {
    pub mpremote: PathBuf,
    pub credentials: Option<std::sync::Arc<dyn sovra_providers::CredentialStore>>,
}

impl std::fmt::Debug for PiPrograms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PiPrograms")
            .field("mpremote", &self.mpremote)
            .finish_non_exhaustive()
    }
}

pub(super) async fn deliver(
    setup: &PiSetup,
    programs: &PiPrograms,
    project: &PiProject,
    pico_identity: &tokio::sync::Mutex<Option<super::pico::Identity>>,
) -> Result<String, String> {
    let board = setup.validate()?;
    project.validate(board)?;
    if board.family == Family::Pico {
        let saved = pico_identity.lock().await;
        let identity = saved
            .as_ref()
            .ok_or("Connect your Pico from this chat before deploying.")?;
        let folder = tokio::task::spawn_blocking(tempfile::tempdir)
            .await
            .map_err(|_| "Couldn't prepare the source folder.")?
            .map_err(|_| "Couldn't prepare the source folder.")?;
        super::pico::deploy(&programs.mpremote, identity, project, folder.path()).await
    } else {
        let model = health::model(setup, programs).await?;
        deploy(setup, programs, &model, project).await
    }
}

pub(super) async fn deploy(
    setup: &PiSetup,
    programs: &PiPrograms,
    model: &str,
    project: &PiProject,
) -> Result<String, String> {
    let board = setup.validate()?;
    project.validate(board)?;
    if !board.matches_device_model(model) {
        return Err(
            "The Pi model does not match this chat. Choose the model reported by your device."
                .into(),
        );
    }
    let payload = serde_json::to_vec(&serde_json::json!({"model":model,"project":project}))
        .map_err(|_| "Couldn't prepare the project.")?;
    let hex = payload.iter().fold(String::new(), |mut hex, byte| {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    let script = format!(
        "import json\nPAYLOAD=json.loads(bytes.fromhex('{hex}'))\n{}",
        include_str!("deploy.py")
    );
    let output = match super::access::run_saved(
        programs.credentials.as_ref(),
        setup,
        script.as_bytes(),
        45,
        16384,
    )
    .await?
    {
        Some(output) => output,
        None => process::run(process::ssh(setup), script.as_bytes(), 45, 16384).await?,
    };
    deployment_report(&output, &setup.host)
}

fn deployment_report(output: &str, host: &str) -> Result<String, String> {
    let report: serde_json::Value = serde_json::from_str(output)
        .map_err(|_| "The Pi did not confirm deployment. Check the device before trying again.")?;
    if report.get("error").is_some() {
        return Err("The Pi couldn't save or start the project. Check free space and the required Python or Node installation.".into());
    }
    let path = report["directory"]
        .as_str()
        .filter(|path| path.len() < 512 && !path.chars().any(char::is_control))
        .ok_or("The deployment receipt was incomplete.")?;
    let running = report["running"]
        .as_bool()
        .ok_or("The deployment receipt was incomplete.")?;
    if !running && report["exit_code"] != 0 {
        return Err(format!(
            "Files reached the Pi, but the program failed to start. Its log is in {path}/sovra-output.log. Earlier projects were kept."
        ));
    }
    let status = if running {
        "Started on your Pi (still running at the one-second check)"
    } else {
        "Ran on your Pi and finished successfully"
    };
    let site = if running && report["kind"] == "website" {
        format!("\nOpen http://{host}:8080 on the same network.")
    } else {
        String::new()
    };
    Ok(format!(
        "{status}.\nSaved in {path}.{site}\nEarlier projects were kept. This check does not verify the program's physical effects."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raspberry_pi_deployment_requires_an_actual_receipt() {
        assert!(deployment_report("{}", "pi.local").is_err());
        assert!(
            deployment_report(
                r#"{"directory":"/home/test/project","running":false,"exit_code":1}"#,
                "pi.local"
            )
            .is_err()
        );
        let done = deployment_report(
            r#"{"directory":"/home/test/project","running":false,"exit_code":0}"#,
            "pi.local",
        )
        .unwrap();
        assert!(done.contains("finished successfully"));
    }
    #[tokio::test]
    async fn raspberry_pi_receiver_writes_and_runs_the_validated_bundle_in_a_new_folder() {
        let directory = tempfile::tempdir().unwrap();
        let project = PiProject {
            kind: sovra_core::raspberry_pi::ProjectKind::Python,
            files: std::collections::BTreeMap::from([(
                "main.py".into(),
                "from pathlib import Path\nPath('ran.txt').write_text('worked')\n".into(),
            )]),
        };
        project
            .validate(sovra_core::raspberry_pi::board("pi-5").unwrap())
            .unwrap();
        let payload =
            serde_json::json!({"model":"Raspberry Pi 5 Model B Rev 1.0","project":project});
        let source = include_str!("deploy.py").split("\ntry:\n").next().unwrap();
        let script = format!(
            "{source}\npayload=json.loads({})\ndeploy(payload, 'Raspberry Pi 5 Model B Rev 1.0', pathlib.Path({}))\n",
            serde_json::to_string(&payload.to_string()).unwrap(),
            serde_json::to_string(directory.path().to_str().unwrap()).unwrap()
        );
        let mut command = tokio::process::Command::new("python3");
        command.args(["-I", "-"]);
        let output = process::run(command, script.as_bytes(), 10, 16384)
            .await
            .unwrap();
        assert!(
            deployment_report(&output, "fixture.invalid")
                .unwrap()
                .contains("finished successfully")
        );
        let receipt: serde_json::Value = serde_json::from_str(&output).unwrap();
        let release = std::path::Path::new(receipt["directory"].as_str().unwrap());
        assert!(release.starts_with(directory.path()));
        assert_eq!(
            tokio::fs::read_to_string(release.join("ran.txt"))
                .await
                .unwrap(),
            "worked"
        );
        assert_eq!(
            tokio::fs::read_to_string(release.join("main.py"))
                .await
                .unwrap(),
            project.files["main.py"]
        );
    }
}

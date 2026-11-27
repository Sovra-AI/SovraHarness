//! Owned child processes with bounded output, input and deadlines.
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

pub(super) async fn run(
    mut command: tokio::process::Command,
    input: &[u8],
    seconds: u64,
    limit: u64,
) -> Result<String, String> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::terminal::own_process_group(&mut command);
    let mut child = command.spawn().map_err(|_| "Couldn't start the required tool. Install OpenSSH or mpremote as indicated in the Pi setup guide.".to_owned())?;
    let _group = crate::terminal::GroupGuard::new(child.id());
    let mut stdin = child.stdin.take().ok_or("Couldn't open tool input.")?;
    let stdout = child.stdout.take().ok_or("Couldn't open tool output.")?;
    let stderr = child.stderr.take().ok_or("Couldn't open tool errors.")?;
    let result = tokio::time::timeout(Duration::from_secs(seconds), async {
        let write = async move {
            stdin.write_all(input).await?;
            stdin.shutdown().await
        };
        let read = async {
            let mut bytes = Vec::new();
            stdout.take(limit + 1).read_to_end(&mut bytes).await?;
            Ok::<_, std::io::Error>(bytes)
        };
        let errors = async {
            let mut bytes = Vec::new();
            stderr.take(16385).read_to_end(&mut bytes).await?;
            Ok::<_, std::io::Error>(bytes)
        };
        tokio::try_join!(write, read, errors, child.wait())
    })
    .await;
    let ((), output, errors, status) = match result {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => {
            return Err("The tool stopped before it finished. Nothing is marked deployed.".into());
        }
        Err(_) => {
            let _ = child.kill().await;
            return Err(
                "The operation timed out. Its result is unknown; check the Pi before trying again."
                    .into(),
            );
        }
    };
    if output.len() as u64 > limit || errors.len() > 16384 {
        return Err("The tool returned too much output. The operation was stopped.".into());
    }
    if !status.success() {
        return Err("The tool reported a failure. Check its installation, sign-in and the device connection. Nothing is marked deployed.".into());
    }
    String::from_utf8(output).map_err(|_| "The tool returned unreadable text.".into())
}

pub(super) fn ssh(setup: &sovra_core::raspberry_pi::PiSetup) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("ssh");
    command.args([
        "-F",
        "none",
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ClearAllForwardings=yes",
        "-o",
        "PermitLocalCommand=no",
        "-o",
        "ConnectTimeout=8",
        "-l",
        &setup.user,
        &setup.host,
        "python3 -I -",
    ]);
    command
}

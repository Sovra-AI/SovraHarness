//! Finding out what computer commands run on, once at start-up.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use sovra_core::terminal::{
    HostOs, HostSystem, ShellKind, choose_shell, os_release_name, windows_version_name,
};

use super::TerminalConfig;

/// System probes are tiny; anything slower is broken and must not delay start-up.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// A login shell sourcing a heavy profile (nvm, conda) can take a few seconds.
const LOGIN_ENV_TIMEOUT: Duration = Duration::from_secs(10);
const ENV_MARKER: &str = "__SOVRA_LOGIN_ENV__";

pub(super) async fn detect(config: &TerminalConfig) -> HostSystem {
    let os = HostOs::current();
    let login_shell = config.login_shell.clone();
    let search_path = config.search_path.clone();
    let (shell, shell_path) = tokio::task::spawn_blocking(move || {
        choose_shell(os, login_shell.as_deref(), |name| {
            find_program(name, search_path.as_deref())
        })
    })
    .await
    .unwrap_or_else(|_| fallback_shell(os));
    let os_version = os_version(os)
        .await
        .unwrap_or_else(|| os.label().to_owned());
    HostSystem {
        os,
        os_version,
        arch: std::env::consts::ARCH.to_owned(),
        shell,
        shell_path,
        working_dir: config.working_dir.display().to_string(),
    }
}

fn fallback_shell(os: HostOs) -> (ShellKind, String) {
    match os {
        HostOs::Windows => (ShellKind::PowerShell, "powershell.exe".to_owned()),
        HostOs::Linux | HostOs::MacOs | HostOs::OtherUnix => (ShellKind::Sh, "/bin/sh".to_owned()),
    }
}

/// Looks `name` up on `PATH` (with `.exe` on Windows).
fn find_program(name: &str, search_path: Option<&OsStr>) -> Option<String> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::env::split_paths(search_path?)
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
        .map(|found| found.display().to_string())
}

async fn probe(program: &str, args: &[&str]) -> Option<String> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    super::process::hide_window(&mut command);
    let output = tokio::time::timeout(PROBE_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|text| !text.is_empty())
}

async fn os_version(os: HostOs) -> Option<String> {
    match os {
        HostOs::Linux => {
            for path in ["/etc/os-release", "/usr/lib/os-release"] {
                if let Ok(text) = tokio::fs::read_to_string(path).await
                    && let Some(name) = os_release_name(&text)
                {
                    return Some(name);
                }
            }
            probe("uname", &["-sr"]).await
        }
        HostOs::MacOs => probe("sw_vers", &["-productVersion"])
            .await
            .map(|version| format!("macOS {version}")),
        HostOs::Windows => probe("cmd", &["/D", "/C", "ver"])
            .await
            .and_then(|text| windows_version_name(&text)),
        HostOs::OtherUnix => probe("uname", &["-sr"]).await,
    }
}

/// The environment a login shell would have. Apps started from a desktop launcher
/// miss `PATH` entries added in shell profiles (Homebrew, nvm, cargo), so commands
/// like `npm` would fail; editors such as VS Code resolve the same way. Empty on
/// Windows (GUI apps get the full environment there) or when the shell misbehaves.
pub(super) async fn login_environment(host: &HostSystem) -> Vec<(OsString, OsString)> {
    let flags: &[&str] = match host.shell {
        ShellKind::Bash | ShellKind::Zsh => &["-l", "-i", "-c"],
        ShellKind::Sh => &["-l", "-c"],
        ShellKind::PowerShell => return Vec::new(),
    };
    let mut command = tokio::process::Command::new(&host.shell_path);
    command
        .args(flags)
        .arg(format!("printf '%s' {ENV_MARKER}; command env -0"))
        .current_dir(Path::new(&host.working_dir))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    super::process::own_group(&mut command);
    match tokio::time::timeout(LOGIN_ENV_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => {
            let env = parse_login_env(&output.stdout);
            tracing::debug!(variables = env.len(), "resolved login shell environment");
            env
        }
        Ok(Err(error)) => {
            tracing::warn!(%error, "could not read the login shell environment");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!("login shell environment timed out; using the app's own");
            Vec::new()
        }
    }
}

/// Variables describing the probe shell itself, not the person's setup.
const SHELL_STATE: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_", "PS1", "PS2", "TERM"];

fn parse_login_env(stdout: &[u8]) -> Vec<(OsString, OsString)> {
    let Some(start) = stdout
        .windows(ENV_MARKER.len())
        .position(|window| window == ENV_MARKER.as_bytes())
    else {
        return Vec::new();
    };
    stdout[start + ENV_MARKER.len()..]
        .split(|&byte| byte == 0)
        .filter_map(|entry| {
            let split = entry.iter().position(|&byte| byte == b'=')?;
            let (key, value) = (&entry[..split], &entry[split + 1..]);
            let key = std::str::from_utf8(key).ok()?;
            let valid = !key.is_empty()
                && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && !SHELL_STATE.contains(&key);
            valid.then(|| (OsString::from(key), bytes_to_os(value)))
        })
        .collect()
}

#[cfg(unix)]
fn bytes_to_os(bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec())
}

#[cfg(not(unix))]
fn bytes_to_os(bytes: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_env_is_read_after_the_marker_only() {
        let mut stdout = b"Welcome back! PATH=/fake\n".to_vec();
        stdout.extend_from_slice(ENV_MARKER.as_bytes());
        stdout.extend_from_slice(
            b"PATH=/opt/homebrew/bin:/usr/bin\0HOME=/Users/ada\0SHLVL=2\0MULTI=a\nb\0bad key=x\0\0",
        );
        let env = parse_login_env(&stdout);
        let keys: Vec<&str> = env.iter().filter_map(|(key, _)| key.to_str()).collect();
        assert_eq!(keys, ["PATH", "HOME", "MULTI"]);
        assert_eq!(env[0].1, OsString::from("/opt/homebrew/bin:/usr/bin"));
        assert_eq!(env[2].1, OsString::from("a\nb"));
        assert_eq!(parse_login_env(b"no marker"), []);
    }

    #[test]
    fn programs_are_found_on_the_search_path() {
        let dir = std::env::temp_dir().join(format!("sovra-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let name = if cfg!(windows) { "tool.exe" } else { "tool" };
        std::fs::write(dir.join(name), b"").expect("fake program");
        let path = std::env::join_paths([Path::new("/definitely/missing"), &dir]).expect("path");
        let found = find_program("tool", Some(&path)).expect("found");
        assert!(found.ends_with(name));
        assert_eq!(find_program("absent", Some(&path)), None);
        assert_eq!(find_program("tool", None), None);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[tokio::test]
    async fn detection_describes_this_computer() {
        let config = TerminalConfig {
            settings_path: std::env::temp_dir().join("unused.json"),
            working_dir: std::env::temp_dir(),
            login_shell: None,
            search_path: std::env::var_os("PATH"),
            use_login_env: false,
        };
        let host = detect(&config).await;
        assert_eq!(host.os, HostOs::current());
        assert_eq!(host.arch, std::env::consts::ARCH);
        assert_ne!(host.os_version, "");
        assert_ne!(host.shell_path, "");
    }
}

//! The GitHub connection through the real runtime, with a scripted connector.

#![allow(
    clippy::unwrap_used,
    reason = "integration tests fail loudly by design"
)]

use std::sync::Arc;
use std::sync::Mutex;

use sovra_core::github::{
    DeviceCode, DevicePoll, GitHubAccount, GitHubCommand, GitHubConnector, GitHubError,
    GitHubEvent, GitHubKey, GitHubRequest, GitHubState,
};
use sovra_core::{AgentConfig, Command, Event};
use sovra_runtime::{Client, RuntimeServices};

#[derive(Debug, Default)]
struct Script {
    saved: Option<GitHubAccount>,
    polls: usize,
    /// Checks that answer "still waiting" before the person presses Authorize.
    waits: usize,
}

#[derive(Debug, Default)]
struct FakeGitHub(Mutex<Script>);

fn octocat() -> GitHubAccount {
    GitHubAccount {
        login: "octocat".into(),
        name: "Mona".into(),
    }
}

#[async_trait::async_trait]
impl GitHubConnector for FakeGitHub {
    fn device_sign_in(&self) -> bool {
        true
    }
    fn restore(&self) -> Result<Option<GitHubAccount>, GitHubError> {
        Ok(self.0.lock().unwrap().saved.clone())
    }
    async fn start_device_sign_in(&self) -> Result<DeviceCode, GitHubError> {
        Ok(DeviceCode {
            device_code: "device".into(),
            user_code: "WDJB-MJHT".into(),
            verification_uri: "https://github.com/login/device".into(),
            interval_secs: 0,
            expires_in_secs: 900,
        })
    }
    async fn poll_device_sign_in(&self, _: &DeviceCode) -> Result<DevicePoll, GitHubError> {
        let mut script = self.0.lock().unwrap();
        script.polls += 1;
        if script.polls <= script.waits {
            Ok(DevicePoll::Waiting)
        } else {
            Ok(DevicePoll::Done(octocat()))
        }
    }
    async fn check_key(&self, key: &GitHubKey) -> Result<GitHubAccount, GitHubError> {
        if key.as_str() == "ghp_good" {
            Ok(octocat())
        } else {
            Err(GitHubError::Failed("GitHub didn't accept that key.".into()))
        }
    }
    fn save(&self, account: &GitHubAccount) -> Result<(), GitHubError> {
        self.0.lock().unwrap().saved = Some(account.clone());
        Ok(())
    }
    fn forget(&self) -> Result<(), GitHubError> {
        self.0.lock().unwrap().saved = None;
        Ok(())
    }
    async fn call(&self, _: GitHubRequest) -> Result<serde_json::Value, GitHubError> {
        Ok(serde_json::Value::Null)
    }
}

fn start(github: Arc<FakeGitHub>) -> Client {
    sovra_runtime::start_with_services(
        "test",
        AgentConfig::default(),
        RuntimeServices {
            github: Some(github),
            ..RuntimeServices::default()
        },
    )
    .unwrap()
}

/// The next GitHub event, skipping other events.
fn next(client: &Client) -> GitHubEvent {
    loop {
        if let Event::GitHub(event) = client.events().recv_blocking().unwrap() {
            return event;
        }
    }
}

/// The next GitHub state change, skipping other events.
fn next_state(client: &Client) -> GitHubState {
    loop {
        if let GitHubEvent::Changed {
            state,
            device_sign_in,
        } = next(client)
        {
            assert!(device_sign_in);
            return state;
        }
    }
}

fn send(client: &Client, command: GitHubCommand) {
    client.send(Command::GitHub(command)).unwrap();
}

#[test]
fn connect_on_githubs_page_then_disconnect() {
    let github = Arc::new(FakeGitHub::default());
    let client = start(github.clone());
    assert_eq!(
        next_state(&client),
        GitHubState::NotConnected,
        "loads on start"
    );

    send(&client, GitHubCommand::Connect);
    assert_eq!(
        next_state(&client),
        GitHubState::Connecting {
            code: "WDJB-MJHT".into(),
            url: "https://github.com/login/device".into(),
        }
    );
    assert_eq!(
        next(&client),
        GitHubEvent::OpenSignIn {
            url: "https://github.com/login/device".into(),
            code: "WDJB-MJHT".into(),
        },
        "the browser opens with the code to type"
    );
    assert_eq!(next_state(&client), GitHubState::Connected(octocat()));
    assert_eq!(
        github.0.lock().unwrap().saved,
        Some(octocat()),
        "kept for next time"
    );

    send(&client, GitHubCommand::Disconnect);
    assert_eq!(next_state(&client), GitHubState::NotConnected);
    assert_eq!(github.0.lock().unwrap().saved, None, "forgotten");
}

#[test]
fn cancel_stops_waiting_and_nothing_connects_later() {
    let github = Arc::new(FakeGitHub::default());
    github.0.lock().unwrap().waits = usize::MAX;
    let client = start(github.clone());
    assert_eq!(next_state(&client), GitHubState::NotConnected);

    send(&client, GitHubCommand::Connect);
    assert!(matches!(
        next_state(&client),
        GitHubState::Connecting { .. }
    ));
    send(&client, GitHubCommand::Cancel);
    assert_eq!(next_state(&client), GitHubState::NotConnected);
    assert_eq!(github.0.lock().unwrap().saved, None);
}

#[test]
fn a_pasted_key_is_checked_and_a_bad_one_says_why() {
    let github = Arc::new(FakeGitHub::default());
    let client = start(github.clone());
    assert_eq!(next_state(&client), GitHubState::NotConnected);

    send(
        &client,
        GitHubCommand::UseKey(GitHubKey::new("nope").unwrap()),
    );
    assert_eq!(next_state(&client), GitHubState::CheckingKey);
    assert_eq!(next_state(&client), GitHubState::NotConnected);
    assert_eq!(
        next(&client),
        GitHubEvent::Notice("GitHub didn't accept that key.".into())
    );

    send(
        &client,
        GitHubCommand::UseKey(GitHubKey::new(" ghp_good ").unwrap()),
    );
    assert_eq!(next_state(&client), GitHubState::CheckingKey);
    assert_eq!(next_state(&client), GitHubState::Connected(octocat()));
    assert_eq!(github.0.lock().unwrap().saved, Some(octocat()));
}

#[test]
fn a_saved_account_is_connected_on_start() {
    let github = Arc::new(FakeGitHub::default());
    github.0.lock().unwrap().saved = Some(octocat());
    let client = start(github);
    assert_eq!(next_state(&client), GitHubState::Connected(octocat()));
}

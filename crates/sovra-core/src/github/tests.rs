use super::*;

fn account() -> GitHubAccount {
    GitHubAccount {
        login: "octocat".into(),
        name: "Mona".into(),
    }
}

fn changed(state: GitHubState) -> GitHubEvent {
    GitHubEvent::Changed {
        state,
        device_sign_in: true,
    }
}

#[test]
fn repo_names_accept_owner_name_and_links_only() {
    let parsed = RepoName::parse(" https://github.com/octocat/Hello-World.git/ ").expect("link");
    assert_eq!(parsed.full(), "octocat/Hello-World");
    assert_eq!(
        RepoName::parse("rust-lang/rust/tree/master")
            .expect("deeper link")
            .full(),
        "rust-lang/rust"
    );
    for bad in [
        "",
        "octocat",
        "octo cat/x",
        "octocat/..",
        "../etc/passwd",
        "a/b?c",
        "a/b#c",
        "a/%2e%2e",
    ] {
        assert!(RepoName::parse(bad).is_err(), "{bad} should be refused");
    }
}

#[test]
fn paths_and_references_cannot_escape_the_repository() {
    assert_eq!(clean_repo_path("/src/lib.rs").as_deref(), Ok("src/lib.rs"));
    assert_eq!(clean_repo_path("  ").as_deref(), Ok(""));
    for bad in ["src/../secret", "./a", "a//b", "a\\b", "a\nb"] {
        assert!(clean_repo_path(bad).is_err(), "{bad:?} should be refused");
    }
    assert_eq!(clean_reference(None), Ok(None));
    assert_eq!(clean_reference(Some(" ")), Ok(None));
    assert_eq!(
        clean_reference(Some("feature/new-login")),
        Ok(Some("feature/new-login".into()))
    );
    for bad in ["a..b", "/main", "main?x=1", "a b"] {
        assert!(
            clean_reference(Some(bad)).is_err(),
            "{bad} should be refused"
        );
    }
}

#[test]
fn searches_say_issues_or_pull_requests_once() {
    assert_eq!(
        search_query("repo:a/b is:open", SearchKind::Issues),
        "repo:a/b is:open is:issue"
    );
    assert_eq!(search_query("", SearchKind::PullRequests), "is:pr");
    assert_eq!(
        search_query("author:@me IS:PR", SearchKind::Issues),
        "author:@me IS:PR"
    );
    assert_eq!(
        search_query("type:pr review-requested:@me", SearchKind::Issues),
        "type:pr review-requested:@me"
    );
}

#[test]
fn only_issues_and_comments_change_things() {
    let repo = RepoName::parse("a/b").expect("repo");
    assert!(
        GitHubRequest::Comment {
            repo: repo.clone(),
            number: 1,
            body: "Thanks".into()
        }
        .changes_things()
    );
    assert!(
        !GitHubRequest::ReadFile {
            repo,
            path: String::new(),
            reference: None
        }
        .changes_things()
    );
}

#[test]
fn device_answers_and_waits_follow_github() {
    assert_eq!(
        device_answer("authorization_pending"),
        DeviceAnswer::Pending
    );
    assert_eq!(device_answer("slow_down"), DeviceAnswer::SlowDown);
    assert_eq!(device_answer("expired_token"), DeviceAnswer::Expired);
    assert_eq!(device_answer("access_denied"), DeviceAnswer::Denied);
    assert_eq!(device_answer("device_flow_disabled"), DeviceAnswer::Failed);
    assert_eq!(poll_interval_secs(0, 0), 5);
    assert_eq!(poll_interval_secs(5, 2), 15);
    assert_eq!(poll_interval_secs(500, 0), 60);
}

#[test]
fn keys_are_trimmed_and_never_printed() {
    assert!(GitHubKey::new("   ").is_none());
    let key = GitHubKey::new(" ghp_secret ").expect("key");
    assert_eq!(key.as_str(), "ghp_secret");
    assert!(!format!("{:?}", GitHubCommand::UseKey(key)).contains("ghp_secret"));
    let device = DeviceCode {
        device_code: "device-secret".into(),
        user_code: "WDJB-MJHT".into(),
        verification_uri: "https://github.com/login/device".into(),
        interval_secs: 5,
        expires_in_secs: 900,
    };
    let shown = format!("{device:?}");
    assert!(shown.contains("WDJB-MJHT") && !shown.contains("device-secret"));
}

#[test]
fn connection_folds_plain_notices() {
    let mut connection = GitHubConnection::default();
    connection.apply(&changed(GitHubState::NotConnected));
    assert!(connection.device_sign_in);
    assert_eq!(connection.notice, None);
    connection.apply(&changed(GitHubState::Connecting {
        code: "WDJB-MJHT".into(),
        url: "https://github.com/login/device".into(),
    }));
    connection.apply(&changed(GitHubState::Connected(account())));
    assert_eq!(
        connection.notice.as_deref(),
        Some("GitHub is connected as @octocat. Sovra can use it now.")
    );
    connection.apply(&changed(GitHubState::NotConnected));
    assert_eq!(
        connection.notice.as_deref(),
        Some("GitHub is disconnected. Sovra can no longer use it.")
    );
    connection.apply(&GitHubEvent::Notice("That key didn't work.".into()));
    connection.apply(&changed(GitHubState::NotConnected));
    assert_eq!(connection.notice.as_deref(), Some("That key didn't work."));
    connection.apply(&changed(GitHubState::CheckingKey));
    assert_eq!(connection.notice, None);
}

#[test]
fn accounts_show_a_name_and_an_initial() {
    assert_eq!(account().display_name(), "Mona");
    assert_eq!(account().initial(), "M");
    let bare = GitHubAccount {
        login: "octocat".into(),
        name: " ".into(),
    };
    assert_eq!(bare.display_name(), "octocat");
    assert_eq!(bare.initial(), "O");
    assert_eq!(
        GitHubState::NeedsSignIn(account()).account(),
        Some(&account())
    );
    assert_eq!(GitHubState::CheckingKey.account(), None);
}

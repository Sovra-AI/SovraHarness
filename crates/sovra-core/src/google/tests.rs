use std::collections::BTreeSet;

use super::*;

fn account(granted: &[GoogleService]) -> GoogleAccount {
    GoogleAccount {
        email: "sam@example.com".into(),
        name: "Sam Lee".into(),
        granted: granted.iter().copied().collect(),
        turned_off: BTreeSet::new(),
    }
}

#[test]
fn every_service_has_one_row_with_scopes_and_plain_words() {
    for (index, service) in GoogleService::ALL.into_iter().enumerate() {
        let info = service_info(service);
        assert_eq!(info.service, service);
        assert_eq!(SERVICES[index].service, service, "table order matches ALL");
        assert_ne!(info.scopes, [] as [&str; 0]);
        assert!(info.explanation.ends_with('.'));
        assert!(
            !info.explanation.to_lowercase().contains("oauth"),
            "no jargon for people"
        );
    }
}

#[test]
fn scopes_start_with_identity_and_never_repeat() {
    let scopes = scopes_for(&[
        GoogleService::Mail,
        GoogleService::Mail,
        GoogleService::Tasks,
    ]);
    assert_eq!(&scopes[..3], &GOOGLE_IDENTITY_SCOPES);
    let unique: BTreeSet<_> = scopes.iter().collect();
    assert_eq!(unique.len(), scopes.len());
    assert!(scopes.contains(&"https://www.googleapis.com/auth/gmail.compose"));
    assert!(scopes.contains(&"https://www.googleapis.com/auth/tasks"));
    assert!(!scopes.iter().any(|s| s.contains("calendar")));
}

#[test]
fn mail_never_asks_for_permission_to_send_or_delete() {
    let mail = service_info(GoogleService::Mail).scopes;
    assert!(!mail.iter().any(|s| s.ends_with("gmail.send")
        || s.ends_with("gmail.modify")
        || *s == "https://mail.google.com/"));
}

#[test]
fn a_service_counts_only_when_every_scope_came_back() {
    // The person unticked "drafts" on Google's consent page.
    let granted = granted_services(
        "openid https://www.googleapis.com/auth/userinfo.email \
         https://www.googleapis.com/auth/gmail.readonly \
         https://www.googleapis.com/auth/calendar.events",
    );
    assert_eq!(granted, BTreeSet::from([GoogleService::Calendar]));
    assert!(granted_services("").is_empty());
    let everything = scopes_for(&GoogleService::ALL).join(" ");
    assert_eq!(
        granted_services(&everything),
        GoogleService::ALL.into_iter().collect()
    );
}

#[test]
fn turned_off_services_are_inactive_but_still_granted() {
    let mut account = account(&[GoogleService::Mail, GoogleService::Calendar]);
    account.turned_off.insert(GoogleService::Mail);
    assert!(!account.is_active(GoogleService::Mail));
    assert!(account.is_active(GoogleService::Calendar));
    assert!(!account.is_active(GoogleService::Files), "never granted");
    assert_eq!(account.active(), BTreeSet::from([GoogleService::Calendar]));
}

#[test]
fn avatar_initial_prefers_the_name_then_the_email() {
    assert_eq!(account(&[]).initial(), "S");
    let unnamed = GoogleAccount {
        email: "ola@example.com".into(),
        ..GoogleAccount::default()
    };
    assert_eq!(unnamed.initial(), "O");
    assert_eq!(GoogleAccount::default().initial(), "G");
}

#[test]
fn reducer_announces_connecting_connected_and_disconnected() {
    let mut google = GoogleConnection::default();
    assert_eq!(google.state, GoogleState::Loading);
    google.apply(&GoogleEvent::Changed(GoogleState::NotConnected));
    assert_eq!(google.notice, None);
    google.apply(&GoogleEvent::Changed(GoogleState::Connecting {
        url: "https://accounts.google.com/o/oauth2/v2/auth?x".into(),
        adding: None,
    }));
    let connected = account(&[GoogleService::Mail]);
    google.apply(&GoogleEvent::Changed(GoogleState::Connected(connected)));
    assert_eq!(
        google.notice.as_deref(),
        Some("Google is connected as sam@example.com.")
    );
    google.apply(&GoogleEvent::Changed(GoogleState::NotConnected));
    assert!(
        google
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("disconnected"))
    );
}

#[test]
fn reducer_confirms_an_added_service_only_when_google_allowed_it() {
    let mut google = GoogleConnection::default();
    let adding = |google: &mut GoogleConnection| {
        google.apply(&GoogleEvent::Changed(GoogleState::Connecting {
            url: "u".into(),
            adding: Some(GoogleService::Files),
        }));
    };
    adding(&mut google);
    google.apply(&GoogleEvent::Changed(GoogleState::Connected(account(&[
        GoogleService::Files,
    ]))));
    assert_eq!(
        google.notice.as_deref(),
        Some("Sovra can now use your Google Drive.")
    );
    adding(&mut google);
    assert_eq!(google.notice, None, "a new attempt clears the old notice");
    google.apply(&GoogleEvent::Notice("Google didn't allow Files.".into()));
    google.apply(&GoogleEvent::Changed(GoogleState::Connected(account(&[]))));
    assert_eq!(google.notice.as_deref(), Some("Google didn't allow Files."));
}

#[test]
fn needs_sign_in_keeps_the_account_for_the_page() {
    let state = GoogleState::NeedsSignIn(account(&[GoogleService::Tasks]));
    assert_eq!(
        state.account().map(|a| a.email.as_str()),
        Some("sam@example.com")
    );
    assert_eq!(GoogleState::NotConnected.account(), None);
    assert_eq!(state.status_label(), "Needs you to sign in again");
}

#[test]
fn requests_name_their_service_and_whether_they_change_things() {
    let draft = GoogleRequest::DraftMail {
        to: vec!["a@b.c".into()],
        subject: "Hi".into(),
        body: "Hello".into(),
        reply_to: None,
    };
    assert_eq!(draft.service(), GoogleService::Mail);
    assert!(draft.changes_things());
    let tasks = GoogleRequest::ListTasks { limit: 5 };
    assert_eq!(tasks.service(), GoogleService::Tasks);
    assert!(!tasks.changes_things());
}

#[test]
fn result_limits_stay_small() {
    assert_eq!(result_limit(None), 10);
    assert_eq!(result_limit(Some(0)), 1);
    assert_eq!(result_limit(Some(500)), 25);
    assert_eq!(result_limit(Some(u64::MAX)), 10);
}

#[test]
fn clip_text_cuts_on_a_word_and_says_so() {
    assert_eq!(clip_text("  short  ", 20), "short");
    let clipped = clip_text("one two three four five six", 12);
    assert_eq!(clipped, "one two … [cut short]");
    assert_eq!(clip_text("ééééé", 3), "ééé … [cut short]");
}

#[test]
fn html_email_becomes_readable_text() {
    let html = "<html><head><title>x</title><style>p{color:red}</style></head>\
                <body><p>Hi&nbsp;Sam,</p><div>Lunch at <b>noon</b> &amp; tea?</div>\
                <script>alert('x')</script><br><br><br>Bye</body></html>";
    assert_eq!(html_to_text(html), "Hi Sam,\n\nLunch at noon & tea?\n\nBye");
    assert_eq!(
        html_to_text("a < b"),
        "a < b",
        "a lone < is text, not a tag"
    );
}

use std::collections::{BTreeMap, BTreeSet};

use super::*;

#[derive(Default)]
struct MemoryVault(Mutex<BTreeMap<String, String>>);

impl SecretVault for MemoryVault {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().expect("test lock").get(key).cloned())
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.0
            .lock()
            .expect("test lock")
            .insert(key.into(), secret.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().expect("test lock").remove(key);
        Ok(())
    }
}

fn adapter(client_id: &str) -> (GoogleAdapter, Arc<MemoryVault>) {
    let vault = Arc::new(MemoryVault::default());
    let environment = (!client_id.is_empty()).then(|| ClientConfig {
        client_id: client_id.into(),
        client_secret: None,
    });
    (
        GoogleAdapter::new(
            ClientSources {
                environment,
                baked: None,
            },
            vault.clone(),
        )
        .expect("client"),
        vault,
    )
}

#[test]
fn a_build_without_a_google_client_says_so_in_plain_words() {
    let (google, _) = adapter("");
    let reason = google.unavailable_reason().expect("unavailable");
    assert!(reason.contains("desktop client id"));
    assert!(!reason.contains("newer version"));
    assert!(google.begin_sign_in(&BTreeSet::new(), None).is_err());
    assert_eq!(
        adapter("id.apps.googleusercontent.com")
            .0
            .unavailable_reason(),
        None
    );
}

#[test]
fn saved_desktop_client_survives_restart_and_account_disconnect() {
    let (google, vault) = adapter("");
    assert!(google.save_desktop_client(" ", None).is_err());
    assert_eq!(vault.get(CLIENT_VAULT_KEY), Ok(None));
    google
        .save_desktop_client(
            " saved.apps.googleusercontent.com ",
            Some(" fixture-secret ".into()),
        )
        .unwrap();
    assert_eq!(google.unavailable_reason(), None);
    let restarted = GoogleAdapter::new(
        ClientSources {
            environment: Some(ClientConfig {
                client_id: "environment".into(),
                client_secret: None,
            }),
            baked: None,
        },
        vault.clone(),
    )
    .unwrap();
    restarted.forget().unwrap();
    let client = restarted.desktop_client().unwrap();
    assert_eq!(client.client_id, "saved.apps.googleusercontent.com");
    assert_eq!(client.client_secret.as_deref(), Some("fixture-secret"));
    assert!(vault.get(CLIENT_VAULT_KEY).unwrap().is_some());
    assert_eq!(vault.get(VAULT_KEY), Ok(None));
}

#[test]
fn saved_sign_in_survives_a_restart_and_forget_removes_it() {
    let (google, vault) = adapter("id");
    assert_eq!(google.restore(), Ok(None));
    assert!(
        google.save(&GoogleAccount::default()).is_err(),
        "nothing to save yet"
    );
    *google.session() = Some(Session {
        refresh_token: "1//refresh".into(),
        scope: "openid".into(),
        ..Session::default()
    });
    let account = GoogleAccount {
        email: "sam@example.com".into(),
        name: "Sam".into(),
        granted: BTreeSet::from([GoogleService::Calendar]),
        turned_off: BTreeSet::from([GoogleService::Calendar]),
    };
    google.save(&account).expect("save");

    let (restarted, _) = adapter("id");
    let restarted = GoogleAdapter {
        vault: vault.clone(),
        ..restarted
    };
    assert_eq!(restarted.restore(), Ok(Some(account)));
    assert_eq!(
        restarted
            .session()
            .as_ref()
            .map(|s| s.refresh_token.as_str()),
        Some("1//refresh")
    );
    restarted.forget().expect("forget");
    assert_eq!(vault.get(VAULT_KEY), Ok(None));
    assert!(restarted.session().is_none());
}

#[test]
fn an_unreadable_vault_entry_signs_out_instead_of_failing() {
    let (google, vault) = adapter("id");
    vault.set(VAULT_KEY, "not json").expect("set");
    assert_eq!(google.restore(), Ok(None));
}

#[test]
fn debug_output_never_contains_secrets() {
    let config = ClientConfig {
        client_id: "id".into(),
        client_secret: Some("shh-secret".into()),
    };
    assert!(!format!("{config:?}").contains("shh"));
    let grant = AuthorizationGrant {
        code: "4/code-secret".into(),
        verifier: "verifier-secret".into(),
        redirect_uri: "http://127.0.0.1:1".into(),
    };
    let shown = format!("{grant:?}");
    assert!(!shown.contains("code-secret") && !shown.contains("verifier-secret"));
}

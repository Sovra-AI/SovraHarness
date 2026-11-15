use std::collections::BTreeMap;
use std::sync::Mutex as StdMutex;

use futures::executor::block_on;

use super::*;

#[derive(Default)]
struct Vault(StdMutex<BTreeMap<String, String>>);

#[async_trait::async_trait]
impl AsyncSecretVault for Vault {
    async fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    async fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.into(), secret.into());
        Ok(())
    }
    async fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

fn config() -> MicrosoftClient {
    MicrosoftClient {
        client_id: "00000000-0000-4000-8000-000000000001".into(),
        tenant: "common".into(),
    }
}

fn stored() -> Stored {
    Stored {
        config: config(),
        refresh_token: "private-refresh-token".into(),
        account: MicrosoftAccount {
            id: "stable-user-id".into(),
            name: "Sam".into(),
            email: "sam@example.com".into(),
            granted: MicrosoftService::defaults(),
            turned_off: BTreeSet::new(),
        },
    }
}

#[test]
fn public_setup_is_local_persistent_and_never_imported_from_legacy_tokens() {
    block_on(async {
        let vault = Arc::new(Vault::default());
        let adapter = Microsoft::new(vault.clone()).unwrap();
        vault
            .set(VAULT_KEY, &serde_json::to_string(&stored()).unwrap())
            .await
            .unwrap();
        assert_eq!(adapter.load_client().await.unwrap(), None);
        adapter.save_client(&config()).await.unwrap();
        assert!(vault.get(VAULT_KEY).await.unwrap().is_none());
        let restarted = Microsoft::new(vault.clone()).unwrap();
        assert_eq!(restarted.load_client().await.unwrap(), Some(config()));
        restarted
            .session(stored(), None)
            .save(&stored().account)
            .await
            .unwrap();
        let mut invalid = config();
        invalid.client_id = "not-an-id".into();
        assert!(restarted.save_client(&invalid).await.is_err());
        assert!(vault.get(VAULT_KEY).await.unwrap().is_some());
        let mut different = config();
        different.tenant = "consumers".into();
        restarted.save_client(&different).await.unwrap();
        assert!(vault.get(VAULT_KEY).await.unwrap().is_none());
        restarted.forget_client().await.unwrap();
        assert_eq!(restarted.load_client().await.unwrap(), None);
        vault.set(CLIENT_KEY, "corrupt-local-setup").await.unwrap();
        let error = restarted.load_client().await.unwrap_err().to_string();
        assert!(!error.contains("corrupt-local-setup"));
        restarted.save_client(&config()).await.unwrap();
        assert_eq!(restarted.load_client().await.unwrap(), Some(config()));
    });
}

#[test]
fn microsoft_authorization_uses_public_client_pkce_and_localhost() {
    let adapter = Microsoft::new(Arc::new(Vault::default())).unwrap();
    let first = adapter
        .begin_sign_in(&config(), &MicrosoftService::defaults(), None)
        .unwrap();
    let second = adapter
        .begin_sign_in(&config(), &MicrosoftService::defaults(), None)
        .unwrap();
    let url = url::Url::parse(first.url()).unwrap();
    assert_eq!(url.host_str(), Some("login.microsoftonline.com"));
    assert_eq!(url.path(), "/common/oauth2/v2.0/authorize");
    let params: BTreeMap<_, _> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["response_type"], "code");
    assert!(params["redirect_uri"].starts_with("http://localhost:"));
    assert!(params["scope"].contains("offline_access"));
    assert!(!params.contains_key("client_secret"));
    assert_ne!(first.url(), second.url());
    assert!(!format!("{first:?}").contains(&params["state"]));
    first.canceller()();
    second.canceller()();
    assert!(matches!(
        first.wait(),
        Err(sovra_core::oauth::OAuthError::Cancelled)
    ));
    assert!(matches!(
        second.wait(),
        Err(sovra_core::oauth::OAuthError::Cancelled)
    ));
}

#[test]
fn uncommitted_session_has_no_persistence_side_effect_and_disconnect_cannot_resurrect_it() {
    block_on(async {
        let vault = Arc::new(Vault::default());
        let adapter = Microsoft::new(vault.clone()).unwrap();
        let session = adapter.session(stored(), None);
        assert!(vault.get(VAULT_KEY).await.unwrap().is_none());
        assert!(!format!("{session:?}").contains("private-refresh-token"));
        let mut account = session.account();
        account.turned_off.insert(MicrosoftService::Mail);
        session.save(&account).await.unwrap();
        let restored = adapter.restore(&config()).await.unwrap().unwrap();
        assert!(!restored.account().active(MicrosoftService::Mail));
        assert!(matches!(
            restored
                .call(MicrosoftRequest::SearchMail {
                    query: String::new()
                })
                .await,
            Err(MicrosoftError::NotAllowed)
        ));
        session.disconnect().await.unwrap();
        assert!(adapter.restore(&config()).await.unwrap().is_none());
        assert!(matches!(
            session.save(&account).await,
            Err(MicrosoftError::NotAllowed)
        ));
        assert!(vault.get(VAULT_KEY).await.unwrap().is_none());
    });
}

#[test]
fn credentials_cannot_cross_application_or_account_boundaries() {
    block_on(async {
        let vault = Arc::new(Vault::default());
        let adapter = Microsoft::new(vault.clone()).unwrap();
        let session = adapter.session(stored(), None);
        let mut wrong = session.account();
        wrong.id = "another-user".into();
        assert!(matches!(
            session.save(&wrong).await,
            Err(MicrosoftError::NotAllowed)
        ));
        let mut foreign = stored();
        foreign.config.tenant = "organizations".into();
        vault
            .set(VAULT_KEY, &serde_json::to_string(&foreign).unwrap())
            .await
            .unwrap();
        assert!(adapter.restore(&config()).await.unwrap().is_none());
        vault.set(VAULT_KEY, "broken-private-json").await.unwrap();
        let error = adapter.restore(&config()).await.unwrap_err();
        assert!(!error.to_string().contains("broken-private-json"));
        assert_eq!(error, MicrosoftError::SignInNeeded);
    });
}

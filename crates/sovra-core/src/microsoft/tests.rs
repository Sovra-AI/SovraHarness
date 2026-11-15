use super::*;

#[test]
fn defaults_never_ask_for_organization_access_or_mail_sending() {
    let defaults = MicrosoftService::defaults();
    assert_eq!(defaults.len(), 6);
    assert!(
        defaults
            .iter()
            .all(|service| !service.info().organization_only)
    );
    let scopes = scopes_for(&defaults);
    assert!(scopes.contains(&"offline_access"));
    assert!(scopes.contains(&"User.Read"));
    assert!(scopes.contains(&"Mail.ReadWrite"));
    assert!(!scopes.contains(&"Mail.Send"));
    assert!(!scopes.contains(&"ChannelMessage.Read.All"));
    assert!(!scopes.contains(&"Files.ReadWrite"));
    assert_eq!(scopes.len(), scopes.iter().collect::<BTreeSet<_>>().len());
}

#[test]
fn effective_grants_require_every_scope_and_recognize_graph_prefixes() {
    assert!(granted_services("").is_empty());
    assert!(
        !granted_services("Team.ReadBasic.All Channel.ReadBasic.All")
            .contains(&MicrosoftService::Channels)
    );
    let grants = granted_services(
        "https://graph.microsoft.com/Mail.ReadWrite Files.Read.All Tasks.ReadWrite",
    );
    assert_eq!(
        grants,
        BTreeSet::from([
            MicrosoftService::Mail,
            MicrosoftService::Files,
            MicrosoftService::Tasks,
            MicrosoftService::Planner
        ])
    );
    assert!(
        granted_services("Team.ReadBasic.All Channel.ReadBasic.All ChannelMessage.Read.All")
            .contains(&MicrosoftService::Channels)
    );
}

#[test]
fn switches_do_not_change_granted_permissions() {
    let account = MicrosoftAccount {
        granted: BTreeSet::from([MicrosoftService::Mail, MicrosoftService::Files]),
        turned_off: BTreeSet::from([MicrosoftService::Mail]),
        ..MicrosoftAccount::default()
    };
    assert!(!account.active(MicrosoftService::Mail));
    assert!(account.active(MicrosoftService::Files));
    assert!(!account.active(MicrosoftService::Channels));
    assert!(account.granted.contains(&MicrosoftService::Mail));
}

#[test]
fn client_configuration_cannot_redirect_authority_to_another_host() {
    let mut client = MicrosoftClient {
        client_id: "00000000-0000-4000-8000-000000000001".into(),
        tenant: "common".into(),
    };
    assert!(client.validate().is_ok());
    for tenant in [
        "organizations",
        "consumers",
        "00000000-0000-4000-8000-000000000002",
    ] {
        client.tenant = tenant.into();
        assert!(client.validate().is_ok());
    }
    for tenant in [
        "https://attacker.test",
        "common/../../elsewhere",
        "",
        "common?x=y",
    ] {
        client.tenant = tenant.into();
        assert!(client.validate().is_err());
    }
    client.tenant = "common".into();
    client.client_id = "not-an-app-id".into();
    assert!(client.validate().is_err());
}

#[test]
fn resource_ids_cannot_change_a_graph_path() {
    for id in [
        "",
        ".",
        "..",
        "../sendMail",
        "a/b",
        "a\\b",
        "a?x=y",
        "a#b",
        "%2e%2e",
        "a\nb",
    ] {
        assert!(ResourceId::try_from(id.to_owned()).is_err(), "{id}");
    }
    for id in [
        "AAMk-abc_123=",
        "b!abc",
        "19:abc@thread.tacv2",
        "contoso.sharepoint.com,abc,def",
    ] {
        let parsed = ResourceId::try_from(id.to_owned()).expect("Graph ID");
        assert_eq!(parsed.as_str(), id);
        assert_eq!(
            serde_json::from_str::<ResourceId>(&serde_json::to_string(&parsed).unwrap()).unwrap(),
            parsed
        );
    }
}

#[test]
fn requests_exclude_sending_inviting_and_arbitrary_graph_calls() {
    for request in [
        r#"{"action":"send_mail","to":"a@example.com"}"#,
        r#"{"action":"get","url":"https://graph.microsoft.com/v1.0/me"}"#,
        r#"{"action":"draft_mail","to":"a@example.com","subject":"Hi","body":"Hello","send":true}"#,
        r#"{"action":"add_event","subject":"Hi","start":"x","end":"y","time_zone":"UTC","attendees":["a@example.com"]}"#,
    ] {
        assert!(serde_json::from_str::<MicrosoftRequest>(request).is_err());
    }
    let draft = MicrosoftRequest::DraftMail {
        to: "a@example.com".into(),
        subject: "Hello".into(),
        body: "A draft\nwith paragraphs.".into(),
    };
    assert!(draft.validate().is_ok());
    assert_eq!(draft.service(), MicrosoftService::Mail);
    assert!(draft.changes_things());
    let bad = MicrosoftRequest::DraftMail {
        to: "a@example.com\r\nBcc:b@example.com".into(),
        subject: "Hello".into(),
        body: String::new(),
    };
    assert!(bad.validate().is_err());
    assert!(
        MicrosoftRequest::SearchFiles {
            query: "a".repeat(513)
        }
        .validate()
        .is_err()
    );
    assert!(!MicrosoftRequest::ListPlannerTasks.changes_things());
    assert_eq!(
        MicrosoftRequest::ListPlannerTasks.service(),
        MicrosoftService::Planner
    );
}

#[test]
fn connection_reducer_preserves_actionable_notices_until_state_changes() {
    let mut connection = MicrosoftConnection::default();
    assert_eq!(connection.state.status_label(), "Checking…");
    connection.apply(&MicrosoftEvent::Notice("Ask your administrator.".into()));
    connection.apply(&MicrosoftEvent::OpenSignIn {
        url: "https://login.microsoftonline.com/".into(),
    });
    assert!(connection.notice.is_some());
    let client = MicrosoftClient::from_setup(
        MicrosoftSetupKind::Personal,
        "10000000-0000-4000-8000-000000000001",
        "",
    )
    .unwrap();
    connection.apply(&MicrosoftEvent::SetupChanged(Some(client.clone())));
    assert_eq!(connection.client, Some(client));
    assert!(connection.notice.is_some());
    connection.apply(&MicrosoftEvent::SetupChanged(None));
    assert_eq!(connection.client, None);
    let account = MicrosoftAccount {
        name: "Sam".into(),
        ..MicrosoftAccount::default()
    };
    connection.apply(&MicrosoftEvent::Changed(MicrosoftState::Connected(
        account.clone(),
    )));
    assert_eq!(connection.state.account(), Some(&account));
    assert_eq!(connection.state.status_label(), "Connected");
    assert!(connection.notice.is_none());
    connection.apply(&MicrosoftEvent::Changed(MicrosoftState::NotConnected));
    assert!(connection.state.account().is_none());
    assert_eq!(connection.state.status_label(), "Not connected");
}

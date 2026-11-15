use super::*;
use sovra_core::microsoft::ResourceId;

fn base() -> url::Url {
    url::Url::parse("https://graph.microsoft.com/v1.0/").unwrap()
}
fn id(value: &str) -> ResourceId {
    value.to_owned().try_into().unwrap()
}

#[test]
fn writes_have_no_send_endpoint_or_attendees() {
    let draft = request(
        &base(),
        &MicrosoftRequest::DraftMail {
            to: "sam@example.com".into(),
            subject: "Hello".into(),
            body: "A draft".into(),
        },
    )
    .unwrap();
    assert_eq!(draft.method, Method::POST);
    assert_eq!(draft.url.path(), "/v1.0/me/messages");
    let body = draft.body.unwrap();
    assert!(body.get("send").is_none());
    assert_eq!(body["body"]["contentType"], "Text");
    let event = request(
        &base(),
        &MicrosoftRequest::AddEvent {
            subject: "Dentist".into(),
            start: "2026-11-15T10:00:00+04:00".into(),
            end: "2026-11-15T11:00:00+04:00".into(),
            time_zone: "UTC".into(),
        },
    )
    .unwrap();
    let body = event.body.unwrap();
    assert!(body.get("attendees").is_none());
    assert_eq!(body["start"]["dateTime"], "2026-11-15T06:00:00Z");
    assert_eq!(body["start"]["timeZone"], "UTC");
}

#[test]
fn dates_are_checked_before_network_io() {
    for (from, to) in [
        ("tomorrow", "next week"),
        ("2026-11-16T00:00:00Z", "2026-11-15T00:00:00Z"),
        ("2026-01-01T00:00:00Z", "2028-01-01T00:00:00Z"),
    ] {
        assert!(
            request(
                &base(),
                &MicrosoftRequest::ListEvents {
                    from: from.into(),
                    to: to.into()
                }
            )
            .is_err()
        );
    }
}

#[test]
fn searches_escape_odata_literals_and_identifiers_remain_path_segments() {
    let spec = request(
        &base(),
        &MicrosoftRequest::SearchFiles {
            query: "O'Brien/a?$filter=x".into(),
        },
    )
    .unwrap();
    assert_eq!(spec.url.host_str(), Some("graph.microsoft.com"));
    let path: Vec<_> = spec.url.path_segments().unwrap().collect();
    assert_eq!(path.len(), 5);
    assert!(path[4].contains("O''Brien%2Fa"));
    assert!(spec.url.query_pairs().all(|(key, _)| key != "$filter"));
    let spec = request(
        &base(),
        &MicrosoftRequest::ReadChannel {
            team_id: id("team-id"),
            channel_id: id("19:abc@thread.tacv2"),
        },
    )
    .unwrap();
    assert_eq!(spec.url.path_segments().unwrap().count(), 6);
}

#[test]
fn downloads_only_follow_microsoft_https_hosts() {
    for url in [
        "https://tenant.sharepoint.com/file",
        "https://public.dm.files.1drv.com/file",
        "https://foo.storage.live.com/file",
    ] {
        assert!(download_host(&url::Url::parse(url).unwrap()));
    }
    for url in [
        "http://tenant.sharepoint.com/file",
        "https://sharepoint.com.evil.test/file",
        "https://evilsharepoint.com/file",
        "https://127.0.0.1/file",
        "https://user@tenant.sharepoint.com/file",
        "https://tenant.sharepoint.com:444/file",
    ] {
        assert!(!download_host(&url::Url::parse(url).unwrap()));
    }
}

#[test]
fn results_strip_temporary_urls_and_bound_outside_content() {
    let value = json!({
        "@odata.nextLink":"https://graph.microsoft.com/v1.0/me/messages?$skip=20",
        "@microsoft.graph.downloadUrl":"private-signed-url",
        "contentUrl":"private-content-url",
        "body":{"contentType":"html","content":"<p>Read me</p><script>doBadThings()</script>"},
        "value":(0..30).map(|_| json!({"text":"a".repeat(1000)})).collect::<Vec<_>>()
    });
    let mut budget = 3000;
    let result = compact(value, &mut budget, 0);
    let text = result.to_string();
    assert!(!text.contains("private-"));
    assert!(!text.contains("nextLink"));
    assert!(!text.contains("doBadThings"));
    assert!(text.contains("Read me"));
    assert_eq!(result["value"].as_array().unwrap().len(), TOP);
    assert!(text.len() < 4000);
}

#[test]
fn every_read_action_maps_to_a_bounded_known_graph_endpoint() {
    let actions = [
        MicrosoftRequest::ReadMail {
            message_id: id("message"),
        },
        MicrosoftRequest::SearchContacts {
            query: "Sam".into(),
        },
        MicrosoftRequest::ListTaskLists,
        MicrosoftRequest::ListTasks {
            list_id: id("list"),
        },
        MicrosoftRequest::ListNotes,
        MicrosoftRequest::ReadNote {
            page_id: id("page"),
        },
        MicrosoftRequest::SearchSites {
            query: "project".into(),
        },
        MicrosoftRequest::ListSiteDrives {
            site_id: id("site"),
        },
        MicrosoftRequest::SearchSiteFiles {
            drive_id: id("drive"),
            query: "budget".into(),
        },
        MicrosoftRequest::ReadSiteFile {
            drive_id: id("drive"),
            item_id: id("file"),
        },
        MicrosoftRequest::ReadFile {
            item_id: id("file"),
        },
        MicrosoftRequest::ListChats,
        MicrosoftRequest::ReadChat {
            chat_id: id("chat"),
        },
        MicrosoftRequest::ListTeams,
        MicrosoftRequest::ListChannels {
            team_id: id("team"),
        },
        MicrosoftRequest::ReadChannelReplies {
            team_id: id("team"),
            channel_id: id("channel"),
            message_id: id("message"),
        },
        MicrosoftRequest::ListPlannerTasks,
    ];
    for action in actions {
        let spec = request(&base(), &action).unwrap();
        assert_eq!(spec.method, Method::GET);
        assert_eq!(spec.url.host_str(), Some("graph.microsoft.com"));
        assert!(spec.url.path().starts_with("/v1.0/"));
        assert!(spec.body.is_none());
    }
}

#[test]
fn teams_endpoints_only_use_the_query_options_microsoft_supports() {
    let teams = request(&base(), &MicrosoftRequest::ListTeams).unwrap();
    assert!(teams.url.query().is_none());
    let chats = request(&base(), &MicrosoftRequest::ListChats).unwrap();
    assert!(chats.url.query_pairs().all(|(key, _)| key == "$top"));
}

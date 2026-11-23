use super::*;

fn s3_place() -> StoragePlace {
    StoragePlace {
        id: "place-1".into(),
        name: "Family photos".into(),
        bucket: "family-photos".into(),
        user: "AKIDEXAMPLE".into(),
        ..StoragePlace::new(StorageService::AmazonS3)
    }
}

#[test]
fn every_service_has_words_and_a_usable_start() {
    let mut slugs = std::collections::BTreeSet::new();
    for service in StorageService::ALL {
        assert!(slugs.insert(service.slug()), "{service:?} slug repeats");
        assert_ne!(service.label(), "");
        assert!(service.sentence().ends_with('.'), "{service:?}");
        assert!(service.keys_help().ends_with('.'), "{service:?}");
        assert!(service.address_example().starts_with("https://"));
        assert_eq!(service.has_region(), !service.default_region().is_empty());
        let place = StoragePlace::new(service);
        assert_eq!(place.service, service);
        if let AddressRule::FromRegion(_) | AddressRule::Fixed(_) = service.address() {
            assert!(place.endpoint.starts_with("https://"), "{service:?}");
        }
    }
    assert_eq!(slugs.len(), StorageService::ALL.len());
}

#[test]
fn addresses_follow_region_and_account() {
    assert_eq!(
        StorageService::AmazonS3
            .suggested_address("eu-west-2", "")
            .as_deref(),
        Some("https://s3.eu-west-2.amazonaws.com")
    );
    assert_eq!(
        StorageService::AzureBlob
            .suggested_address("", "MyAccount")
            .as_deref(),
        Some("https://myaccount.blob.core.windows.net")
    );
    assert_eq!(StorageService::MinIo.suggested_address("x", "y"), None);
    let azure = StoragePlace {
        name: "Backups".into(),
        bucket: "backups".into(),
        user: "myaccount".into(),
        ..StoragePlace::new(StorageService::AzureBlob)
    }
    .tidied();
    assert_eq!(azure.endpoint, "https://myaccount.blob.core.windows.net");
    assert_eq!(azure.region, "");
    azure.validate().expect("azure place is fine");
}

#[test]
fn a_good_place_validates_and_bad_fields_are_named() {
    s3_place().tidied().validate().expect("good place");
    let cases = [
        (
            StoragePlace {
                name: String::new(),
                ..s3_place()
            },
            "short name",
        ),
        (
            StoragePlace {
                bucket: "Bad_Bucket".into(),
                ..s3_place()
            },
            "bucket name",
        ),
        (
            StoragePlace {
                endpoint: "http://s3.example.com".into(),
                ..s3_place()
            },
            "https://",
        ),
        (
            StoragePlace {
                endpoint: "https://s3.example.com/bucket".into(),
                ..s3_place()
            },
            "nothing after it",
        ),
        (
            StoragePlace {
                endpoint: "https://<account id>.r2.cloudflarestorage.com".into(),
                service: StorageService::CloudflareR2,
                ..s3_place()
            },
            "",
        ),
        (
            StoragePlace {
                folder: "../escape".into(),
                ..s3_place()
            },
            "folder",
        ),
        (
            StoragePlace {
                user: String::new(),
                ..s3_place()
            },
            "access key",
        ),
    ];
    for (place, words) in cases {
        let error = place.tidied().validate().expect_err("should be refused");
        assert!(error.to_string().contains(words), "{error} / {words}");
    }
}

#[test]
fn plain_http_only_reaches_this_computer_or_the_home_network() {
    let nas = |endpoint: &str| StoragePlace {
        name: "NAS".into(),
        endpoint: endpoint.into(),
        user: "sam".into(),
        ..StoragePlace::new(StorageService::OtherWebDav)
    };
    for ok in [
        "http://192.168.1.20:5005/home/",
        "http://nas.local/dav/",
        "http://localhost:8080/",
        "https://nas.example.com/dav/",
    ] {
        nas(ok).tidied().validate().expect(ok);
    }
    for bad in ["http://nas.example.com/dav/", "http://8.8.8.8/"] {
        assert!(nas(bad).tidied().validate().is_err(), "{bad}");
    }
    // Public clouds only accept http on this computer.
    let mut cloud = s3_place();
    cloud.endpoint = "http://192.168.1.20:9000".into();
    assert!(cloud.validate().is_err());
    cloud.service = StorageService::MinIo;
    cloud.validate().expect("MinIO at home");
}

#[test]
fn webdav_addresses_keep_their_folder_and_end_in_a_slash() {
    let place = StoragePlace {
        name: "Nextcloud".into(),
        endpoint: " https://cloud.example.com/remote.php/dav/files/sam ".into(),
        user: "sam".into(),
        bucket: "ignored".into(),
        region: "ignored".into(),
        ..StoragePlace::new(StorageService::Nextcloud)
    }
    .tidied();
    assert_eq!(
        place.endpoint,
        "https://cloud.example.com/remote.php/dav/files/sam/"
    );
    assert_eq!(place.bucket, "");
    assert_eq!(place.region, "");
    place.validate().expect("nextcloud place");
    assert_eq!(place.summary(), "sam on cloud.example.com");
}

#[test]
fn paths_are_tidied_and_escapes_refused() {
    assert_eq!(clean_path(" /photos/2026/ ").as_deref(), Ok("photos/2026"));
    assert_eq!(clean_path("").as_deref(), Ok(""));
    for bad in ["a/../b", "./a", "a//b", "a\\b", "a\u{0}b"] {
        assert!(clean_path(bad).is_err(), "{bad}");
    }
    let place = StoragePlace {
        folder: "sovra".into(),
        ..s3_place()
    };
    assert_eq!(place.full_path("notes.md"), "sovra/notes.md");
    assert_eq!(place.full_path(""), "sovra");
    assert_eq!(place.relative_path("sovra/notes.md"), Some("notes.md"));
    assert_eq!(place.relative_path("sovra"), Some(""));
    assert_eq!(place.relative_path("sovrax/notes.md"), None);
    assert_eq!(s3_place().relative_path("a/b"), Some("a/b"));
    assert_eq!(parent_folder("a/b/c"), "a/b");
    assert_eq!(parent_folder("a"), "");
    assert_eq!(StorageEntry::name_of("a/b/"), "b");
}

#[test]
fn listings_put_folders_first_and_respect_the_limit() {
    let entry = |path: &str, folder| StorageEntry {
        path: path.into(),
        name: StorageEntry::name_of(path),
        folder,
        size: None,
        modified: None,
    };
    let listing = Listing {
        entries: vec![
            entry("b.txt", false),
            entry("Photos", true),
            entry("a.txt", false),
            entry("a.txt", false),
            entry("", true),
        ],
        more: false,
    }
    .tidy("", 2);
    let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Photos", "a.txt"]);
    assert!(listing.more);
}

#[test]
fn places_resolve_by_id_or_name_and_one_place_needs_no_name() {
    let one = [s3_place()];
    assert_eq!(resolve_place(&one, "").map(|p| &p.id), Ok(&one[0].id));
    let two = [
        s3_place(),
        StoragePlace {
            id: "place-2".into(),
            name: "Work".into(),
            ..s3_place()
        },
    ];
    assert!(resolve_place(&two, "").is_err());
    assert_eq!(
        resolve_place(&two, "work").map(|p| p.id.as_str()),
        Ok("place-2")
    );
    assert_eq!(
        resolve_place(&two, "place-1").map(|p| p.id.as_str()),
        Ok("place-1")
    );
    assert!(resolve_place(&two, "nope").is_err());
    assert!(resolve_place(&[], "").is_err());
    let id = new_place_id(1, &two);
    assert!(!two.iter().any(|place| place.id == id));
}

#[test]
fn secrets_never_print_and_a_pasted_sas_loses_its_question_mark() {
    let secret = StorageSecret::new(" ?sv=2024&sig=abc ").expect("secret");
    assert_eq!(secret.as_str(), "sv=2024&sig=abc");
    assert!(!format!("{secret:?}").contains("abc"));
    assert_eq!(StorageSecret::new("   "), None);
    let place = s3_place();
    assert!(!format!("{place:?}").contains("sig"));
}

#[test]
fn the_book_folds_events_and_forgets_removed_places() {
    let mut book = StorageBook::default();
    let view = PlaceView {
        place: s3_place(),
        has_secret: true,
    };
    book.apply(&StorageEvent::Places(vec![view]));
    assert!(book.loaded);
    assert_eq!(book.status_of("place-1"), &PlaceStatus::Unchecked);
    book.apply(&StorageEvent::Status {
        id: "place-1".into(),
        status: PlaceStatus::Works,
    });
    assert_eq!(book.status_of("place-1").label(), "Working");

    book.browse("place-1", "photos");
    book.apply(&StorageEvent::Listing {
        id: "place-1".into(),
        folder: "other".into(),
        result: Ok(Listing::default()),
    });
    assert_eq!(book.browsing.as_ref().and_then(|b| b.listing.clone()), None);
    book.apply(&StorageEvent::Listing {
        id: "place-1".into(),
        folder: "photos".into(),
        result: Ok(Listing::default()),
    });
    assert!(book.browsing.as_ref().is_some_and(|b| b.listing.is_some()));

    book.apply(&StorageEvent::SaveRefused("bad keys".into()));
    assert_eq!(book.refused.as_deref(), Some("bad keys"));
    book.apply(&StorageEvent::Saved {
        id: "place-1".into(),
        name: "Family photos".into(),
    });
    assert_eq!(book.refused, None);
    assert!(
        book.notice
            .as_deref()
            .is_some_and(|n| n.contains("Family photos"))
    );

    book.apply(&StorageEvent::Places(Vec::new()));
    assert!(book.status.is_empty());
    assert!(book.browsing.is_none());
}

#[test]
fn saved_places_round_trip_without_secrets() {
    let place = s3_place();
    let json = serde_json::to_string(&place).expect("json");
    assert!(json.contains("\"amazon-s3\""));
    let back: StoragePlace = serde_json::from_str(&json).expect("back");
    assert_eq!(back, place);
}

#[test]
fn file_types_follow_the_name() {
    assert_eq!(content_type("notes.MD"), "text/markdown; charset=utf-8");
    assert_eq!(content_type("data.csv"), "text/csv; charset=utf-8");
    assert_eq!(content_type("README"), "text/plain; charset=utf-8");
}

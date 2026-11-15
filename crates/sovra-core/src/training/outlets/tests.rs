use super::*;
use crate::group::ContentHash;
use crate::training::library::tests::{card_for, micro_file};
use crate::training::{
    Architecture, BaseModel, ModelCard, ModelHeader, ModelOrigin, TrainingCommand, encode_model,
    inspect_model,
};

type Change = Box<dyn Fn(&mut BucketSettings)>;

fn bucket() -> BucketSettings {
    BucketSettings {
        endpoint: "https://s3.eu-west-2.amazonaws.com".into(),
        region: "eu-west-2".into(),
        bucket: "my-models".into(),
        folder: "sovra".into(),
        access_key_id: "AKIAEXAMPLE".into(),
    }
}

fn library_model(fill: f32, name: &str) -> LibraryModel {
    let mut card = card_for(&micro_file(fill));
    card.name = name.into();
    LibraryModel {
        card,
        saved_at: String::new(),
        delivery: Vec::new(),
    }
}

fn starter_model() -> LibraryModel {
    let file = encode_model(
        &ModelHeader {
            base: BaseModel::Starter,
            architecture: Architecture::STARTER,
            rounds: 1,
            tokens: 1,
            loss_milli: None,
            check_loss_milli: None,
        },
        &vec![0.0; Architecture::STARTER.parameters()],
    )
    .unwrap();
    LibraryModel {
        card: ModelCard::from_facts(
            &inspect_model(&file).unwrap(),
            "Letters".into(),
            "Ada".into(),
            ModelOrigin::TrainedHere,
            None,
            1,
        ),
        saved_at: String::new(),
        delivery: Vec::new(),
    }
}

#[test]
fn a_bucket_is_tidied_and_every_wrong_field_is_named() {
    let typed = BucketSettings {
        endpoint: " https://s3.eu-west-2.amazonaws.com/ ".into(),
        region: " eu-west-2 ".into(),
        bucket: " my-models ".into(),
        folder: "/sovra/".into(),
        access_key_id: " AKIAEXAMPLE ".into(),
    };
    assert_eq!(typed.tidied(), bucket());
    assert_eq!(bucket().validate(), Ok(()));
    assert_eq!(bucket().object_key("a.model"), "sovra/a.model");
    assert_eq!(bucket().place("a.model"), "my-models/sovra/a.model");
    let unfiled = BucketSettings {
        folder: String::new(),
        ..bucket()
    };
    assert_eq!(unfiled.validate(), Ok(()));
    assert_eq!(unfiled.object_key("a.model"), "a.model");

    let wrong: Vec<(&str, Change)> = vec![
        (
            "web address",
            Box::new(|b| b.endpoint = "not an address".into()),
        ),
        ("web address", Box::new(|b| b.endpoint = String::new())),
        // A key must not travel in the clear to another computer.
        (
            "https://",
            Box::new(|b| b.endpoint = "http://storage.example.com".into()),
        ),
        (
            "https://",
            Box::new(|b| b.endpoint = "ftp://storage.example.com".into()),
        ),
        (
            "nothing after it",
            Box::new(|b| b.endpoint = "https://storage.example.com/bucket".into()),
        ),
        (
            "nothing after it",
            Box::new(|b| b.endpoint = "https://key:secret@storage.example.com".into()),
        ),
        (
            "nothing after it",
            Box::new(|b| b.endpoint = "https://storage.example.com?x=1".into()),
        ),
        ("region", Box::new(|b| b.region = String::new())),
        ("region", Box::new(|b| b.region = "us east".into())),
        ("bucket name", Box::new(|b| b.bucket = "ab".into())),
        ("bucket name", Box::new(|b| b.bucket = "My-Models".into())),
        ("bucket name", Box::new(|b| b.bucket = "-models".into())),
        ("bucket name", Box::new(|b| b.bucket = "a/b".into())),
        ("folder", Box::new(|b| b.folder = "a/../b".into())),
        ("folder", Box::new(|b| b.folder = "a//b".into())),
        ("folder", Box::new(|b| b.folder = "a\nb".into())),
        (
            "access key ID",
            Box::new(|b| b.access_key_id = String::new()),
        ),
        (
            "access key ID",
            Box::new(|b| b.access_key_id = "two words".into()),
        ),
    ];
    for (named, change) in wrong {
        let mut settings = bucket();
        change(&mut settings);
        let error = settings.validate().unwrap_err().to_string();
        assert!(error.contains(named), "{settings:?}: {error}");
    }
    // A service on this computer itself may be plain http.
    for address in [
        "http://localhost:9000",
        "http://127.0.0.1:9000",
        "http://[::1]:9000",
    ] {
        let local = BucketSettings {
            endpoint: address.into(),
            ..bucket()
        };
        assert_eq!(local.validate(), Ok(()), "{address}");
    }
}

#[test]
fn bucket_commands_are_checked_before_anything_is_stored() {
    let save = |settings: BucketSettings, secret: Option<&str>| {
        TrainingCommand::SaveBucket {
            settings,
            secret: secret.map(str::to_owned),
        }
        .validate()
    };
    assert_eq!(save(bucket(), Some("abc123/+=")), Ok(()));
    // No new secret keeps the stored one.
    assert_eq!(save(bucket(), None), Ok(()));
    assert!(save(bucket(), Some("")).is_err());
    assert!(save(bucket(), Some("has space")).is_err());
    assert!(save(bucket(), Some(&"k".repeat(MAX_BUCKET_SECRET_CHARS + 1))).is_err());
    let mut bad = bucket();
    bad.bucket = "X".into();
    assert!(save(bad, Some("abc")).is_err());
    let id = ContentHash::of(b"model");
    for command in [
        TrainingCommand::ForgetBucket,
        TrainingCommand::SetAutomaticBackup(true),
        TrainingCommand::SetAutomaticOllama(false),
        TrainingCommand::BackUp(id),
        TrainingCommand::AddToOllama(id),
    ] {
        assert_eq!(command.validate(), Ok(()));
    }
}

#[test]
fn copies_are_recorded_bounded_and_settle_after_a_restart() {
    let first = library_model(0.25, "First");
    let second = library_model(0.5, "Second");
    let (a, b) = (first.card.id, second.card.id);
    let mut state = ModelOutletState::default();
    assert!(state.copies(a).is_none());
    state.record(a, Outlet::Bucket, CopyState::Working);
    assert!(state.working(a, Outlet::Bucket));
    assert!(!state.working(a, Outlet::Ollama));
    state.record(
        a,
        Outlet::Bucket,
        CopyState::Done {
            place: "my-models/sovra/first.model".into(),
            at_unix_ms: 5,
        },
    );
    state.record(a, Outlet::Ollama, CopyState::Working);
    state.record(b, Outlet::Ollama, CopyState::Failed("x".repeat(1000)));
    // The newest model comes first, and one model has one entry.
    assert_eq!(state.copies.len(), 2);
    assert_eq!(state.copies[0].model, b);
    let Some(CopyState::Failed(why)) = &state.copies(b).unwrap().ollama else {
        panic!("the failure is kept");
    };
    assert_eq!(why.chars().count(), MAX_COPY_MESSAGE_CHARS);

    // Saved and reopened: nothing is being sent any more, and the secret flag
    // is not part of the file.
    state.bucket = Some(bucket());
    state.backup_automatically = true;
    state.bucket_key_saved = true;
    let saved = serde_json::to_string(&state.clone().settled(&[first.clone(), second])).unwrap();
    assert!(!saved.contains("working") && !saved.contains("bucket_key_saved"));
    let reopened: ModelOutletState = serde_json::from_str(&saved).unwrap();
    assert!(!reopened.bucket_key_saved);
    assert!(reopened.backup_automatically && !reopened.ollama_automatically);
    assert_eq!(reopened.bucket, Some(bucket()));
    assert!(!reopened.working(a, Outlet::Ollama));
    assert!(matches!(
        reopened.copies(a).unwrap().bucket,
        Some(CopyState::Done { .. })
    ));
    assert_eq!(reopened.copies(a).unwrap().ollama, None);
    // A model no longer in the library is forgotten.
    let only_first = state.settled(&[first]);
    assert!(only_first.copies(b).is_none() && only_first.copies(a).is_some());
    // An empty or older file reads as everything off.
    assert_eq!(
        serde_json::from_str::<ModelOutletState>("{}").unwrap(),
        ModelOutletState::default()
    );
    assert!(serde_json::from_str::<ModelOutletState>(r#"{"secret":"x"}"#).is_err());

    let mut many = ModelOutletState::default();
    for index in 0..(MAX_COPY_RECORDS + 10) {
        many.record(
            ContentHash::of(index.to_string().as_bytes()),
            Outlet::Bucket,
            CopyState::Working,
        );
    }
    assert_eq!(many.copies.len(), MAX_COPY_RECORDS);
}

#[test]
fn names_for_files_and_for_ollama_are_plain_and_never_collide() {
    let id = ContentHash::of(b"model");
    let start: String = id.to_hex().chars().take(8).collect();
    assert_eq!(
        outlet_name("My Garden Notes!", id),
        format!("my-garden-notes-{start}")
    );
    assert_eq!(outlet_name("  ", id), format!("model-{start}"));
    assert_eq!(outlet_name("حديقة", id), format!("model-{start}"));
    assert_eq!(ollama_name("Letters", id), format!("sovra-letters-{start}"));
    let long = outlet_name(&"word ".repeat(40), id);
    assert!(long.len() <= 49 && !long.contains("--"), "{long}");
    assert_ne!(
        outlet_name("Same", ContentHash::of(b"a")),
        outlet_name("Same", ContentHash::of(b"b"))
    );
    assert!(
        ollama_name("A/B:C", id)
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    );
}

#[test]
fn the_description_says_what_the_model_is_and_never_what_it_read() {
    let small = library_model(0.25, "Garden notes");
    let text = model_readme(&small, "garden-notes-1234abcd");
    assert!(text.starts_with("# Garden notes\n"));
    assert!(text.contains("| Starting point | Small |"));
    assert!(text.contains("garden-notes-1234abcd.model"));
    assert!(text.contains("garden-notes-1234abcd.gguf"));
    assert!(text.contains("Ollama 0.40.15 or newer"));
    assert!(text.contains(&small.card.id.to_hex()));
    assert_eq!(ollama_refusal(small.card.architecture), None);

    let starter = starter_model();
    let text = model_readme(&starter, "letters-1");
    assert!(text.contains("letters-1.model") && !text.contains(".gguf"));
    assert!(
        ollama_refusal(starter.card.architecture)
            .unwrap()
            .contains("Starter")
    );
}

use super::*;
use crate::group::{GroupId, MAX_PEER_MESSAGE_BYTES, PeerRequest, PeerResponse};
use crate::swarm::learn::scavenger::GridSyncer;
use crate::training::library::tests::{MICRO, card_for, micro_file};
use crate::training::{
    BASE_MODELS, BaseModel, HardwareInfo, LrSchedule, TrainingPreset, TrainingProgress,
};

const WORKER: DeviceId = DeviceId([1; 32]);

type Change<T> = Box<dyn Fn(&mut T)>;

fn settings(architecture: Architecture) -> TrainingSettings {
    TrainingSettings::recommended(
        TrainingPreset::Balanced,
        architecture,
        4000,
        &HardwareInfo::default(),
    )
}

fn task_for(architecture: Architecture, settings: &TrainingSettings, text: &str) -> TrainingTask {
    let parameters = vec![0.0; architecture.parameters()];
    let mut grid = GridSyncer::new(
        ContentHash::of(b"run"),
        parameters.clone(),
        settings.grid_settings(architecture),
    )
    .unwrap();
    TrainingTask {
        recipe: TrainingRecipe::for_round(architecture, settings, 0),
        lease: grid.issue(WORKER, 0).unwrap(),
        model: encode_parameters(&parameters, architecture).unwrap(),
        text: text.into(),
    }
}

fn legacy_task() -> TrainingTask {
    task_for(
        Architecture::STARTER,
        &settings(Architecture::STARTER),
        "a garden",
    )
}

fn micro_task() -> TrainingTask {
    task_for(MICRO, &settings(MICRO), "a garden grows here")
}

#[test]
fn the_first_wire_form_is_unchanged_for_computers_on_the_first_release() {
    let task = legacy_task();
    assert_eq!(task.recipe, TrainingRecipe::ByteBigramV1);
    let wire = serde_json::to_value(&task).unwrap();
    assert_eq!(wire["recipe"], "byte_bigram_v1");
    // Sorted here: the order a JSON object lists its keys in depends on how
    // the JSON library was built, and is not part of the wire form.
    let keys: std::collections::BTreeSet<&str> = wire
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        std::collections::BTreeSet::from(["lease", "model", "recipe", "text"])
    );
    assert_eq!(
        serde_json::to_value(TrainingPeerRequest::Work).unwrap(),
        serde_json::json!({ "kind": "work" })
    );
    assert_eq!(
        serde_json::to_value(TrainingPeerResponse::Unavailable).unwrap(),
        serde_json::json!({ "kind": "unavailable" })
    );
    assert_eq!(
        serde_json::to_value(TrainingPeerResponse::Accepted(TrainingProgress {
            rounds: 1,
            total_rounds: 80,
            tokens: 28,
            loss_milli: Some(5545),
        }))
        .unwrap(),
        serde_json::json!({ "kind": "accepted", "value": {
            "rounds": 1, "total_rounds": 80, "tokens": 28, "loss_milli": 5545
        }})
    );
    // What the first release computed for tokens still holds: pairs × steps.
    assert_eq!(task.lease.steps, 4);
    assert_eq!(task.expected_tokens().unwrap(), 28);
    assert_eq!(task.recipe.max_text_bytes(), 512);
    assert_eq!(
        task.recipe.work(),
        WorkRecipe {
            architecture: Architecture::STARTER,
            windows: 1,
            learning_rate_micro: 100_000,
            optimizer: LocalOptimizer::Sgd,
            weight_decay_micro: 0,
            clip_milli: 0,
            seed: 0,
        }
    );
}

#[test]
fn a_recipe_names_the_shape_and_this_rounds_local_rule() {
    let mut s = settings(MICRO);
    s.seed = 5;
    s.schedule = LrSchedule::WarmupCosine;
    s.warmup_rounds = 10;
    s.learning_rate_micro = 3000;
    let first = TrainingRecipe::for_round(MICRO, &s, 0);
    let TrainingRecipe::ByteModelV2(recipe) = first else {
        panic!("a transformer uses the second wire form");
    };
    assert_eq!(recipe.architecture, MICRO);
    assert_eq!(recipe.windows, s.windows);
    assert_eq!(recipe.learning_rate_micro, 300);
    assert_eq!(recipe.optimizer, LocalOptimizer::Adam);
    assert_eq!(recipe.weight_decay_micro, s.weight_decay_micro);
    assert_eq!(recipe.clip_milli, s.clip_milli);
    assert_eq!(recipe.seed, 5);
    assert_eq!(first.validate(), Ok(()));
    assert_eq!(first.architecture(), MICRO);
    assert_eq!(first.max_text_bytes(), 4096);
    let later = TrainingRecipe::for_round(MICRO, &s, 10).work();
    assert_eq!(later.learning_rate_micro, 3000);
    assert_ne!(later.seed, recipe.seed);
    assert_eq!(later, TrainingRecipe::for_round(MICRO, &s, 10).work());
    // A Starter run with its own settings uses the second form too.
    let mut table = settings(Architecture::STARTER);
    table.learning_rate_micro = 50_000;
    let custom = TrainingRecipe::for_round(Architecture::STARTER, &table, 3);
    assert!(matches!(custom, TrainingRecipe::ByteModelV2(_)));
    assert_eq!(custom.validate(), Ok(()));
    assert_eq!(custom.expected_tokens(100, 4), Ok(99 * 4));
    // Tokens are windows × bytes per window × steps, and never zero.
    assert_eq!(
        first.expected_tokens(20, 2),
        Ok(u32::from(s.windows) * 8 * 2)
    );
    assert_eq!(
        first.expected_tokens(5, 2),
        Ok(u32::from(s.windows) * 4 * 2)
    );
    assert!(first.expected_tokens(1, 2).is_err());
    assert!(first.expected_tokens(20, 0).is_err());
    assert!(first.expected_tokens(20, u32::MAX).is_err());
    let wire = serde_json::to_value(first).unwrap();
    assert_eq!(wire["byte_model_v2"]["architecture"]["context"], 8);
    assert_eq!(wire["byte_model_v2"]["optimizer"], "adam");
    assert_eq!(
        serde_json::from_value::<TrainingRecipe>(wire).unwrap(),
        first
    );
}

#[test]
fn recipes_outside_the_bounds_are_refused() {
    let TrainingRecipe::ByteModelV2(good) = micro_task().recipe else {
        panic!("second wire form");
    };
    let changes: Vec<Change<WorkRecipe>> = vec![
        Box::new(|r| r.architecture.width = 9),
        Box::new(|r| r.architecture.layers = 13),
        Box::new(|r| r.architecture = BaseModel::Large.architecture()),
        Box::new(|r| r.windows = 0),
        Box::new(|r| r.windows = 65),
        Box::new(|r| {
            r.architecture = Architecture::STARTER;
            r.windows = 2;
        }),
        Box::new(|r| r.learning_rate_micro = 0),
        Box::new(|r| r.learning_rate_micro = 10_000_001),
        Box::new(|r| r.weight_decay_micro = 1_000_001),
        Box::new(|r| r.clip_milli = 1_000_001),
    ];
    for (index, change) in changes.iter().enumerate() {
        let mut altered = good;
        change(&mut altered);
        assert_eq!(
            TrainingRecipe::ByteModelV2(altered).validate(),
            Err(TrainingError::InvalidWork),
            "change {index}"
        );
        let mut task = micro_task();
        task.recipe = TrainingRecipe::ByteModelV2(altered);
        assert!(task.parameters(WORKER).is_err(), "change {index}");
    }
    for unknown in [
        serde_json::json!("run-a-command"),
        serde_json::json!({ "byte_model_v3": {} }),
        serde_json::json!({ "byte_model_v2": { "architecture": MICRO, "windows": 1 } }),
    ] {
        assert!(serde_json::from_value::<TrainingRecipe>(unknown).is_err());
    }
    let mut extra = serde_json::to_value(TrainingRecipe::ByteModelV2(good)).unwrap();
    extra["byte_model_v2"]["path"] = "/bin/sh".into();
    assert!(serde_json::from_value::<TrainingRecipe>(extra).is_err());
}

#[test]
fn tasks_reject_changed_identity_model_recipe_cost_or_batch_before_training() {
    for original in [legacy_task(), micro_task()] {
        let architecture = original.recipe.architecture();
        assert_eq!(
            original.parameters(WORKER).unwrap().len(),
            architecture.parameters()
        );
        assert!(original.parameters(DeviceId([2; 32])).is_err());
        for change in 0..9 {
            let mut altered = original.clone();
            match change {
                0 => altered.model.0[0] = 1,
                1 => altered.lease.steps = MAX_LOCAL_STEPS + 1,
                2 => altered.text = "x".repeat(original.recipe.max_text_bytes() + 1),
                3 => altered.text.clear(),
                4 => altered.lease.token_budget = 1,
                5 => altered.lease.steps = 0,
                6 => altered.text = "x".into(),
                7 => {
                    altered.model.0.extend_from_slice(&[0; 4]);
                    altered.lease.base_model = ContentHash::of(&altered.model.0);
                }
                _ => {
                    altered.model.0[..4].copy_from_slice(&f32::NAN.to_le_bytes());
                    altered.lease.base_model = ContentHash::of(&altered.model.0);
                }
            }
            assert_eq!(
                altered.parameters(WORKER),
                Err(TrainingError::InvalidWork),
                "change {change}"
            );
        }
        let mut wire = serde_json::to_value(&original).unwrap();
        wire["command"] = "run".into();
        assert!(serde_json::from_value::<TrainingTask>(wire).is_err());
    }
    // A model sized for one shape is refused under a recipe naming another.
    let mut swapped = legacy_task();
    swapped.recipe = micro_task().recipe;
    assert!(swapped.parameters(WORKER).is_err());
    let mut swapped = micro_task();
    swapped.recipe = TrainingRecipe::ByteBigramV1;
    assert!(swapped.parameters(WORKER).is_err());
    // The first wire form keeps its 512-byte packet limit.
    let s = settings(Architecture::STARTER);
    let edge = task_for(Architecture::STARTER, &s, &"x".repeat(512));
    assert!(edge.parameters(WORKER).is_ok());
    // Every recommended plan's full packet fits the budget its lease carries.
    for row in BASE_MODELS.iter().filter(|row| row.group_capable) {
        let s = settings(row.architecture);
        let task = task_for(row.architecture, &s, &"x".repeat(s.batch_bytes as usize));
        assert_eq!(task.expected_tokens().unwrap(), task.lease.token_budget);
        assert!(task.parameters(WORKER).is_ok(), "{row:?}");
    }
}

#[test]
fn returned_updates_have_exact_bounded_finite_parameters_and_owned_identity() {
    let assignment = micro_task();
    let tokens = assignment.expected_tokens().unwrap();
    let mut result = TrainingResult {
        lease: assignment.lease,
        tokens,
        pseudo_gradient: assignment.model,
    };
    let merged = result.contribution(WORKER, MICRO).unwrap();
    assert_eq!(merged.pseudo_gradient.len(), MICRO.parameters());
    assert_eq!(merged.tokens, tokens);
    assert!(result.contribution(DeviceId([2; 32]), MICRO).is_err());
    assert!(result.contribution(WORKER, Architecture::STARTER).is_err());
    result.tokens = 0;
    assert!(result.contribution(WORKER, MICRO).is_err());
    result.tokens = result.lease.token_budget + 1;
    assert!(result.contribution(WORKER, MICRO).is_err());
    result.tokens = tokens;
    result.pseudo_gradient.0.pop();
    assert!(result.contribution(WORKER, MICRO).is_err());
    result.pseudo_gradient.0.push(0);
    result.pseudo_gradient.0[..4].copy_from_slice(&f32::INFINITY.to_le_bytes());
    assert!(result.contribution(WORKER, MICRO).is_err());

    assert!(encode_parameters(&[0.0], MICRO).is_err());
    assert!(encode_parameters(&vec![f32::INFINITY; MICRO.parameters()], MICRO).is_err());
    assert!(encode_parameters(&vec![0.0; BYTE_MODEL_PARAMETERS], MICRO).is_err());
    let values: Vec<f32> = (0..MICRO.parameters())
        .map(|i| f32::from(u16::try_from(i).unwrap()))
        .collect();
    let bytes = encode_parameters(&values, MICRO).unwrap();
    assert_eq!(decode_parameters(&bytes, MICRO).unwrap(), values);
    assert!(decode_parameters(&bytes, Architecture::STARTER).is_err());
    assert_eq!(BYTE_MODEL_PARAMETERS, Architecture::STARTER.parameters());
}

#[test]
fn the_largest_packet_and_update_of_every_group_model_fit_one_message() {
    // Worst case: the most text a packet may carry, made of a character that
    // costs six bytes each in JSON, and a lease with the widest numbers.
    for row in BASE_MODELS {
        let architecture = row.architecture;
        if !row.group_capable {
            assert!(architecture.parameters() * 8 > MAX_PEER_MESSAGE_BYTES);
            continue;
        }
        let mut s = settings(architecture);
        if !architecture.is_table() {
            s.batch_bytes = 4096;
        }
        let mut task = task_for(architecture, &s, &"\u{1}".repeat(s.batch_bytes as usize));
        task.lease.id = u64::MAX;
        task.lease.base_round = u64::MAX;
        task.lease.expires_ms = u64::MAX;
        task.lease.token_budget = u32::MAX;
        task.model = Bytes(vec![0xff; architecture.parameters() * 4]);
        let update = TrainingResult {
            lease: task.lease.clone(),
            tokens: u32::MAX,
            pseudo_gradient: task.model.clone(),
        };
        let assigned = serde_json::to_vec(&PeerResponse::Training {
            work: TrainingPeerResponse::Assigned(Box::new(task)),
        })
        .unwrap();
        assert!(assigned.len() <= MAX_PEER_MESSAGE_BYTES, "{row:?}");
        let submit = serde_json::to_vec(&PeerRequest::Training {
            group: GroupId([0xff; 32]),
            work: TrainingPeerRequest::Submit(Box::new(update)),
        })
        .unwrap();
        assert!(submit.len() <= MAX_PEER_MESSAGE_BYTES, "{row:?}");
    }
}

#[test]
fn finding_a_run_and_delivering_models_use_bounded_checked_messages() {
    let card = card_for(&micro_file(0.25));
    let id = card.id;
    let requests = [
        (
            TrainingPeerRequest::Status,
            serde_json::json!({ "kind": "status" }),
        ),
        (
            TrainingPeerRequest::Offers { have: vec![id] },
            serde_json::json!({ "kind": "offers", "value": { "have": [id.to_hex()] } }),
        ),
        (
            TrainingPeerRequest::FetchModel {
                model: id,
                offset: 64,
            },
            serde_json::json!({ "kind": "fetch_model", "value": { "model": id.to_hex(), "offset": 64 } }),
        ),
        (
            TrainingPeerRequest::Received { model: id },
            serde_json::json!({ "kind": "received", "value": { "model": id.to_hex() } }),
        ),
    ];
    for (request, wire) in requests {
        assert_eq!(request.validate(), Ok(()));
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<TrainingPeerRequest>(wire).unwrap(),
            request
        );
    }
    let announce = TrainingPeerRequest::Announce {
        models: vec![card.clone(); MAX_OFFERED_MODELS],
    };
    assert_eq!(announce.validate(), Ok(()));
    let wire = serde_json::to_string(&announce).unwrap();
    assert_eq!(
        serde_json::from_str::<TrainingPeerRequest>(&wire).unwrap(),
        announce
    );

    let mut bad_card = card.clone();
    bad_card.bytes += 1;
    for bad in [
        TrainingPeerRequest::Offers {
            have: vec![id; MAX_OFFER_HAVE + 1],
        },
        TrainingPeerRequest::FetchModel {
            model: id,
            offset: MAX_MODEL_BYTES as u64,
        },
        TrainingPeerRequest::Announce {
            models: vec![card.clone(); MAX_OFFERED_MODELS + 1],
        },
        TrainingPeerRequest::Announce {
            models: vec![bad_card.clone()],
        },
    ] {
        assert_eq!(bad.validate(), Err(TrainingError::InvalidWork));
    }
    assert_eq!(
        TrainingPeerRequest::Offers {
            have: vec![id; MAX_OFFER_HAVE]
        }
        .validate(),
        Ok(())
    );

    let run = RunSummary {
        name: "Garden notes".into(),
        base: BaseModel::Small,
        architecture: MICRO,
        rounds: 3,
        total_rounds: 1000,
    };
    let replies = [
        TrainingPeerResponse::Running(run.clone()),
        TrainingPeerResponse::Offered(vec![card.clone()]),
        TrainingPeerResponse::Offered(Vec::new()),
        TrainingPeerResponse::ModelPart {
            total: 100,
            bytes: Bytes(vec![7; 100]),
        },
        TrainingPeerResponse::Thanks,
        TrainingPeerResponse::Unavailable,
        TrainingPeerResponse::Rejected,
    ];
    for reply in replies {
        assert_eq!(reply.validate(), Ok(()), "{reply:?}");
        let wire = serde_json::to_string(&reply).unwrap();
        assert_eq!(
            serde_json::from_str::<TrainingPeerResponse>(&wire).unwrap(),
            reply
        );
    }
    assert_eq!(
        serde_json::to_value(TrainingPeerResponse::Thanks).unwrap(),
        serde_json::json!({ "kind": "thanks" })
    );
    let bad_replies = [
        TrainingPeerResponse::Offered(vec![card; MAX_OFFERED_MODELS + 1]),
        TrainingPeerResponse::Offered(vec![bad_card]),
        TrainingPeerResponse::ModelPart {
            total: 100,
            bytes: Bytes(Vec::new()),
        },
        TrainingPeerResponse::ModelPart {
            total: 100,
            bytes: Bytes(vec![0; 101]),
        },
        TrainingPeerResponse::ModelPart {
            total: MAX_MODEL_BYTES as u64 + 1,
            bytes: Bytes(vec![0; 4]),
        },
        TrainingPeerResponse::ModelPart {
            total: MAX_MODEL_BYTES as u64,
            bytes: Bytes(vec![0; MAX_MODEL_PART_BYTES + 1]),
        },
        TrainingPeerResponse::Running(RunSummary {
            name: String::new(),
            ..run.clone()
        }),
        TrainingPeerResponse::Running(RunSummary {
            rounds: 1001,
            ..run.clone()
        }),
        TrainingPeerResponse::Running(RunSummary {
            total_rounds: 100_001,
            ..run.clone()
        }),
        TrainingPeerResponse::Running(RunSummary {
            base: BaseModel::Starter,
            ..run.clone()
        }),
        TrainingPeerResponse::Running(RunSummary {
            architecture: BaseModel::Large.architecture(),
            ..run.clone()
        }),
        TrainingPeerResponse::Running(RunSummary {
            architecture: Architecture { width: 9, ..MICRO },
            ..run
        }),
    ];
    for (index, reply) in bad_replies.iter().enumerate() {
        assert_eq!(
            reply.validate(),
            Err(TrainingError::InvalidWork),
            "reply {index}"
        );
    }
    // Unknown kinds and extra fields do not decode at all.
    for unknown in [
        serde_json::json!({ "kind": "run", "value": "sh" }),
        serde_json::json!({ "kind": "status", "extra": 1 }),
        serde_json::json!({ "kind": "fetch_model", "value": { "model": id.to_hex(), "offset": 0, "path": "/etc" } }),
    ] {
        assert!(serde_json::from_value::<TrainingPeerRequest>(unknown).is_err());
    }
}

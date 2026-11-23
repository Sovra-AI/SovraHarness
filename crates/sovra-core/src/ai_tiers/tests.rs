use super::*;

fn key(id: &str) -> ModelKey {
    ModelKey {
        connection_id: "openrouter".into(),
        model_id: id.into(),
    }
}

#[test]
fn quick_sorter_keeps_greetings_simple_and_code_complex() {
    assert_eq!(quick_sort("hi").tier, ModelTier::Fast);
    assert_eq!(quick_sort("Thanks!").tier, ModelTier::Fast);
    assert_eq!(quick_sort("مرحبا").tier, ModelTier::Fast);
    let code = "Why does this fail?\n```rust\nfn main() { let x: u8 = 300; }\n```";
    let sorted = quick_sort(code);
    assert_eq!(sorted.tier, ModelTier::Deep);
    assert!(sorted.confidence >= QUICK_SURE);
}

#[test]
fn quick_sorter_reads_whole_words_not_fragments() {
    // "current" contains "rent" and "planet" contains "plan": neither is a hard word.
    let sorted = quick_sort("What is the current planet count?");
    assert_eq!(sorted.tier, ModelTier::Fast);
}

#[test]
fn quick_sorter_separates_everyday_and_multi_step_work() {
    assert_eq!(
        quick_sort("Write a short email to my landlord about the broken heater").tier,
        ModelTier::Standard
    );
    assert_eq!(
        quick_sort(
            "Design an algorithm to schedule 40 nurses across three shifts, then compare it with a greedy approach and explain the trade-offs"
        )
        .tier,
        ModelTier::Deep
    );
}

#[test]
fn asking_overrides_every_sorter() {
    let hard_laya = Some(LayaJudgement {
        difficulty: 3.0,
        confidence: 0.99,
    });
    let decision = decide_tier("Quickly, what's 2+2?", hard_laya);
    assert_eq!(decision.tier, ModelTier::Fast);
    assert_eq!(decision.source, DecisionSource::Asked);

    let easy_laya = Some(LayaJudgement {
        difficulty: 0.0,
        confidence: 0.99,
    });
    let decision = decide_tier("Think hard: is 91 prime?", easy_laya);
    assert_eq!(decision.tier, ModelTier::Deep);
    assert_eq!(decision.source, DecisionSource::Asked);
}

#[test]
fn laya_decides_unsure_cases_and_is_ignored_when_unsure_itself() {
    let prompt = "What should I cook for six people tonight with chickpeas and spinach?";
    assert!(quick_sort(prompt).confidence < QUICK_SURE);
    let decision = decide_tier(
        prompt,
        Some(LayaJudgement {
            difficulty: 2.7,
            confidence: 0.6,
        }),
    );
    assert_eq!(
        (decision.tier, decision.source),
        (ModelTier::Deep, DecisionSource::Laya)
    );
    assert!(decision.reason.contains("60%"), "{}", decision.reason);

    let unsure = decide_tier(
        prompt,
        Some(LayaJudgement {
            difficulty: 2.7,
            confidence: 0.05,
        }),
    );
    assert_eq!(unsure.source, DecisionSource::QuickSorter);

    let broken = decide_tier(
        prompt,
        Some(LayaJudgement {
            difficulty: f32::NAN,
            confidence: 0.9,
        }),
    );
    assert_eq!(broken.source, DecisionSource::QuickSorter);
}

#[test]
fn a_firm_complex_verdict_is_not_lowered_by_laya() {
    let prompt = "Plan a two-week trip across Japan for a family with a toddler, with budget, trains and rest days";
    let decision = decide_tier(
        prompt,
        Some(LayaJudgement {
            difficulty: 1.84,
            confidence: 0.3,
        }),
    );
    assert_eq!(
        (decision.tier, decision.source),
        (ModelTier::Deep, DecisionSource::QuickSorter)
    );
    // Short proofs are complex too.
    assert_eq!(
        quick_sort("Prove that there are infinitely many primes").tier,
        ModelTier::Deep
    );
}

#[test]
fn sure_quick_verdicts_are_not_second_guessed_by_laya() {
    let decision = decide_tier(
        "hello",
        Some(LayaJudgement {
            difficulty: 3.0,
            confidence: 0.9,
        }),
    );
    assert_eq!(
        (decision.tier, decision.source),
        (ModelTier::Fast, DecisionSource::QuickSorter)
    );
}

#[test]
fn laya_levels_round_onto_three_tiers() {
    // Bands observed from the real checkpoint (see `tier_for_laya`).
    assert_eq!(tier_for_laya(0.05), ModelTier::Fast);
    assert_eq!(tier_for_laya(0.44), ModelTier::Fast);
    assert_eq!(tier_for_laya(1.3), ModelTier::Standard);
    assert_eq!(tier_for_laya(2.08), ModelTier::Standard);
    assert_eq!(tier_for_laya(2.14), ModelTier::Deep);
    assert_eq!(tier_for_laya(3.0), ModelTier::Deep);
    assert_eq!(LAYA_DIFFICULTY_LEVELS.len(), 4);
}

#[test]
fn routing_off_uses_the_chat_model_and_its_thinking_level() {
    let mut routing = TierRouting {
        chat_thinking: ThinkingLevel::High,
        ..TierRouting::default()
    };
    routing.tiers.insert(
        ModelTier::Fast,
        TierChoice {
            model: Some(key("cheap")),
            thinking: ThinkingLevel::Off,
        },
    );
    let chat = key("chat");
    assert_eq!(
        routing.resolve(ModelTier::Fast, Some(&chat)),
        Some((chat.clone(), ThinkingLevel::High))
    );
    assert_eq!(
        routing.executions(Some(&chat)),
        vec![(chat, ThinkingLevel::High)]
    );
}

#[test]
fn a_tier_without_a_model_borrows_the_chat_model_but_keeps_its_thinking() {
    let mut routing = TierRouting {
        enabled: true,
        ..TierRouting::default()
    };
    routing.tiers.insert(
        ModelTier::Fast,
        TierChoice {
            model: Some(key("cheap")),
            thinking: ThinkingLevel::Off,
        },
    );
    let chat = key("chat");
    assert_eq!(
        routing.resolve(ModelTier::Fast, Some(&chat)),
        Some((key("cheap"), ThinkingLevel::Off))
    );
    assert_eq!(
        routing.resolve(ModelTier::Deep, Some(&chat)),
        Some((chat.clone(), ThinkingLevel::High))
    );
    assert_eq!(routing.resolve(ModelTier::Deep, None), None);
    assert_eq!(routing.executions(Some(&chat)).len(), 3);
}

#[test]
fn forgetting_a_model_or_connection_clears_only_matching_tiers() {
    let mut routing = TierRouting::default();
    routing.tiers.insert(
        ModelTier::Fast,
        TierChoice {
            model: Some(key("cheap")),
            thinking: ThinkingLevel::Off,
        },
    );
    routing.tiers.insert(
        ModelTier::Deep,
        TierChoice {
            model: Some(ModelKey {
                connection_id: "ollama".into(),
                model_id: "qwen3".into(),
            }),
            thinking: ThinkingLevel::Max,
        },
    );
    routing.forget_model(&key("cheap"));
    assert_eq!(routing.choice(ModelTier::Fast).model, None);
    assert_eq!(routing.choice(ModelTier::Fast).thinking, ThinkingLevel::Off);
    assert!(routing.choice(ModelTier::Deep).model.is_some());
    routing.forget_connection("ollama");
    assert_eq!(routing.choice(ModelTier::Deep).model, None);
}

#[test]
fn saved_routing_round_trips_and_older_files_load_with_defaults() {
    let mut routing = TierRouting {
        enabled: true,
        sorter: Sorter::Laya,
        ..TierRouting::default()
    };
    routing.tiers.insert(
        ModelTier::Standard,
        TierChoice {
            model: Some(key("mid")),
            thinking: ThinkingLevel::Medium,
        },
    );
    let json = serde_json::to_string(&routing).expect("routing serializes");
    let back: TierRouting = serde_json::from_str(&json).expect("routing parses");
    assert_eq!(back, routing);
    let empty: TierRouting = serde_json::from_str("{}").expect("older file parses");
    assert!(!empty.enabled);
    assert_eq!(empty.choice(ModelTier::Deep).thinking, ThinkingLevel::High);
}

#[test]
fn laya_files_are_pinned_and_fully_hashed() {
    assert_eq!(LAYA_REVISION.len(), 40);
    for file in LAYA_FILES {
        assert_eq!(file.sha256.len(), 64, "{}", file.remote);
        assert!(
            file.sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert!(!file.local.contains(".."));
    }
    assert!(laya_download_bytes() > 600_000_000);
}

#[test]
fn every_thinking_level_has_plain_copy() {
    for level in ThinkingLevel::ALL {
        assert_ne!(level.label(), "");
        assert!(level.description().ends_with('.'));
    }
    assert_eq!(ThinkingLevel::ModelDefault.wire_name(), None);
    assert_eq!(ThinkingLevel::Max.wire_name(), Some("max"));
}

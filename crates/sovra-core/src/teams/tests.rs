use super::*;

fn team() -> Vec<TeamMember> {
    default_team()
}

#[test]
fn default_team_has_one_conductor_and_unique_handles() {
    let members = team();
    assert_eq!(members.iter().filter(|m| m.conductor).count(), 1);
    let mut ids: Vec<&str> = members.iter().map(|m| m.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), members.len());
    assert!(members.iter().all(|m| m.builtin && m.active));
    assert!(helper_templates().iter().all(|t| !t.builtin));
    assert_eq!(
        builtin_member("priya").map(|m| m.name),
        Some("Priya".into())
    );
    assert_eq!(builtin_member("theo"), None);
}

#[test]
fn mentions_then_names_then_everyone_then_expertise_choose_speakers() {
    let members = team();
    assert_eq!(
        first_round_speakers("@jamie can you help with a letter", &members),
        ["jamie"]
    );
    assert_eq!(
        first_round_speakers("What do Priya and Kai think?", &members),
        ["priya", "kai"]
    );
    let everyone = first_round_speakers("What does everyone think?", &members);
    assert_eq!(everyone.len(), members.len() - 1, "all but the Conductor");
    assert!(!everyone.contains(&"alex".to_owned()));
    let health = first_round_speakers("I can't sleep and feel stress", &members);
    assert_eq!(health.first().map(String::as_str), Some("sam"));
    assert!((2..=3).contains(&health.len()));
    assert_eq!(first_round_speakers("hello", &members).len(), 2);
}

#[test]
fn mentions_ignore_longer_handles_and_inactive_members() {
    let mut members = team();
    assert!(!has_mention("ask @samuel", "sam"));
    assert!(mentions_member("Ask @Sam please", &members));
    for m in &mut members {
        m.active = m.id != "sam";
    }
    assert!(!mentions_member("Ask @sam please", &members));
    assert_ne!(first_round_speakers("@sam sleep", &members), ["sam"]);
}

#[test]
fn team_command_and_conductor_rules() {
    assert_eq!(strip_team_command("/team plan a trip"), Some("plan a trip"));
    assert_eq!(strip_team_command("/teamwork"), None);
    let members = team();
    assert!(conductor_for(&members, &["sam".into()]).is_none());
    assert_eq!(
        conductor_for(&members, &["sam".into(), "kai".into()]).map(|m| m.id.as_str()),
        Some("alex")
    );
}

#[test]
fn conductor_tail_is_parsed_and_hidden() {
    let (text, decision) = parse_conductor_reply(
        "Let's compare costs.\n[[CONDUCTOR: {\"verdict\":\"CONTINUE\",\"next_speakers\":[\"@morgan\",\"ghost\",\"alex\"]}]]",
    );
    assert_eq!(text, "Let's compare costs.");
    assert_eq!(decision.verdict, Verdict::Continue);
    let members = team();
    assert_eq!(
        next_round_speakers(&decision, &members, &["priya".into()]),
        ["morgan"]
    );
    let (text, decision) = parse_conductor_reply("Summary without a tail");
    assert_eq!(text, "Summary without a tail");
    assert_eq!(decision.verdict, Verdict::YieldToUser);
    let (_, broken) = parse_conductor_reply("x [[CONDUCTOR: {nope]]");
    assert_eq!(broken.verdict, Verdict::YieldToUser);
    let none = ConductorDecision {
        verdict: Verdict::Continue,
        next_speakers: vec![],
    };
    assert_eq!(
        next_round_speakers(&none, &members, &["a".into(), "b".into(), "c".into()]),
        ["a", "b"]
    );
}

#[test]
fn visible_stream_never_shows_a_pass_or_the_tail() {
    let mut pass = VisibleStream::default();
    let shown: String = ["<pa", "ss", "/>"].iter().map(|d| pass.push(d)).collect();
    assert_eq!(shown + &pass.finish(), "");
    assert!(is_pass(pass.full_text()));

    let mut reply = VisibleStream::default();
    let mut shown = String::new();
    for delta in [
        "Hello ",
        "there [",
        "[COND",
        "UCTOR: {\"verdict\":\"YIELD_TO_USER\"}]]",
    ] {
        shown.push_str(&reply.push(delta));
    }
    shown.push_str(&reply.finish());
    assert_eq!(shown, "Hello there ");

    let mut normal = VisibleStream::default();
    let mut shown = normal.push("A [link]");
    shown.push_str(&normal.push(" ok"));
    shown.push_str(&normal.finish());
    assert_eq!(shown, "A [link] ok");

    let mut unicode = VisibleStream::default();
    let shown = unicode.push("Café ☕") + &unicode.finish();
    assert_eq!(shown, "Café ☕");
}

#[test]
fn instructions_include_protocols_only_where_they_apply() {
    let members = team();
    let sam = members.iter().find(|m| m.id == "sam").expect("sam");
    let alex = members.iter().find(|m| m.conductor).expect("conductor");
    let moderated = member_instruction(sam, &members, true, "Ann");
    assert!(moderated.contains(PASS_TOKEN));
    assert!(moderated.contains("@kai"));
    assert!(!member_instruction(sam, &members, false, "Ann").contains(PASS_TOKEN));
    let lead = member_instruction(alex, &members, true, "Ann");
    assert!(lead.contains(CONDUCTOR_TAIL));
    assert!(!lead.contains("# ADD OR SKIP"));
    let history = vec![TranscriptLine {
        speaker: "User".into(),
        text: "Hi".into(),
    }];
    assert!(member_prompt(&history, "Continue", 1, false).contains("[User]: Hi"));
}

#[test]
fn roster_and_activity_track_changes() {
    let mut roster = TeamRoster::default();
    roster.apply(&TeamEvent::Loaded { members: team() });
    let mut theo = helper_templates().remove(0);
    theo.id = member_id_for("Theo", &roster.members);
    roster.apply(&TeamEvent::MemberSaved(theo));
    assert_eq!(
        roster.get("theo").map(|m| m.role.as_str()),
        Some("Tech helper")
    );
    assert_eq!(member_id_for("Theo", &roster.members), "theo2");
    assert_eq!(member_id_for("!!", &roster.members), "helper");
    roster.apply(&TeamEvent::MemberRemoved { id: "theo".into() });
    assert!(roster.get("theo").is_none());

    let members = team();
    let mut activity = TeamActivity::new(1, &members, &["sam".into(), "kai".into()]);
    activity.set("sam", SpeakerState::Speaking);
    assert_eq!(activity.status, "Sam is answering…");
    activity.set("sam", SpeakerState::Done);
    activity.finish("Your team is done");
    assert!(activity.finished);
    assert_eq!(activity.speakers[1].state, SpeakerState::Passed);
}

#[test]
fn team_prompt_preserves_the_current_request_while_bounding_earlier_text() {
    let current = format!(
        "Long current request {} END OF CURRENT REQUEST",
        "茶".repeat(9000)
    );
    let history = vec![TranscriptLine {
        speaker: "User".into(),
        text: "Old context".repeat(10000),
    }];
    let prompt = member_prompt(&history, &current, 1, false);
    assert!(prompt.contains(&current));
    assert!(prompt.contains("Earlier conversation omitted or shortened"));
    assert!(prompt.len() < current.len() + crate::MAX_CONTEXT_TEXT_BYTES + 1024);
    assert!(member_prompt(&[], "What shall we build?", 1, false).contains("What shall we build?"));
}

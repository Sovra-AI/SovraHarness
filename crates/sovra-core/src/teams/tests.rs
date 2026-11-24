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

fn group() -> TeamRoom {
    TeamRoom::all_helpers()
}

fn speakers(prompt: &str, members: &[TeamMember]) -> Vec<String> {
    route_speakers(prompt, members, &group(), &[])
}

fn non_leads(members: &[TeamMember]) -> Vec<String> {
    members
        .iter()
        .filter(|member| member.active && !member.conductor)
        .map(|member| member.id.clone())
        .collect()
}

#[test]
fn a_question_picks_as_few_or_as_many_helpers_as_it_needs() {
    let members = team();
    let rooms = [TeamRoom::ordinary(), group()];
    for room in &rooms {
        assert_eq!(
            route_speakers("I can't sleep and feel stress", &members, room, &[]),
            ["sam"],
            "{room:?}"
        );
        // A stronger writing match beats the stray garden word.
        assert_eq!(
            route_speakers("Write a letter about the garden", &members, room, &[]),
            ["jamie"],
            "{room:?}"
        );
        assert_eq!(
            route_speakers("Plan my garden", &members, room, &[]),
            ["morgan", "robin", "alex"],
            "{room:?}"
        );
        assert_eq!(
            route_speakers(
                "Research the history of tea. Research the science of coffee.",
                &members,
                room,
                &[]
            ),
            ["priya", "priya"],
            "{room:?}"
        );
        let many = route_speakers(
            "Research the history of tea. Research the science of coffee. I can't sleep and I feel stress. Write a letter. Plan the budget. Pack for the trip. Check this contract for a scam.",
            &members,
            room,
            &[],
        );
        assert_eq!(
            many,
            [
                "priya", "priya", "sam", "jamie", "morgan", "robin", "kai", "alex",
            ],
            "{room:?}"
        );
        assert!(!many.iter().any(|id| id == "noor"), "{many:?}");
        let hello = route_speakers("hello", &members, room, &[]);
        assert_eq!(hello.len(), 1, "{hello:?}");
        assert_ne!(hello, ["alex"]);
    }
}

#[test]
fn route_speakers_follows_sovra_srx_precedence() {
    let members = team();
    let leads_and = |mut ids: Vec<&str>| {
        ids.push("alex");
        ids.into_iter().map(str::to_owned).collect::<Vec<_>>()
    };
    assert_eq!(
        speakers("@jamie can you help with a letter", &members),
        ["jamie"]
    );
    assert_eq!(speakers("@alex what is next", &members), ["alex"]);
    assert_eq!(
        speakers("What do Priya and Kai think?", &members),
        leads_and(vec!["priya", "kai"])
    );
    assert_eq!(
        speakers("Kai and Priya, your thoughts", &members),
        leads_and(vec!["kai", "priya"])
    );
    assert_eq!(speakers("Alex and Morgan", &members), ["morgan"]);
    assert_eq!(speakers("Morgan, what do you think?", &members), ["morgan"]);
    assert_eq!(speakers("I can't sleep and feel stress", &members), ["sam"]);
    assert_eq!(speakers("hello", &members), ["morgan"]);
    // A short question still asks one helper. The Skills example is several
    // jobs, so it asks every helper those jobs need, including Priya twice.
    assert_eq!(
        route_speakers("hello", &members, &TeamRoom::ordinary(), &[]),
        ["morgan"]
    );
    let example = crate::skills::builtin_skills()
        .into_iter()
        .find(|skill| skill.name == "team")
        .expect("team skill")
        .friendly_card_copy()
        .sample_draft;
    let asked = strip_team_command(&example).expect("the skills example asks the team");
    let many = [
        "priya", "priya", "sam", "jamie", "morgan", "robin", "kai", "alex",
    ];
    assert_eq!(
        route_speakers(asked, &members, &TeamRoom::ordinary(), &[]),
        many
    );
    assert_eq!(route_speakers(asked, &members, &group(), &[]), many);
    assert_eq!(
        route_speakers(
            "Write a letter. Write a card. Research the history of tea.",
            &members,
            &TeamRoom::group(["jamie"]),
            &[]
        ),
        ["jamie", "jamie"]
    );
    assert_eq!(
        route_speakers("@jamie write a card", &members, &TeamRoom::ordinary(), &[]),
        ["jamie"]
    );

    let everyone = non_leads(&members);
    // "Everyone" still stops at the reply cap. The lead stays last, so a larger
    // roster does not push Alex out or invite every new helper past the cap.
    let with_lead = leads_and(
        everyone
            .iter()
            .map(String::as_str)
            .take(MAX_REPLIES_PER_TURN.saturating_sub(1))
            .collect(),
    );
    for phrase in [
        "team",
        "everyone",
        "everybody",
        "all of you",
        "all   of    you",
        "all three",
        "all four",
        "y'all",
        "yall",
        "the group",
        "the room",
        "who wants",
        "who thinks",
        "debate",
        "debates",
        "debated",
        "debate this",
        "round-table",
        "round table",
        "roundtable",
        "all hands",
        "What does the team think?",
    ] {
        assert_eq!(speakers(phrase, &members), with_lead, "{phrase}");
    }
    for phrase in [
        "steam",
        "teammate",
        "debating the menu",
        "all of your ideas",
    ] {
        let chosen = speakers(phrase, &members);
        assert_eq!(chosen.len(), 1, "{phrase} -> {chosen:?}");
        assert_ne!(chosen, ["alex"], "{phrase}");
    }

    let mut quiet = members.clone();
    for member in &mut quiet {
        if member.id == "sam" {
            member.active = false;
        }
    }
    assert!(!speakers("@sam please", &quiet).contains(&"sam".into()));
    assert_eq!(
        speakers("I can't sleep and feel stress", &quiet),
        ["morgan"]
    );

    assert_eq!(
        route_speakers("everyone @sam", &members, &TeamRoom::direct("jamie"), &[]),
        ["jamie"]
    );
    let mut off = members.clone();
    off.iter_mut()
        .find(|member| member.id == "jamie")
        .expect("jamie")
        .active = false;
    assert_eq!(
        route_speakers("hello", &off, &TeamRoom::direct("jamie"), &[]),
        Vec::<String>::new()
    );

    let asked = TeamRoom::all_helpers().with_ask_everyone(true);
    assert_eq!(route_speakers("hello", &members, &asked, &[]), with_lead);
    let chosen = TeamRoom::group(["priya", "morgan"]);
    assert_eq!(
        route_speakers(
            "what does everyone think about sleep",
            &members,
            &chosen,
            &[]
        ),
        ["morgan", "priya", "alex"]
    );
    assert_eq!(route_speakers("hello", &members, &chosen, &[]), ["morgan"]);
    // Two specialities in one question ask both helpers. Who spoke last does
    // not drop a match; that wait only applies when nobody matches.
    assert_eq!(
        route_speakers("Plan my garden", &members, &group(), &[]),
        ["morgan", "robin", "alex"]
    );
    let morgan_spoke = vec![TranscriptLine {
        source: None,
        speaker: "Morgan".into(),
        text: "An earlier plan".into(),
    }];
    assert_eq!(
        route_speakers("Plan my garden", &members, &group(), &morgan_spoke),
        ["morgan", "robin", "alex"]
    );
    assert_eq!(
        route_speakers("hello", &members, &group(), &morgan_spoke),
        ["priya"]
    );
    let both_spoke = vec![
        TranscriptLine {
            source: None,
            speaker: "morgan".into(),
            text: "First".into(),
        },
        TranscriptLine {
            source: None,
            speaker: "Priya".into(),
            text: "Later".into(),
        },
    ];
    assert_eq!(
        route_speakers("hello", &members, &group(), &both_spoke),
        ["jamie"]
    );
    let all_spoke: Vec<TranscriptLine> = members
        .iter()
        .filter(|member| !member.conductor)
        .map(|member| TranscriptLine {
            source: None,
            speaker: member.name.clone(),
            text: "Said something".into(),
        })
        .collect();
    assert_eq!(
        route_speakers("hello", &members, &group(), &all_spoke),
        ["morgan"]
    );
}

#[test]
fn continue_hands_back_closes_a_repeating_or_thin_round() {
    let fresh = |confidence: Option<f64>| {
        let tail = match confidence {
            Some(value) => format!(
                "[[CONDUCTOR: {{\"verdict\":\"CONTINUE\",\"next_speakers\":[\"morgan\"],\"confidence\":{value}}}]]"
            ),
            None => {
                "[[CONDUCTOR: {\"verdict\":\"CONTINUE\",\"next_speakers\":[\"morgan\"]}]]".into()
            }
        };
        parse_conductor_reply(&format!("Still going. {tail}")).1
    };
    let different = vec![round_signature("alpha beta gamma")];
    let other = vec![round_signature("delta epsilon zeta")];
    assert!(continue_hands_back(
        2,
        &fresh(Some(0.649)),
        &different,
        &other
    ));
    assert!(!continue_hands_back(
        2,
        &fresh(Some(0.65)),
        &different,
        &other
    ));
    assert!(!continue_hands_back(
        1,
        &fresh(Some(0.1)),
        &different,
        &other
    ));
    let omitted = fresh(None);
    assert_eq!(omitted.confidence, Confidence::FULL);
    assert!(!continue_hands_back(2, &omitted, &different, &other));
    let (_, broken) = parse_conductor_reply(
        "x [[CONDUCTOR: {\"verdict\":\"CONTINUE\",\"confidence\":\"high\"}]]",
    );
    assert_eq!(broken.verdict, Verdict::YieldToUser);

    let same = round_signature("one two three");
    assert!(continue_hands_back(
        2,
        &fresh(Some(0.9)),
        std::slice::from_ref(&same),
        std::slice::from_ref(&same)
    ));
    assert!(!continue_hands_back(
        2,
        &fresh(Some(0.9)),
        &different,
        &other
    ));

    let shared: Vec<String> = (0..10).map(|index| format!("w{index}")).collect();
    let mut extended = shared.clone();
    extended.push("n1".into());
    extended.push("n2".into());
    let extra = vec![round_signature("x y z")];
    let mut previous = vec![round_signature(&shared.join(" "))];
    previous.extend(extra);
    let current = vec![round_signature(&extended.join(" "))];
    assert!(
        !continue_hands_back(2, &fresh(Some(0.9)), &previous, &current),
        "exactly a fifth new, and overlap under 0.8, stays open"
    );

    let ten: Vec<String> = (0..10).map(|index| format!("w{index}")).collect();
    let mut twelve = ten.clone();
    twelve.push("n1".into());
    twelve.push("n2".into());
    assert!(
        continue_hands_back(
            2,
            &fresh(Some(0.9)),
            &[round_signature(&ten.join(" "))],
            &[round_signature(&twelve.join(" "))],
        ),
        "overlap of 0.8 hands back even when exactly a fifth of the wording is new"
    );

    let long: Vec<String> = (0..15).map(|index| format!("w{index}")).collect();
    let short = long[..10].join(" ");
    assert!(continue_hands_back(
        2,
        &fresh(Some(0.9)),
        &[round_signature(&long.join(" "))],
        &[round_signature(&short)],
    ));
}

#[test]
fn two_copies_of_one_helper_finish_on_their_own_rows() {
    let members = team();
    let mut activity = TeamActivity::new(1, &members, &["priya".into(), "priya".into()]);
    assert_eq!(activity.speakers.len(), 2);
    activity.start_together(&["priya".into(), "priya".into()]);
    assert_eq!(activity.speaking_count(), 2);
    activity.set_doing("priya", "First part");
    activity.set_doing("priya", "Second part");
    let doing: Vec<_> = activity
        .speakers
        .iter()
        .map(|row| row.doing.as_deref())
        .collect();
    assert_eq!(doing, [Some("First part"), Some("Second part")]);
    activity.set("priya", SpeakerState::Done);
    assert_eq!(activity.speaking_count(), 1);
    activity.set("priya", SpeakerState::Done);
    assert_eq!(activity.speaking_count(), 0);
    assert!(
        activity
            .speakers
            .iter()
            .all(|row| row.state == SpeakerState::Done)
    );
    activity.queue(&members, &["priya".into()]);
    assert_eq!(activity.speakers.len(), 2);
    assert_eq!(
        activity
            .speakers
            .iter()
            .filter(|row| row.state == SpeakerState::Waiting)
            .count(),
        1
    );
    activity.start_together(&["priya".into()]);
    assert_eq!(activity.speaking_count(), 1);
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
    assert_ne!(speakers("@sam sleep", &members), ["sam"]);
}

#[test]
fn team_command_and_conductor_rules() {
    assert_eq!(strip_team_command("/team plan a trip"), Some("plan a trip"));
    assert_eq!(strip_team_command("/teamwork"), None);
    let members = team();
    assert!(conductor_for(&members, &["sam".into()]).is_none());
    assert!(conductor_for(&members, &["sam".into(), "kai".into()]).is_none());
    assert_eq!(
        conductor_for(&members, &["sam".into(), "kai".into(), "alex".into()])
            .map(|m| m.id.as_str()),
        Some("alex")
    );
    assert_eq!(
        conductor_for(&members, &["alex".into()]).map(|m| m.id.as_str()),
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
        confidence: Confidence::FULL,
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
fn a_later_round_stays_with_the_helpers_in_that_chat() {
    let members = team();
    let room = TeamRoom::group(["morgan", "priya"]);
    let chat: Vec<TeamMember> = chat_members(&members, &room).into_iter().cloned().collect();
    let ids: Vec<&str> = chat.iter().map(|member| member.id.as_str()).collect();
    assert_eq!(ids, ["alex", "morgan", "priya"]);
    let decision = ConductorDecision {
        verdict: Verdict::Continue,
        next_speakers: vec!["sam".into(), "@kai".into()],
        confidence: Confidence::FULL,
    };
    let next = next_round_speakers(
        &decision,
        &chat,
        &["morgan".into(), "priya".into(), "alex".into()],
    );
    assert_eq!(next, ["morgan", "priya"]);
    let lead = chat.iter().find(|member| member.conductor).expect("lead");
    let told = member_instruction(lead, &chat, true, "Ann");
    assert!(told.contains("@morgan"), "{told}");
    assert!(told.contains("@priya"), "{told}");
    assert!(!told.contains("@sam"), "{told}");
    assert!(!told.contains("@kai"), "{told}");
    let direct: Vec<&str> = chat_members(&members, &TeamRoom::direct("sam"))
        .iter()
        .map(|member| member.id.as_str())
        .collect();
    assert_eq!(direct, ["sam"]);
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
        source: None,
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
    activity.set("kai", SpeakerState::Passed);
    assert_eq!(activity.status, "Kai: Nothing to add · 1 still answering");
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
        source: None,
        speaker: "User".into(),
        text: "Old context".repeat(10000),
    }];
    let prompt = member_prompt(&history, &current, 1, false);
    assert!(prompt.contains(&current));
    assert!(prompt.contains("Earlier conversation omitted or shortened"));
    assert!(prompt.len() < current.len() + crate::MAX_CONTEXT_TEXT_BYTES + 1024);
    assert!(member_prompt(&[], "What shall we build?", 1, false).contains("What shall we build?"));
}

#[test]
fn members_answer_side_by_side_on_a_kanban_board() {
    let members = team();
    let ids: Vec<String> = members.iter().take(3).map(|m| m.id.clone()).collect();
    let names: Vec<String> = members.iter().take(3).map(|m| m.name.clone()).collect();
    let mut activity = TeamActivity::new(1, &members, &ids);
    assert_eq!(activity.lanes().waiting.len(), 3);
    activity.start_together(&ids);
    assert_eq!(
        activity.status,
        format!(
            "{}, {} and {} are answering at the same time…",
            names[0], names[1], names[2]
        )
    );
    let lanes = activity.lanes();
    assert_eq!((lanes.waiting.len(), lanes.working.len()), (0, 3));
    activity.set_doing(&ids[0], "Searching the web for “soup”");
    assert_eq!(
        activity.speakers[0].doing.as_deref(),
        Some("Searching the web for “soup”")
    );
    assert_eq!(activity.speakers[0].steps, 1);
    activity.set(&ids[0], SpeakerState::Done);
    assert_eq!(activity.speakers[0].doing, None, "a finished card is quiet");
    assert_eq!(
        activity.status,
        format!("{} is done · 2 still answering", names[0])
    );
    activity.set(&ids[1], SpeakerState::Passed);
    let lanes = activity.lanes();
    assert_eq!((lanes.working.len(), lanes.done.len()), (1, 2));
}

#[test]
fn older_saved_team_cards_still_load() {
    let saved = r#"{"round":1,"max_rounds":3,"status":"Done","finished":true,
        "speakers":[{"member_id":"kai","name":"Kai","icon":"","role":"r","state":"done"}]}"#;
    let activity: TeamActivity = serde_json::from_str(saved).expect("old card");
    assert_eq!(activity.speakers[0].steps, 0);
    assert_eq!(activity.speakers[0].doing, None);
}

#[test]
fn the_starting_team_has_the_named_helpers_and_the_eight_roles() {
    let members = team();
    let names: Vec<_> = members.iter().map(|member| member.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Alex", "Morgan", "Priya", "Jamie", "Sam", "Robin", "Noor", "Kai", "Scout", "Quinn",
            "Mara",
        ]
    );
    let mut ids: Vec<_> = members.iter().map(|member| member.id.as_str()).collect();
    let unique = {
        ids.sort_unstable();
        let mut unique = ids.clone();
        unique.dedup();
        unique
    };
    assert_eq!(ids, unique);
    assert!(members.iter().all(|member| member.personality.is_some()));
    let roles: Vec<_> = helper_templates()
        .into_iter()
        .map(|member| member.role)
        .collect();
    for role in [
        "Tech helper",
        "Engineer",
        "Designer",
        "Sales",
        "Operations",
        "Skeptic",
        "Visionary",
        "Diplomat",
        "Hustler",
    ] {
        assert!(roles.iter().any(|item| item == role), "{role}");
    }
}

#[test]
fn a_speciality_question_can_choose_scout_quinn_or_mara() {
    let members = team();
    assert_eq!(
        speakers("Search the web for the latest news", &members),
        ["scout"]
    );
    assert_eq!(
        speakers("Which price is cheaper after shipping?", &members),
        ["quinn"]
    );
    assert_eq!(
        speakers("Send this on WhatsApp and pick the right channel", &members),
        ["mara"]
    );
    assert_eq!(speakers("I can't sleep and feel stress", &members), ["sam"]);
    assert_eq!(
        speakers("Write a letter about the web", &members),
        ["jamie"],
        "one stray web word does not pull Scout in"
    );
    assert_eq!(
        speakers(
            "Search the web. Which price is cheaper? Send this on WhatsApp.",
            &members
        ),
        ["scout", "quinn", "mara", "alex"]
    );
    let room = TeamRoom::group(["scout", "quinn"]);
    let chat: Vec<TeamMember> = chat_members(&members, &room).into_iter().cloned().collect();
    let scout = chat
        .iter()
        .find(|member| member.id == "scout")
        .expect("scout");
    let told = member_instruction(scout, &chat, true, "Ann");
    assert!(told.contains("@quinn"), "{told}");
    assert!(told.contains("@alex"), "{told}");
    assert!(!told.contains("@sam"), "{told}");
    assert!(told.contains("# YOUR LENS"), "{told}");
    assert!(told.contains("# WHERE YOU STEP BACK"), "{told}");
    assert!(told.contains(PASS_TOKEN), "{told}");
}

#[test]
fn personalities_keep_each_voice_and_an_older_roster_still_loads() {
    let members = team();
    let told = |id: &str| {
        let member = members.iter().find(|member| member.id == id).expect(id);
        member_instruction(member, &members, true, "Ann")
    };
    let morgan = told("morgan");
    assert!(morgan.contains("Decision time:"), "{morgan}");
    assert!(morgan.contains("leverage"), "{morgan}");
    assert!(morgan.contains("Leads with the decision"), "{morgan}");
    let priya = told("priya");
    assert!(
        priya.contains("Separates what is known from what is guessed"),
        "{priya}"
    );
    assert!(priya.contains("(inference)"), "{priya}");
    let jamie = told("jamie");
    assert!(jamie.contains("Cut."), "{jamie}");
    assert!(jamie.contains("Cuts weak sentences"), "{jamie}");
    let alex = told("alex");
    assert!(alex.contains(CONDUCTOR_TAIL), "{alex}");
    assert!(!alex.contains("# ADD OR SKIP"), "{alex}");
    assert!(alex.contains("do not solve the task"), "{alex}");

    let custom: TeamMember = serde_json::from_str(
        r#"{"id":"grace","name":"Grace","icon":"🌻","role":"Garden expert","tagline":"Knows plants","instructions":"Speak softly about soil.","expertise":["soil"],"conductor":false,"active":true,"builtin":false}"#,
    )
    .expect("a roster saved before personalities");
    assert!(custom.personality.is_none());
    let note = member_instruction(&custom, std::slice::from_ref(&custom), false, "Ann");
    assert!(note.contains("Speak softly about soil."), "{note}");
    assert!(!note.contains(PASS_TOKEN), "{note}");

    let mut edited = builtin_member("morgan").expect("morgan");
    edited.tagline = "Changed".into();
    edited
        .personality
        .as_mut()
        .expect("shipped voice")
        .voice
        .tone = "Still Morgan".into();
    let decoded: TeamMember =
        serde_json::from_str(&serde_json::to_string(&edited).expect("save")).expect("load");
    assert_eq!(decoded.tagline, "Changed");
    assert_eq!(
        decoded.personality.expect("kept").voice.tone,
        "Still Morgan"
    );
    let restored = builtin_member("morgan").expect("reset");
    assert_ne!(restored.tagline, "Changed");
    assert!(
        restored
            .personality_line()
            .contains("Cuts consulting jargon")
    );

    let mut old = default_team();
    old.retain(|member| !matches!(member.id.as_str(), "scout" | "quinn" | "mara"));
    let alex = old
        .iter_mut()
        .find(|member| member.id == "alex")
        .expect("alex");
    alex.tagline = "My own lead".into();
    alex.personality = None;
    old.iter_mut()
        .find(|member| member.id == "morgan")
        .expect("morgan")
        .active = false;
    old.push(custom);
    let merged = merge_shipped(old);
    let alex = merged
        .iter()
        .find(|member| member.id == "alex")
        .expect("alex");
    assert_eq!(alex.tagline, "My own lead");
    assert!(alex.personality.is_none());
    assert!(
        !merged
            .iter()
            .find(|member| member.id == "morgan")
            .expect("morgan")
            .active
    );
    for id in ["scout", "quinn", "mara", "grace"] {
        assert!(merged.iter().any(|member| member.id == id), "{id}");
    }
    assert_eq!(merge_shipped(merged.clone()).len(), merged.len());
}

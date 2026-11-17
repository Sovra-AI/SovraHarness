use super::*;

fn person(id: &str) -> DecisionPerson {
    DecisionPerson {
        account_id: id.into(),
        name: id.into(),
    }
}

fn draft() -> DecisionDraft {
    DecisionDraft {
        question: "Which supplier should we choose?".into(),
        decision: "Choose Acme for the next quarter.".into(),
        criteria: "Delivery within two days; annual cost below our budget.".into(),
        evidence: vec![DecisionEvidence {
            reference: "Acme quote, page 2, read 2026-11-17".into(),
            passage: "Delivery within two days.".into(),
        }],
        review_on: "2027-02-01".into(),
        ends_when: "Delivery takes longer than two days.".into(),
        hard_stops: vec![DecisionStop {
            condition: "Contract must be reviewed.".into(),
            cleared: false,
        }],
        chat: Some(DecisionEvidence {
            reference: "chat-one / reply-two".into(),
            passage: "Consider Acme.".into(),
        }),
    }
}

fn record() -> DecisionRecord {
    DecisionRecord::create("one".into(), draft(), person("writer"), Timestamp(1)).unwrap()
}

#[test]
fn only_a_second_person_accepts_after_explicit_stops_are_cleared() {
    let record = record();
    assert_eq!(
        record.summary().unwrap().status(Timestamp(2)),
        DecisionStatus::Blocked
    );
    assert!(record.accept(1, person("writer"), Timestamp(2)).is_err());
    assert!(record.accept(1, person("reviewer"), Timestamp(2)).is_err());
    let mut cleared = draft();
    cleared.hard_stops[0].cleared = true;
    let edited = record
        .edit(1, cleared, person("editor"), Timestamp(2))
        .unwrap();
    for author in ["writer", "editor"] {
        assert!(edited.accept(2, person(author), Timestamp(3)).is_err());
    }
    let accepted = edited.accept(2, person("reviewer"), Timestamp(3)).unwrap();
    assert_eq!(accepted.revision, 3);
    assert_eq!(
        accepted.summary().unwrap().status(Timestamp(4)),
        DecisionStatus::Accepted
    );
    assert_eq!(
        accepted
            .current()
            .unwrap()
            .accepted
            .as_ref()
            .unwrap()
            .by
            .name,
        "reviewer"
    );
    assert!(accepted.accept(3, person("another"), Timestamp(4)).is_err());
}

#[test]
fn changes_keep_evidence_and_old_acceptance_but_require_fresh_acceptance() {
    let mut draft = draft();
    draft.hard_stops.clear();
    let original =
        DecisionRecord::create("one".into(), draft.clone(), person("writer"), Timestamp(1))
            .unwrap()
            .accept(1, person("reviewer"), Timestamp(2))
            .unwrap();
    draft.decision = "Choose Beta instead.".into();
    draft.evidence[0].passage = "New quote.".into();
    draft.chat = None;
    let edited = original
        .edit(2, draft, person("writer"), Timestamp(3))
        .unwrap();
    assert_eq!(edited.versions[0], original.versions[0]);
    assert_eq!(
        edited.current().unwrap().draft.chat,
        original.current().unwrap().draft.chat
    );
    assert!(edited.current().unwrap().accepted.is_none());
    assert_eq!(
        edited.summary().unwrap().status(Timestamp(4)),
        DecisionStatus::Draft
    );
    let restored: DecisionRecord =
        serde_json::from_str(&serde_json::to_string(&edited).unwrap()).unwrap();
    assert_eq!(restored, edited);
}

#[test]
fn stale_actions_and_ending_never_destroy_history() {
    let record = record();
    assert!(
        record
            .edit(0, draft(), person("writer"), Timestamp(2))
            .is_err()
    );
    assert!(record.accept(0, person("reviewer"), Timestamp(2)).is_err());
    assert!(
        record
            .end(0, "Late delivery".into(), person("writer"), Timestamp(2))
            .is_err()
    );
    assert!(
        record
            .end(1, " ".into(), person("writer"), Timestamp(2))
            .is_err()
    );
    let ended = record
        .end(
            1,
            "Delivery took a week.".into(),
            person("writer"),
            Timestamp(2),
        )
        .unwrap();
    assert_eq!(
        ended.summary().unwrap().status(Timestamp(3)),
        DecisionStatus::Ended
    );
    assert_eq!(
        ended.current().unwrap().draft,
        record.current().unwrap().draft
    );
    assert!(
        ended
            .end(2, "Again".into(), person("writer"), Timestamp(3))
            .is_err()
    );
    assert!(ended.accept(2, person("reviewer"), Timestamp(3)).is_err());
}

#[test]
fn review_due_is_a_status_boundary_and_blocks_new_acceptance() {
    let mut draft = draft();
    draft.hard_stops.clear();
    let due = review_date(&draft.review_on).unwrap();
    let record =
        DecisionRecord::create("one".into(), draft, person("writer"), Timestamp(1)).unwrap();
    assert!(record.accept(1, person("reviewer"), due).is_err());
    let accepted = record
        .accept(1, person("reviewer"), Timestamp(due.0 - 1))
        .unwrap();
    assert_eq!(
        accepted.summary().unwrap().status(Timestamp(due.0 - 1)),
        DecisionStatus::Accepted
    );
    assert_eq!(
        accepted.summary().unwrap().status(due),
        DecisionStatus::ReviewDue
    );
    assert_eq!(
        DecisionStatus::ReviewDue.label(),
        "Review due — needs a new decision"
    );
}

#[test]
fn dates_reject_bad_days_unicode_and_non_leap_centuries() {
    assert_eq!(decision_time(Timestamp(0)), "1970-01-01 00:00 UTC");
    assert_eq!(decision_time(Timestamp(-1)), "1969-12-31 23:59 UTC");
    assert_eq!(review_date("1970-01-01").unwrap(), Timestamp(0));
    assert_eq!(
        review_date("2000-01-01").unwrap(),
        Timestamp(946_684_800_000)
    );
    assert_eq!(
        review_date("2000-03-01").unwrap().0 - review_date("2000-02-28").unwrap().0,
        2 * 86_400_000
    );
    for date in [
        "",
        "2026-02-29",
        "2100-02-29",
        "2026-13-01",
        "2026-00-01",
        "2026-04-31",
        "2026-01-00",
        "1969-12-31",
        "ééééé",
        "2026-1-01",
    ] {
        assert!(review_date(date).is_err(), "{date}");
    }
}

#[test]
fn readable_copy_contains_the_evidence_authors_status_and_old_versions() {
    let first = record();
    let next = first
        .edit(1, draft(), person("editor"), Timestamp(2))
        .unwrap();
    let ended = next
        .end(
            2,
            "Delivery failed.".into(),
            person("reviewer"),
            Timestamp(3),
        )
        .unwrap();
    let text = ended.text(Timestamp(4));
    for expected in [
        "Version 1 (historical)",
        "Version 2 (current)",
        "Drafted by writer",
        "Drafted by editor",
        "Delivery within two days.",
        "Original chat: chat-one",
        "unresolved",
        "Ended by reviewer",
        "Delivery failed.",
        "Current status: No longer valid",
    ] {
        assert!(text.contains(expected), "{expected}");
    }
}

#[test]
fn bounded_required_fields_and_corrupt_empty_records_fail_closed() {
    let mut invalid = draft();
    invalid.evidence.clear();
    assert!(invalid.validate().is_err());
    invalid = draft();
    invalid.question = " ".into();
    assert!(invalid.validate().is_err());
    invalid = draft();
    invalid.criteria = "x".repeat(16_385);
    assert!(invalid.validate().is_err());
    invalid = draft();
    invalid.ends_when.clear();
    assert!(invalid.validate().is_err());
    let empty = DecisionRecord {
        id: "one".into(),
        revision: 1,
        versions: vec![],
    };
    assert!(empty.summary().is_err());
    assert!(empty.accept(1, person("reviewer"), Timestamp(2)).is_err());
    let mut record = record();
    record.versions = vec![record.versions[0].clone(); MAX_VERSIONS];
    assert!(
        record
            .edit(1, draft(), person("writer"), Timestamp(2))
            .is_err()
    );
}

#[test]
fn book_folds_only_runtime_results_and_keeps_open_record_on_failure() {
    let record = record();
    let mut book = DecisionBook::default();
    book.apply(&DecisionEvent::Listed {
        records: vec![record.summary().unwrap()],
        offset: 100,
        more: true,
    });
    book.apply(&DecisionEvent::Opened(Box::new(record.clone())));
    book.apply(&DecisionEvent::Error("Disk full".into()));
    assert_eq!(book.open.as_deref(), Some(&record));
    assert_eq!(book.error.as_deref(), Some("Disk full"));
    let ended = record
        .end(1, "Delivery failed.".into(), person("writer"), Timestamp(2))
        .unwrap();
    book.apply(&DecisionEvent::Saved(Box::new(ended)));
    assert!(book.error.is_none());
    assert!(book.records[0].ended);
    assert_eq!(book.offset, 100);
    assert!(book.more);
    let session = AuthSession {
        account_id: "writer".into(),
        full_name: "Writer".into(),
        email: "private@example.com".into(),
        provider: crate::auth::AuthProvider::Password,
        signed_in_at: Timestamp(1),
    };
    assert_eq!(
        DecisionPerson::from(&session),
        DecisionPerson {
            account_id: "writer".into(),
            name: "Writer".into()
        }
    );
}

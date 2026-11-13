use super::*;

const PHONE: DeviceId = DeviceId([1; 32]);
const OTHER: DeviceId = DeviceId([2; 32]);
const GENERATION: [u8; 32] = [3; 32];
const SECRET: InvitationSecret = InvitationSecret([4; 32]);

fn enabled() -> RemoteAccess {
    let mut access = RemoteAccess::default();
    access.enable(GENERATION);
    access
}

fn paired() -> RemoteAccess {
    let mut access = enabled();
    access.invite(&SECRET, 100).unwrap();
    access.pair(1, PHONE, "My phone", &SECRET, 101).unwrap();
    access
}

#[test]
fn identity_generation_and_activation_are_independent_gates() {
    let mut access = paired();
    assert_eq!(access.authorize(1, PHONE, &GENERATION), Ok(()));
    assert_eq!(
        access.authorize(1, OTHER, &GENERATION),
        Err(AccessError::NotPaired)
    );
    assert_eq!(
        access.authorize(2, PHONE, &GENERATION),
        Err(AccessError::UnsupportedVersion)
    );
    assert_eq!(
        access.authorize(1, PHONE, &[8; 32]),
        Err(AccessError::StaleGeneration)
    );
    access.disable();
    assert_eq!(
        access.authorize(1, PHONE, &GENERATION),
        Err(AccessError::Disabled)
    );
    access.enable([8; 32]);
    assert_eq!(
        access.authorize(1, PHONE, &GENERATION),
        Err(AccessError::StaleGeneration)
    );
    assert_eq!(access.authorize(1, PHONE, &[8; 32]), Ok(()));
    access.forget(PHONE);
    assert_eq!(
        access.authorize(1, PHONE, &[8; 32]),
        Err(AccessError::NotPaired)
    );
    assert_eq!(access.phones(), []);
}

#[test]
fn invitation_is_single_use_and_expiry_boundary_is_exclusive() {
    let mut access = enabled();
    let expires = access.invite(&SECRET, 100).unwrap();
    assert_eq!(expires, 100 + PAIRING_LIFETIME_MS);
    assert_eq!(
        access.pair(1, PHONE, "Phone", &SECRET, expires),
        Err(AccessError::InvalidInvitation)
    );
    assert_eq!(access.phones(), []);
    access.invite(&SECRET, expires).unwrap();
    access
        .pair(1, PHONE, "Phone", &SECRET, expires + 1)
        .unwrap();
    assert_eq!(
        access.pair(1, OTHER, "Other", &SECRET, expires + 2),
        Err(AccessError::InvalidInvitation)
    );
    assert_eq!(access.phones().len(), 1);
}

#[test]
fn replacement_and_disable_invalidate_invitations_but_repeated_enable_does_not() {
    let mut access = enabled();
    access.invite(&SECRET, 0).unwrap();
    let replacement = InvitationSecret([5; 32]);
    access.invite(&replacement, 1).unwrap();
    access.enable([9; 32]);
    assert_eq!(access.generation(), Some(GENERATION));
    assert_eq!(
        access.pair(1, PHONE, "Phone", &SECRET, 2),
        Err(AccessError::InvalidInvitation)
    );
    access.pair(1, PHONE, "Phone", &replacement, 2).unwrap();
    access.invite(&SECRET, 3).unwrap();
    access.disable();
    assert_eq!(access.invite(&SECRET, 4), Err(AccessError::Disabled));
    assert_eq!(
        access.pair(1, OTHER, "Other", &SECRET, 4),
        Err(AccessError::Disabled)
    );
    access.enable([9; 32]);
    assert_eq!(
        access.pair(1, OTHER, "Other", &SECRET, 5),
        Err(AccessError::InvalidInvitation)
    );
}

#[test]
fn failed_pairing_does_not_consume_invitation_or_change_allowlist() {
    let mut access = enabled();
    access.invite(&SECRET, 0).unwrap();
    assert_eq!(
        access.pair(2, PHONE, "Phone", &SECRET, 1),
        Err(AccessError::UnsupportedVersion)
    );
    assert_eq!(
        access.pair(1, PHONE, "Phone", &InvitationSecret([0; 32]), 1),
        Err(AccessError::InvalidInvitation)
    );
    for name in [" ".to_owned(), "bad\nname".into(), "é".repeat(41)] {
        assert_eq!(
            access.pair(1, PHONE, &name, &SECRET, 1),
            Err(AccessError::InvalidName)
        );
    }
    assert_eq!(access.phones(), []);
    access.pair(1, PHONE, &"é".repeat(40), &SECRET, 1).unwrap();
    access.invite(&SECRET, 2).unwrap();
    access.pair(1, OTHER, "Other", &SECRET, 3).unwrap();
}

#[test]
fn paired_phone_rescanning_a_valid_code_consumes_it_without_a_duplicate() {
    let mut access = enabled();
    access.invite(&SECRET, 0).unwrap();
    access.pair(1, PHONE, "Phone", &SECRET, 1).unwrap();
    access.invite(&SECRET, 2).unwrap();
    // A phone that forgot this computer locally scans a fresh code: the code closes,
    // so control is not paused until it expires, and the allowlist is unchanged.
    access.pair(1, PHONE, "Renamed", &SECRET, 3).unwrap();
    assert_eq!(access.phones().len(), 1);
    assert_eq!(access.phones()[0].name, "Phone");
    assert_eq!(
        access.pair(1, OTHER, "Other", &SECRET, 4),
        Err(AccessError::InvalidInvitation)
    );
}

#[test]
fn restored_phones_start_disabled_and_corrupt_records_fail_closed() {
    let original = paired();
    let json = serde_json::to_string(original.phones()).unwrap();
    let restored = RemoteAccess::restore(serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(restored.generation(), None);
    assert_eq!(restored.phones(), original.phones());
    assert_eq!(
        restored.authorize(1, PHONE, &GENERATION),
        Err(AccessError::Disabled)
    );
    let phone = original.phones()[0].clone();
    assert_eq!(
        RemoteAccess::restore(vec![phone.clone(), phone]).unwrap_err(),
        AccessError::DuplicatePhone
    );
    assert_eq!(
        RemoteAccess::restore(vec![PairedPhone {
            device: PHONE,
            name: String::new()
        }])
        .unwrap_err(),
        AccessError::InvalidName
    );
    assert!(serde_json::from_str::<PairedPhone>(r#"{"device":"bad","name":"Phone"}"#).is_err());
}

#[test]
fn capacity_overflow_and_diagnostics_are_bounded() {
    let phones: Vec<_> = (0..8)
        .map(|id| PairedPhone {
            device: DeviceId([id; 32]),
            name: "Phone".into(),
        })
        .collect();
    let mut access = RemoteAccess::restore(phones.clone()).unwrap();
    access.enable(GENERATION);
    assert_eq!(access.invite(&SECRET, 0), Err(AccessError::PhoneLimit));
    let mut too_many = phones;
    too_many.push(PairedPhone {
        device: DeviceId([9; 32]),
        name: "Phone".into(),
    });
    assert_eq!(
        RemoteAccess::restore(too_many).unwrap_err(),
        AccessError::PhoneLimit
    );
    access.forget(PHONE);
    assert_eq!(
        access.invite(&SECRET, u64::MAX),
        Err(AccessError::InvalidInvitation)
    );
    assert_eq!(format!("{SECRET:?}"), "InvitationSecret([redacted])");
    assert!(!format!("{access:?}").contains("digest"));
}

#[test]
fn uncommitted_candidate_does_not_grant_access() {
    let mut live = enabled();
    live.invite(&SECRET, 0).unwrap();
    let mut candidate = live.clone();
    candidate.pair(1, PHONE, "Phone", &SECRET, 1).unwrap();
    assert_eq!(
        live.authorize(1, PHONE, &GENERATION),
        Err(AccessError::NotPaired)
    );
    assert_eq!(candidate.authorize(1, PHONE, &GENERATION), Ok(()));
    // A storage failure discards the candidate; retrying remains possible.
    live.pair(1, OTHER, "Other", &SECRET, 2).unwrap();
    assert_eq!(
        live.authorize(1, PHONE, &GENERATION),
        Err(AccessError::NotPaired)
    );
}

use super::*;

fn syncer() -> GridSyncer {
    GridSyncer::new(
        ContentHash::of(b"test run"),
        vec![0.0, 0.0],
        GridSettings::default(),
    )
    .unwrap()
}

fn update(grid: &mut GridSyncer, worker: u8, gradient: [f32; 2]) -> Contribution {
    Contribution {
        lease: grid.issue(DeviceId([worker; 32]), 0).unwrap(),
        tokens: 10,
        pseudo_gradient: gradient.to_vec(),
    }
}

#[test]
fn one_survivor_advances_while_another_worker_disappears() {
    let mut grid = syncer();
    let lost = update(&mut grid, 1, [1.0, 0.0]);
    let ready = update(&mut grid, 2, [1.0, -1.0]);
    assert!(grid.has_lease(&lost.lease));
    assert!(grid.has_lease(&ready.lease));
    let ready_lease = ready.lease.clone();
    grid.merge(&[ready], 1).unwrap();
    assert!(!grid.has_lease(&ready_lease));
    assert_eq!(grid.round(), 1);
    assert_eq!(grid.tokens(), 10);
    assert!(grid.params()[0] < 0.0 && grid.params()[1] > 0.0);
    assert_eq!(grid.issue(lost.lease.worker, 2), Err(GridError::Busy));
    let replacement = grid.issue(lost.lease.worker, 60_000).unwrap();
    assert!(!grid.has_lease(&lost.lease));
    assert_ne!(replacement.id, lost.lease.id);
    assert_eq!(replacement.base_round, 1);
    let before = grid.clone();
    assert_eq!(grid.merge(&[lost], 60_000), Err(GridError::InvalidLease));
    assert_eq!(grid, before);
}

#[test]
fn replay_and_changed_base_are_rejected_without_changing_any_state() {
    let mut grid = syncer();
    let good = update(&mut grid, 1, [0.2, -0.3]);
    let mut wrong = good.clone();
    wrong.lease.base_model = ContentHash::of(b"wrong model");
    let before = grid.clone();
    assert_eq!(grid.merge(&[wrong], 1), Err(GridError::InvalidLease));
    assert_eq!(grid, before);
    grid.merge(std::slice::from_ref(&good), 1).unwrap();
    let before = grid.clone();
    assert_eq!(grid.merge(&[good], 2), Err(GridError::InvalidLease));
    assert_eq!(grid, before);
}

#[test]
fn invalid_batch_does_not_consume_valid_peers_work() {
    let mut grid = syncer();
    let good = update(&mut grid, 1, [0.1, 0.2]);
    let original = update(&mut grid, 2, [0.2, 0.3]);
    for values in [
        vec![f32::NAN, 1.0],
        vec![f32::INFINITY, 1.0],
        vec![1.0],
        vec![],
    ] {
        let mut bad = original.clone();
        bad.pseudo_gradient = values;
        let before = grid.clone();
        assert_eq!(
            grid.merge(&[good.clone(), bad], 1),
            Err(GridError::InvalidUpdate)
        );
        assert_eq!(grid, before);
    }
    for tokens in [0, original.lease.token_budget + 1] {
        let mut bad = original.clone();
        bad.tokens = tokens;
        assert_eq!(grid.merge(&[bad], 1), Err(GridError::InvalidUpdate));
    }
    assert_eq!(
        grid.merge(&[good.clone(), good.clone()], 1),
        Err(GridError::InvalidLease)
    );
    grid.merge(&[good, original], 1).unwrap();
    assert_eq!(grid.tokens(), 20);
}

#[test]
fn token_weighting_and_age_gating_happen_before_both_moments() {
    let mut grid = syncer();
    let old = update(&mut grid, 1, [0.4, 0.0]);
    let first = update(&mut grid, 2, [0.0, 0.2]);
    grid.merge(&[first], 1).unwrap();
    let mut fresh = update(&mut grid, 3, [-0.2, 0.0]);
    fresh.tokens = 30;
    let expected = 0.4_f64 * grid.saved.settings.gate(1) * 0.25 - 0.2 * 0.75;
    grid.merge(&[old, fresh], 2).unwrap();
    assert!((grid.saved.first[0] - 0.1 * expected).abs() < 1e-8);
    assert!((grid.saved.second[0] - 0.001 * expected * expected).abs() < 1e-8);
    assert_eq!(grid.tokens(), 50);
}

#[test]
fn cutoff_drops_old_work_without_touching_moments() {
    let mut grid = syncer();
    let old = update(&mut grid, 1, [f32::MAX, f32::MAX]);
    for _ in 0..grid.saved.settings.cutoff_rounds {
        let fresh = update(&mut grid, 2, [0.1, 0.2]);
        grid.merge(&[fresh], 1).unwrap();
    }
    let before = grid.clone();
    assert_eq!(grid.merge(&[old], 1), Err(GridError::Stale));
    assert_eq!(grid, before);
    assert!(grid.issue(DeviceId([1; 32]), 1).is_ok());
    let settings = GridSettings::default();
    assert_eq!(settings.gate(0), 1.0);
    assert!(settings.gate(1) < 1.0 && settings.gate(1) > 0.0);
    assert_eq!(settings.gate(u64::MAX), 0.0);
}

#[test]
fn checkpoint_roundtrip_preserves_the_next_optimizer_step_and_invalidates_leases() {
    let mut uninterrupted = syncer();
    for _ in 0..7 {
        let first = update(&mut uninterrupted, 1, [0.2, -0.4]);
        uninterrupted.merge(&[first], 1).unwrap();
    }
    let lost = update(&mut uninterrupted, 2, [0.1, 0.2]);
    let bytes = serde_json::to_vec(&uninterrupted.checkpoint()).unwrap();
    let mut restored = GridSyncer::restore(serde_json::from_slice(&bytes).unwrap()).unwrap();
    assert_eq!(restored.merge(&[lost], 1), Err(GridError::InvalidLease));
    let a = update(&mut uninterrupted, 3, [0.3, -0.1]);
    let b = update(&mut restored, 3, [0.3, -0.1]);
    uninterrupted.merge(&[a], 2).unwrap();
    restored.merge(&[b], 2).unwrap();
    assert_eq!(restored.checkpoint(), uninterrupted.checkpoint());
}

#[test]
fn checkpoint_rejects_corruption_and_unsupported_versions() {
    let grid = syncer();
    let original = grid.checkpoint();
    for field in 0..8 {
        let mut saved = original.clone();
        match field {
            0 => saved.version = 0,
            1 => saved.params[0] = f32::NAN,
            2 => saved.first.clear(),
            3 => saved.second[0] = -1.0,
            4 => saved.beta1_power = 0.0,
            5 => saved.next_lease = 0,
            6 => saved.settings.beta2 = 1.0,
            7 => saved.first[0] = f64::INFINITY,
            _ => unreachable!(),
        }
        assert_eq!(
            GridSyncer::restore(saved),
            Err(GridError::InvalidCheckpoint)
        );
    }
}

#[test]
fn settings_bounds_capacity_and_overflow_are_checked() {
    assert!(GridSyncer::new(ContentHash::of(b"empty"), vec![], GridSettings::default()).is_err());
    for rate in [0.0, -0.1, f64::NAN, f64::INFINITY] {
        let settings = GridSettings {
            learning_rate: rate,
            ..GridSettings::default()
        };
        assert!(GridSyncer::new(ContentHash::of(b"bad"), vec![0.0], settings).is_err());
    }
    let mut grid = syncer();
    for i in 0..64u8 {
        grid.issue(DeviceId([i; 32]), 0).unwrap();
    }
    assert_eq!(grid.issue(DeviceId([65; 32]), 0), Err(GridError::Busy));
    assert_eq!(
        grid.issue(DeviceId([65; 32]), u64::MAX),
        Err(GridError::Exhausted)
    );
    assert!(grid.issue(DeviceId([65; 32]), 60_000).is_ok());
}

#[test]
fn huge_finite_updates_are_clipped_before_squaring_into_moments() {
    let mut grid = syncer();
    let huge = update(&mut grid, 1, [f32::MAX, -f32::MAX]);
    grid.merge(&[huge], 1).unwrap();
    assert!(grid.params().iter().all(|x| x.is_finite()));
    assert!(
        grid.saved
            .second
            .iter()
            .all(|x| x.is_finite() && *x <= 0.001)
    );
}

#[test]
fn the_outer_step_size_can_follow_a_schedule_but_only_inside_its_range() {
    let mut grid = syncer();
    let before = grid.clone();
    for bad in [0.0, -0.1, 1.5, f64::NAN, f64::INFINITY] {
        assert_eq!(
            grid.set_learning_rate(bad),
            Err(GridError::InvalidSettings),
            "{bad}"
        );
        assert_eq!(grid, before);
    }
    grid.set_learning_rate(0.5).unwrap();
    assert!((grid.settings().learning_rate - 0.5).abs() < f64::EPSILON);
    let big = update(&mut grid, 1, [1.0, 0.0]);
    grid.merge(&[big], 1).unwrap();
    let mut small = syncer();
    small.set_learning_rate(0.001).unwrap();
    let step = update(&mut small, 1, [1.0, 0.0]);
    small.merge(&[step], 1).unwrap();
    assert!(grid.params()[0].abs() > small.params()[0].abs() * 100.0);
    // The saved state carries the changed rate.
    let restored = GridSyncer::restore(grid.checkpoint()).unwrap();
    assert!((restored.settings().learning_rate - 0.5).abs() < f64::EPSILON);
}

#[test]
fn the_compact_saved_form_round_trips_and_rejects_every_wrong_length() {
    let mut grid = syncer();
    let step = update(&mut grid, 1, [0.25, -0.5]);
    grid.merge(&[step], 1).unwrap();
    let saved = grid.checkpoint();
    let bytes = saved.to_bytes();
    assert_eq!(bytes.len(), GridCheckpoint::byte_len(2));
    assert_eq!(saved.parameters(), 2);
    let decoded = GridCheckpoint::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, saved);
    assert_eq!(
        GridSyncer::restore(decoded).unwrap().params(),
        grid.params()
    );

    for cut in 0..bytes.len() {
        assert_eq!(
            GridCheckpoint::from_bytes(&bytes[..cut]),
            Err(GridError::InvalidCheckpoint),
            "cut at {cut}"
        );
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(
        GridCheckpoint::from_bytes(&longer),
        Err(GridError::InvalidCheckpoint)
    );
    let mut magic = bytes.clone();
    magic[0] ^= 1;
    assert_eq!(
        GridCheckpoint::from_bytes(&magic),
        Err(GridError::InvalidCheckpoint)
    );
    // A huge declared size is refused from the header alone.
    let count_at = bytes.len() - 2 * 20 - 8;
    let mut huge = bytes.clone();
    huge[count_at..count_at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(
        GridCheckpoint::from_bytes(&huge),
        Err(GridError::InvalidCheckpoint)
    );
    // Values the grid would never produce decode, then fail the restore check.
    let mut version = bytes.clone();
    version[8] = 9;
    let decoded = GridCheckpoint::from_bytes(&version).unwrap();
    assert_eq!(
        GridSyncer::restore(decoded),
        Err(GridError::InvalidCheckpoint)
    );
    let mut value = bytes;
    let at = value.len() - 2 * 20;
    value[at..at + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    let decoded = GridCheckpoint::from_bytes(&value).unwrap();
    assert!(GridSyncer::restore(decoded).is_err());
}

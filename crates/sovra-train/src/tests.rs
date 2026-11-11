//! Burn gradients are right, and three departments with conflicting data train one
//! shared model with SCAFFOLD without any example leaving its computer.

use sovra_core::swarm::learn::scaffold::{
    Algorithm, LocalWork, Objective, ScaffoldClient, ScaffoldServer, mean,
};

use super::*;

/// A department whose data follows `y = slope·x₀ + offset·x₁ + bias` on its own
/// slice of inputs (non-IID: each sees a different region).
fn department(slope: f32, offset: f32, bias: f32, start: f32) -> LinearModel {
    let rows: Vec<Vec<f32>> = (0..12)
        .map(|i| {
            let i = f32::from(u8::try_from(i).expect("small"));
            vec![start + i * 0.25, (start + i * 0.5).sin()]
        })
        .collect();
    let targets = rows
        .iter()
        .map(|r| slope * r[0] + offset * r[1] + bias)
        .collect();
    LinearModel::new(Examples::new(rows, targets).expect("well formed"))
}

#[test]
fn burn_autodiff_gives_the_analytic_gradient() {
    let model = department(2.0, -1.0, 0.5, 0.0);
    let params = [0.3, 0.1, -0.2];
    let grad = model.gradient(&params);
    // Central differences on the same loss.
    let h = 1e-3;
    for i in 0..3 {
        let mut up = params;
        let mut down = params;
        up[i] += h;
        down[i] -= h;
        let numeric = (model.loss(&up) - model.loss(&down)) / (2.0 * h);
        assert!(
            (grad[i] - numeric).abs() < 2e-2,
            "{i}: {} vs {numeric}",
            grad[i]
        );
    }
    assert_eq!(model.dimension(), 3);
}

#[test]
fn ragged_or_empty_examples_are_refused() {
    assert_eq!(
        Examples::new(Vec::new(), Vec::new()),
        Err(TrainError::NoExamples)
    );
    assert_eq!(
        Examples::new(vec![vec![1.0], vec![1.0, 2.0]], vec![0.0, 1.0]),
        Err(TrainError::RaggedExamples)
    );
}

#[test]
fn departments_train_one_model_together_with_scaffold_on_burn() {
    // The same rule everywhere, seen through very different slices of data.
    let parts = [
        department(2.0, -1.0, 0.5, 0.0),
        department(2.0, -1.0, 0.5, 3.0),
        department(2.0, -1.0, 0.5, 6.0),
    ];
    let work = LocalWork {
        steps: 10,
        learning_rate: 0.01,
        algorithm: Algorithm::Scaffold,
    };
    let mut server = ScaffoldServer::new(vec![0.0; 3], parts.len(), work);
    let mut clients: Vec<ScaffoldClient> = parts.iter().map(|_| ScaffoldClient::new(3)).collect();
    let total = |params: &[f32]| parts.iter().map(|p| p.loss(params)).sum::<f32>();
    let start = total(&server.params);
    for _ in 0..60 {
        let deltas: Vec<Vec<f32>> = clients
            .iter_mut()
            .zip(&parts)
            .map(|(client, part)| client.train(part, &server.params, &server.control, work))
            .collect();
        server.apply_mean(&mean(&deltas), deltas.len());
    }
    let end = total(&server.params);
    assert!(end < start * 0.05, "loss {start} -> {end}");
    assert!(
        (server.params[0] - 2.0).abs() < 0.2,
        "slope {}",
        server.params[0]
    );
}

/// Deterministic randomness for the test only; the app uses the OS source.
struct Seeded(u64);

impl sovra_core::swarm::learn::RandomSource for Seeded {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn pair_seed(lo: u8, hi: u8, round: u64) -> [u8; 32] {
    let mut bytes = vec![lo, hi];
    bytes.extend_from_slice(&round.to_le_bytes());
    sovra_core::group::ContentHash::of(&bytes).0
}

#[test]
fn a_private_round_on_burn_models_lowers_everyones_loss_within_budget() {
    use std::collections::{BTreeMap, BTreeSet};

    use sovra_core::Timestamp;
    use sovra_core::group::{ContentHash, DeviceId, PersonId};
    use sovra_core::swarm::department::{Department, DepartmentId, Directory, TrainingConsent};
    use sovra_core::swarm::egress::EgressGate;
    use sovra_core::swarm::learn::privacy::{PrivacyBudget, Quantiser};
    use sovra_core::swarm::learn::round::{
        RoundPlan, RoundSettings, aggregate, contribute, model_hash, upload_bytes,
    };
    use sovra_core::swarm::learn::secagg::MaskingClient;

    // Ten computers in one consenting department, each with its own slice of data.
    let finance = DepartmentId::new("finance").expect("valid");
    let approver = PersonId([99; 32]);
    let mut directory = Directory::default();
    let mut finance_dept = Department::new(finance.clone(), "Finance");
    finance_dept.training = TrainingConsent::ApprovedExamples;
    finance_dept.members.insert(approver);
    finance_dept.approvers.insert(approver);
    directory.add(finance_dept);
    let computers: Vec<u8> = (1..=10).collect();
    let parts: Vec<LinearModel> = computers
        .iter()
        .map(|n| department(2.0, -1.0, 0.5, f32::from(*n) * 0.7))
        .collect();
    let roster: BTreeSet<DeviceId> = computers.iter().map(|n| DeviceId([*n; 32])).collect();
    let work = LocalWork {
        steps: 10,
        learning_rate: 0.01,
        algorithm: Algorithm::Scaffold,
    };
    let mut server = ScaffoldServer::new(vec![0.0; 3], parts.len(), work);
    let mut clients: Vec<ScaffoldClient> = parts.iter().map(|_| ScaffoldClient::new(3)).collect();
    let mut budget = PrivacyBudget::for_epsilon(50.0, 1e-5);
    let mut rng = Seeded(7);
    let total = |params: &[f32]| parts.iter().map(|p| p.loss(params)).sum::<f32>();
    let start = total(&server.params);
    let rounds = 20u32;
    for round in 1..=u64::from(rounds) {
        let plan = RoundPlan::new(
            RoundSettings {
                round,
                base_model: model_hash(&server.params),
                departments: [finance.clone()].into(),
                roster: roster.clone(),
                dimension: 3,
                quantiser: Quantiser {
                    clip: 2.0,
                    scale: 4096.0,
                },
                assumed_colluders: 0,
                rounds_left: rounds - u32::try_from(round).expect("small") + 1,
            },
            &budget,
            &directory,
        )
        .expect("finance consented");
        let mut uploads = BTreeMap::new();
        let mut maskers = Vec::new();
        for (index, n) in computers.iter().enumerate() {
            let seeds = computers
                .iter()
                .filter(|m| *m != n)
                .map(|m| (DeviceId([*m; 32]), pair_seed(*n.min(m), *n.max(m), round)))
                .collect();
            let masker = MaskingClient::new(
                DeviceId([*n; 32]),
                round,
                roster.clone(),
                plan.threshold,
                [n.wrapping_add(u8::try_from(round % 200).expect("small")); 32],
                seeds,
            )
            .expect("everyone agreed a seed");
            let delta = clients[index].train(&parts[index], &server.params, &server.control, work);
            let contribution = contribute(
                &plan,
                &directory,
                &finance,
                &delta,
                &masker,
                &mut rng,
                Timestamp(1),
            )
            .expect("consented");
            // The department's approver signs off on these exact bytes before they leave.
            let mut gate = EgressGate::default();
            let id = gate.hold(contribution.draft);
            gate.approve(id, approver, Timestamp(2), &directory)
                .expect("the approver signs off");
            gate.release(id, &ContentHash::of(&upload_bytes(&contribution.masked)))
                .expect("exact bytes");
            uploads.insert(DeviceId([*n; 32]), contribution.masked);
            maskers.push(masker);
        }
        let survivors: BTreeSet<DeviceId> = uploads.keys().copied().collect();
        let reveals: Vec<_> = maskers
            .iter_mut()
            .map(|m| m.reveal(&survivors).expect("one list"))
            .collect();
        let outcome = aggregate(&plan, &uploads, &reveals, &mut budget).expect("within budget");
        server.apply_mean(&outcome.mean, outcome.participants);
    }
    let end = total(&server.params);
    assert!(end < start * 0.5, "loss {start} -> {end}");
    assert!(budget.left() >= 0.0);
}

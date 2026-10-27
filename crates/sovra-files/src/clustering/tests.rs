use super::*;
use sovra_core::file_index::{semantic::EmbeddingVector, summary::SummaryInputRef};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Observer {
    remaining: AtomicUsize,
}
impl Observer {
    fn new(remaining: usize) -> Self {
        Self {
            remaining: AtomicUsize::new(remaining),
        }
    }
}
impl ClusterObserver for Observer {
    fn cancelled(&self) -> bool {
        self.remaining
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_err()
    }
}

fn input(index: usize, embedding: Vec<f32>, text_bytes: usize) -> ClusterInput {
    ClusterInput {
        reference: SummaryInputRef::Summary(u64::try_from(index + 1).unwrap()),
        embedding: EmbeddingVector::new(embedding).unwrap(),
        text_bytes,
    }
}

#[test]
fn global_local_umap_and_mixtures_cover_every_source_within_request_budgets() {
    let inputs: Vec<_> = (0..36_u16)
        .map(|index| {
            let mut values = vec![0.; 6];
            values[usize::from(index % 3)] = 1.;
            values[3] = f32::from(index % 7) * 0.03;
            values[4] = f32::from(index % 11) * 0.02;
            values[5] = f32::from(index % 5) * 0.01;
            input(usize::from(index), values, 250)
        })
        .collect();
    let options = RaptorOptions {
        dimensions: 2,
        max_components: 4,
        umap_epochs: 40,
        max_group_inputs: 8,
        max_group_bytes: 2048,
        ..RaptorOptions::default()
    };
    let clusterer = RaptorClusterer::new().unwrap();
    assert_eq!(clusterer.pool.current_num_threads(), 1);
    let plan = clusterer
        .cluster(&inputs, &options, &Observer::new(10_000_000))
        .unwrap();
    assert_eq!(plan.algorithm_revision, RAPTOR_CLUSTERING_REVISION);
    assert!(plan.diagnostics.reductions >= 2);
    assert!(plan.diagnostics.mixture_fits >= plan.diagnostics.reductions);
    let covered: BTreeSet<_> = plan.groups.iter().flatten().copied().collect();
    assert_eq!(covered, (0..inputs.len()).collect());
    assert!(plan.groups.iter().all(|g| !g.is_empty()
        && g.len() <= 8
        && g.iter().map(|i| inputs[*i].text_bytes).sum::<usize>() <= 2048));
    assert!(
        clusterer
            .cluster(&inputs, &options, &Observer::new(55))
            .unwrap_err()
            .0
            .contains("cancelled")
    );
}

#[test]
fn identical_embeddings_are_partitioned_explicitly_without_inventing_clusters() {
    let inputs: Vec<_> = (0..24).map(|i| input(i, vec![1., 0.], 500)).collect();
    let options = RaptorOptions {
        max_group_bytes: 2048,
        ..RaptorOptions::default()
    };
    let plan = RaptorClusterer::new()
        .unwrap()
        .cluster(&inputs, &options, &Observer::new(10_000))
        .unwrap();
    assert_eq!(plan.diagnostics.reductions, 0);
    assert!(plan.diagnostics.budget_splits > 0);
    assert!(plan.groups.iter().all(|g| g.len() <= 4));
    assert_eq!(
        plan.groups
            .iter()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>(),
        (0..24).collect()
    );
}

#[test]
fn resource_bisection_groups_by_embedding_distance_instead_of_input_adjacency() {
    let inputs = vec![
        input(0, vec![1., 0.], 100),
        input(1, vec![0., 1.], 100),
        input(2, vec![1., 0.], 100),
        input(3, vec![0., 1.], 100),
    ];
    let (left, right) = bisect(&inputs, &[0, 1, 2, 3]);
    let mut groups = vec![left, right];
    groups.sort();
    assert_eq!(groups, vec![vec![0, 2], vec![1, 3]]);
}

#[test]
fn tiny_batches_and_cancellation_preserve_input_and_do_not_start_numerical_work() {
    let inputs = vec![input(0, vec![1., 0.], 100)];
    let clusterer = RaptorClusterer::new().unwrap();
    let plan = clusterer
        .cluster(&inputs, &RaptorOptions::default(), &Observer::new(100))
        .unwrap();
    assert_eq!(plan.groups, vec![vec![0]]);
    assert_eq!(plan.diagnostics, ClusterDiagnostics::default());
    assert!(
        clusterer
            .cluster(&inputs, &RaptorOptions::default(), &Observer::new(0))
            .is_err()
    );
    assert!(
        clusterer
            .cluster(
                &[input(0, vec![1., 0.], 100_000)],
                &RaptorOptions::default(),
                &Observer::new(100)
            )
            .is_err()
    );
}

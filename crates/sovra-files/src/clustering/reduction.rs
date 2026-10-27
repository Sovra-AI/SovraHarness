use super::{Budget, ClusterInput, IndexError, RaptorOptions};
use ndarray::Array2;
use umap_rs::{
    EuclideanMetric, GraphParams, MetricType, OptimizationParams, Optimizer, Umap, UmapConfig,
};

pub(super) fn reduce(
    inputs: &[ClusterInput],
    ids: &[usize],
    options: &RaptorOptions,
    global: bool,
    budget: &Budget<'_>,
) -> Result<Vec<Vec<f64>>, IndexError> {
    budget.check()?;
    let width = inputs[ids[0]].embedding.values().len();
    let dimensions = usize::from(options.dimensions)
        .min(ids.len() - 2)
        .min(width);
    let global_neighbors = (2..=ids.len())
        .take_while(|n| n * n < ids.len())
        .last()
        .unwrap_or(2);
    let neighbors = if global { global_neighbors } else { 10 }.clamp(2, ids.len() - 1);
    let data = Array2::from_shape_vec(
        (ids.len(), width),
        ids.iter()
            .flat_map(|id| inputs[*id].embedding.values().iter().copied())
            .collect(),
    )
    .map_err(|_| IndexError("The clustering embedding matrix has an invalid shape.".into()))?;
    let (indices, distances) = neighbors_for(inputs, ids, neighbors, budget)?;
    let config = UmapConfig {
        n_components: dimensions,
        graph: GraphParams {
            n_neighbors: neighbors,
            ..GraphParams::default()
        },
        optimization: OptimizationParams {
            n_epochs: Some(usize::from(options.umap_epochs)),
            ..OptimizationParams::default()
        },
        ..UmapConfig::default()
    };
    // learn_manifold consumes our cosine KNN graph; optimization uses Euclidean
    // geometry in the reduced space. It does not recompute input distances.
    let manifold =
        Umap::new(config.clone()).learn_manifold(data.view(), indices.view(), distances.view());
    budget.check()?;
    let init = Array2::from_shape_fn((ids.len(), dimensions), |(row, column)| {
        let key = u32::try_from(row + 1)
            .expect("bounded row")
            .wrapping_mul(2_654_435_761)
            ^ u32::try_from(column + 1)
                .expect("bounded dimensions")
                .wrapping_mul(2_246_822_519);
        let mixed = key ^ (key >> 16);
        f32::from(u16::try_from(mixed & 0xffff).expect("masked initializer")) / 65535. * 20. - 10.
    });
    let mut optimizer = Optimizer::new(
        manifold,
        init,
        usize::from(options.umap_epochs),
        &config,
        MetricType::Euclidean,
    );
    while optimizer.remaining_epochs() > 0 {
        budget.check()?;
        optimizer.step_epochs(optimizer.remaining_epochs().min(5), &EuclideanMetric);
    }
    let reduced: Vec<Vec<_>> = optimizer
        .embedding()
        .outer_iter()
        .map(|row| row.iter().map(|v| f64::from(*v)).collect())
        .collect();
    if reduced.iter().flatten().any(|v| !v.is_finite()) {
        return Err(IndexError(
            "UMAP produced a non-finite reduced embedding. No clustering result was saved.".into(),
        ));
    }
    Ok(reduced)
}

fn neighbors_for(
    inputs: &[ClusterInput],
    ids: &[usize],
    neighbors: usize,
    budget: &Budget<'_>,
) -> Result<(Array2<u32>, Array2<f32>), IndexError> {
    let mut indices = Array2::zeros((ids.len(), neighbors));
    let mut distances = Array2::zeros((ids.len(), neighbors));
    for (row, id) in ids.iter().enumerate() {
        budget.check()?;
        let mut ranked: Vec<_> = ids
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != row)
            .map(|(column, other)| {
                let dot = inputs[*id]
                    .embedding
                    .values()
                    .iter()
                    .zip(inputs[*other].embedding.values())
                    .map(|(a, b)| f64::from(*a) * f64::from(*b))
                    .sum::<f64>();
                (column, (1. - dot).clamp(0., 2.))
            })
            .collect();
        ranked.sort_by(|(ia, a), (ib, b)| a.total_cmp(b).then_with(|| ia.cmp(ib)));
        // UMAP expects self at column zero, including for duplicate vectors.
        indices[(row, 0)] = u32::try_from(row).expect("bounded neighbor index");
        for (position, (index, distance)) in ranked.into_iter().take(neighbors - 1).enumerate() {
            indices[(row, position + 1)] = u32::try_from(index).expect("bounded neighbor index");
            distances[(row, position + 1)] = bounded_distance(distance);
        }
    }
    Ok((indices, distances))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "cosine distances are clamped to [0,2] before f32 UMAP input"
)]
fn bounded_distance(value: f64) -> f32 {
    value as f32
}

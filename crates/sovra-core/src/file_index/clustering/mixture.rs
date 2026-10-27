//! Full-covariance EM with stable likelihoods and explicit BIC model selection.

use super::{
    ClusterObserver, IndexError, MAX_CLUSTER_INPUTS, RaptorOptions, gaussian_bic, soft_memberships,
};

const REGULARIZATION: f64 = 1e-6;
const TOLERANCE: f64 = 1e-3;

#[derive(Debug, Clone, PartialEq)]
pub struct MixtureSelection {
    pub groups: Vec<Vec<usize>>,
    pub components: usize,
    pub bic: f64,
    pub attempted: u32,
    pub failed: u32,
}

#[derive(Debug)]
enum FitFailure {
    Cancelled,
    Numerical,
    NotConverged,
}

fn check(observer: &dyn ClusterObserver) -> Result<(), FitFailure> {
    if observer.cancelled() {
        Err(FitFailure::Cancelled)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct Gaussian {
    weight: f64,
    mean: Vec<f64>,
    lower: Vec<Vec<f64>>,
    log_determinant: f64,
}

impl Gaussian {
    fn new(weight: f64, mean: Vec<f64>, covariance: &[Vec<f64>]) -> Result<Self, FitFailure> {
        let dimensions = mean.len();
        let mut lower = vec![vec![0.; dimensions]; dimensions];
        for row in 0..dimensions {
            for column in 0..=row {
                let residual = covariance[row][column]
                    - (0..column)
                        .map(|i| lower[row][i] * lower[column][i])
                        .sum::<f64>();
                if row == column {
                    if !residual.is_finite() || residual <= 0. {
                        return Err(FitFailure::Numerical);
                    }
                    lower[row][column] = residual.sqrt();
                } else {
                    lower[row][column] = residual / lower[column][column];
                }
            }
        }
        let log_determinant = 2. * (0..dimensions).map(|i| lower[i][i].ln()).sum::<f64>();
        Ok(Self {
            weight,
            mean,
            lower,
            log_determinant,
        })
    }

    fn log_probability(&self, point: &[f64]) -> f64 {
        let mut solved = vec![0.; point.len()];
        for (i, value) in point.iter().enumerate() {
            solved[i] = (value
                - self.mean[i]
                - self.lower[i][..i]
                    .iter()
                    .zip(&solved[..i])
                    .map(|(a, b)| a * b)
                    .sum::<f64>())
                / self.lower[i][i];
        }
        self.weight.ln()
            - 0.5
                * (count(point.len()) * (2. * std::f64::consts::PI).ln()
                    + self.log_determinant
                    + solved.iter().map(|v| v * v).sum::<f64>())
    }
}

fn count(value: usize) -> f64 {
    f64::from(u32::try_from(value).expect("validated numerical dimensions"))
}

fn distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum()
}

fn covariance(points: &[Vec<f64>], weights: &[f64], mean: &[f64], mass: f64) -> Vec<Vec<f64>> {
    let dimensions = mean.len();
    let mut result = vec![vec![0.; dimensions]; dimensions];
    for (row, values) in result.iter_mut().enumerate() {
        for (column, value) in values.iter_mut().enumerate() {
            *value = points
                .iter()
                .zip(weights)
                .map(|(p, w)| w * (p[row] - mean[row]) * (p[column] - mean[column]))
                .sum::<f64>()
                / mass;
            if row == column {
                *value += REGULARIZATION;
            }
        }
    }
    result
}

fn initialize(points: &[Vec<f64>], components: usize) -> Result<Vec<Gaussian>, FitFailure> {
    let dimensions = points[0].len();
    let mean: Vec<_> = (0..dimensions)
        .map(|d| points.iter().map(|p| p[d]).sum::<f64>() / count(points.len()))
        .collect();
    let covariance = covariance(points, &vec![1.; points.len()], &mean, count(points.len()));
    let mut centers = vec![points[0].clone()];
    // Farthest-point initialization separates centers without a hidden RNG source.
    while centers.len() < components {
        let mut next = 0;
        let mut greatest = -1.;
        for (index, point) in points.iter().enumerate() {
            let nearest = centers
                .iter()
                .map(|center| distance(point, center))
                .fold(f64::INFINITY, f64::min);
            if nearest > greatest {
                greatest = nearest;
                next = index;
            }
        }
        centers.push(points[next].clone());
    }
    centers
        .into_iter()
        .map(|mean| Gaussian::new(1. / count(components), mean, &covariance))
        .collect()
}

fn expectation(
    points: &[Vec<f64>],
    model: &[Gaussian],
    observer: &dyn ClusterObserver,
) -> Result<(f64, Vec<Vec<f64>>), FitFailure> {
    let mut likelihood = 0.;
    let mut probabilities = Vec::with_capacity(points.len());
    for point in points {
        check(observer)?;
        let logs: Vec<_> = model.iter().map(|g| g.log_probability(point)).collect();
        let maximum = logs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let total = logs.iter().map(|p| (p - maximum).exp()).sum::<f64>();
        if !maximum.is_finite() || !total.is_finite() || total <= 0. {
            return Err(FitFailure::Numerical);
        }
        likelihood += maximum + total.ln();
        probabilities.push(
            logs.into_iter()
                .map(|p| (p - maximum).exp() / total)
                .collect(),
        );
    }
    if !likelihood.is_finite() {
        return Err(FitFailure::Numerical);
    }
    Ok((likelihood, probabilities))
}

fn maximize(
    points: &[Vec<f64>],
    probabilities: &[Vec<f64>],
    observer: &dyn ClusterObserver,
) -> Result<Vec<Gaussian>, FitFailure> {
    let dimensions = points[0].len();
    let components = probabilities[0].len();
    let mut model = Vec::with_capacity(components);
    for component in 0..components {
        check(observer)?;
        let weights: Vec<_> = probabilities.iter().map(|row| row[component]).collect();
        let mass = weights.iter().sum::<f64>();
        if !mass.is_finite() || mass < 1e-10 {
            return Err(FitFailure::Numerical);
        }
        let mean: Vec<_> = (0..dimensions)
            .map(|d| {
                points
                    .iter()
                    .zip(&weights)
                    .map(|(p, w)| w * p[d])
                    .sum::<f64>()
                    / mass
            })
            .collect();
        let covariance = covariance(points, &weights, &mean, mass);
        model.push(Gaussian::new(
            mass / count(points.len()),
            mean,
            &covariance,
        )?);
    }
    Ok(model)
}

fn fit(
    points: &[Vec<f64>],
    components: usize,
    iterations: u16,
    observer: &dyn ClusterObserver,
) -> Result<(f64, Vec<Vec<f64>>), FitFailure> {
    check(observer)?;
    let mut model = initialize(points, components)?;
    let mut previous = f64::NEG_INFINITY;
    for _ in 0..iterations {
        let (likelihood, probabilities) = expectation(points, &model, observer)?;
        if (likelihood - previous).abs() / count(points.len()) < TOLERANCE {
            return Ok((likelihood, probabilities));
        }
        model = maximize(points, &probabilities, observer)?;
        previous = likelihood;
    }
    Err(FitFailure::NotConverged)
}

/// Fit each admissible component count and select the smallest full-covariance BIC.
pub fn select_mixture(
    points: &[Vec<f64>],
    options: &RaptorOptions,
    observer: &dyn ClusterObserver,
) -> Result<MixtureSelection, IndexError> {
    options.validate()?;
    let dimensions = points.first().map_or(0, Vec::len);
    if !(2..=MAX_CLUSTER_INPUTS).contains(&points.len())
        || !(1..=16).contains(&dimensions)
        || points
            .iter()
            .any(|p| p.len() != dimensions || p.iter().any(|v| !v.is_finite() || v.abs() > 1e12))
    {
        return Err(IndexError(
            "Mixture fitting needs 2–512 finite reduced vectors with 1–16 matching dimensions."
                .into(),
        ));
    }
    let maximum = usize::from(options.max_components).min(points.len() - 1);
    let mut best: Option<MixtureSelection> = None;
    let mut failed = 0;
    for components in 1..=maximum {
        match fit(points, components, options.gmm_iterations, observer) {
            Ok((likelihood, probabilities)) => {
                let bic = gaussian_bic(likelihood, points.len(), dimensions, components)?;
                if best.as_ref().is_none_or(|best| bic < best.bic) {
                    best = Some(MixtureSelection {
                        groups: soft_memberships(&probabilities, options.membership_threshold)?,
                        components,
                        bic,
                        attempted: 0,
                        failed: 0,
                    });
                }
            }
            Err(FitFailure::Cancelled) => {
                return Err(IndexError(
                    "Clustering was cancelled or exceeded its computation budget.".into(),
                ));
            }
            Err(FitFailure::Numerical | FitFailure::NotConverged) => failed += 1,
        }
    }
    let mut best=best.ok_or_else(||IndexError("No Gaussian mixture converged for these inputs. Reduce the scope or change the embedding model.".into()))?;
    best.attempted = u32::try_from(maximum).expect("bounded mixture candidates");
    best.failed = failed;
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Observe(AtomicUsize);
    impl ClusterObserver for Observe {
        fn cancelled(&self) -> bool {
            self.0.fetch_sub(1, Ordering::Relaxed) == 0
        }
    }
    #[test]
    fn full_covariance_density_matches_a_correlated_analytic_gaussian() {
        let g = Gaussian::new(1., vec![0., 0.], &[vec![4., 1.], vec![1., 1.]]).unwrap();
        let expected = -(2. * std::f64::consts::PI).ln() - 0.5 * 3_f64.ln() - 0.5 * 4. / 3.;
        assert!((g.log_probability(&[2., 1.]) - expected).abs() < 1e-12);
        assert!(Gaussian::new(1., vec![0., 0.], &[vec![1., 2.], vec![2., 1.]]).is_err());
    }
    #[test]
    fn likelihood_remains_finite_for_distant_points_and_normalizes_posteriors() {
        let model = vec![
            Gaussian::new(0.5, vec![-100.], &[vec![1.]]).unwrap(),
            Gaussian::new(0.5, vec![100.], &[vec![1.]]).unwrap(),
        ];
        let (likelihood, probabilities) = expectation(
            &[vec![0.], vec![1000.]],
            &model,
            &Observe(AtomicUsize::new(100)),
        )
        .unwrap();
        assert!(likelihood.is_finite());
        assert!((probabilities[0][0] - 0.5).abs() < 1e-12);
        for row in probabilities {
            assert!((row.iter().sum::<f64>() - 1.).abs() < 1e-12);
        }
    }
    #[test]
    fn em_recovers_full_covariance_likelihood_and_handles_coincident_points() {
        let points = vec![vec![-2., -1.], vec![2., 1.], vec![-1., 0.], vec![1., 0.]];
        let options = RaptorOptions {
            max_components: 1,
            ..RaptorOptions::default()
        };
        let result =
            select_mixture(&points, &options, &Observe(AtomicUsize::new(100_000))).unwrap();
        // The sample covariance is [[2.5,1],[1,0.5]], before diagonal regularization.
        let determinant = 2.500_001 * 0.500_001 - 1.;
        let likelihood = -4. * (2. * std::f64::consts::PI).ln()
            - 2. * f64::ln(determinant)
            - 0.5 * 2.000_012 / determinant;
        assert!((result.bic - (-2. * likelihood + 5. * 4_f64.ln())).abs() < 1e-10);
        let same = select_mixture(
            &vec![vec![3., 3.]; 8],
            &RaptorOptions::default(),
            &Observe(AtomicUsize::new(100_000)),
        )
        .unwrap();
        assert_eq!(same.components, 1);
        assert_eq!(same.groups, vec![(0..8).collect::<Vec<_>>()]);
    }
    #[test]
    fn bic_selects_separated_populations_and_fitting_observes_cancellation() {
        let points: Vec<_> = (0..120_u32)
            .map(|i| {
                let u = (f64::from((i * 37) % 120) + 0.5) / 120.;
                let v = (f64::from((i * 71) % 120) + 0.5) / 120.;
                let noise = (-2. * u.ln()).sqrt() * (2. * std::f64::consts::PI * v).cos();
                vec![if i % 2 == 0 {
                    -5. + noise * 0.3
                } else {
                    5. + noise * 0.3
                }]
            })
            .collect();
        let options = RaptorOptions {
            max_components: 4,
            ..RaptorOptions::default()
        };
        let selected =
            select_mixture(&points, &options, &Observe(AtomicUsize::new(1_000_000))).unwrap();
        assert_eq!(selected.components, 2);
        assert_eq!(selected.attempted, 4);
        assert_eq!(selected.groups.len(), 2);
        assert!(
            selected
                .groups
                .iter()
                .all(|g| g.iter().all(|i| i % 2 == g[0] % 2))
        );
        assert!(
            select_mixture(&points, &options, &Observe(AtomicUsize::new(15)))
                .unwrap_err()
                .0
                .contains("cancelled")
        );
        assert!(
            select_mixture(
                &[vec![f64::NAN], vec![1.]],
                &options,
                &Observe(AtomicUsize::new(100))
            )
            .is_err()
        );
    }
}

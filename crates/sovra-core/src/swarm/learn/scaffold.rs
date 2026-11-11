//! SCAFFOLD: stochastic controlled averaging (Karimireddy et al., ICML 2020,
//! <https://arxiv.org/abs/1910.06378>).
//!
//! Departments hold very different data (code vs. contracts). With plain federated
//! averaging, each computer's many local steps drift toward its own optimum and the
//! average lands somewhere none of them wants. SCAFFOLD keeps a **control variate**
//! per computer (`cᵢ`) and one for the fleet (`c`); each local step is corrected by
//! `c − cᵢ`, the estimated difference between the fleet's and this computer's
//! gradient, which pulls local trajectories back toward the global optimum.
//!
//! **Only the parameter change leaves a computer.** With the paper's option II,
//! `Δcᵢ = −c − Δyᵢ / (Kη)` exactly, so the fleet derives the mean control change
//! from the mean parameter change it already has. Sending `Δcᵢ` too would double the
//! traffic and the privacy noise for no information.

/// Something a computer can take gradients of on its own data. The data never
/// leaves the implementation.
pub trait Objective {
    fn dimension(&self) -> usize;
    fn loss(&self, params: &[f32]) -> f32;
    fn gradient(&self, params: &[f32]) -> Vec<f32>;
}

/// Plain averaging, or SCAFFOLD's corrected steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    FedAvg,
    Scaffold,
}

/// How much local work each computer does per round. Every computer in a round
/// uses the same values; the fleet needs them to derive the control update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalWork {
    pub steps: usize,
    pub learning_rate: f32,
    pub algorithm: Algorithm,
}

impl LocalWork {
    /// `K·η`.
    #[expect(clippy::cast_precision_loss, reason = "local step counts are small")]
    fn span(&self) -> f32 {
        self.steps.max(1) as f32 * self.learning_rate
    }
}

/// One computer's local state across rounds.
#[derive(Debug, Clone, PartialEq)]
pub struct ScaffoldClient {
    pub control: Vec<f32>,
}

impl ScaffoldClient {
    pub fn new(dimension: usize) -> Self {
        Self {
            control: vec![0.0; dimension],
        }
    }

    /// Local steps from the fleet's `global` parameters. Returns `Δy`, the only thing
    /// that is sent, and updates this computer's own control variate (option II).
    pub fn train(
        &mut self,
        objective: &dyn Objective,
        global: &[f32],
        fleet_control: &[f32],
        work: LocalWork,
    ) -> Vec<f32> {
        let mut y = global.to_vec();
        for _ in 0..work.steps {
            let g = objective.gradient(&y);
            for i in 0..y.len() {
                let correction = match work.algorithm {
                    Algorithm::Scaffold => fleet_control[i] - self.control[i],
                    Algorithm::FedAvg => 0.0,
                };
                y[i] -= work.learning_rate * (g[i] + correction);
            }
        }
        let delta: Vec<f32> = y.iter().zip(global).map(|(a, b)| a - b).collect();
        if work.algorithm == Algorithm::Scaffold {
            let span = work.span();
            for i in 0..delta.len() {
                self.control[i] = self.control[i] - fleet_control[i] - delta[i] / span;
            }
        }
        delta
    }
}

/// The fleet's model and control variate.
#[derive(Debug, Clone, PartialEq)]
pub struct ScaffoldServer {
    pub params: Vec<f32>,
    pub control: Vec<f32>,
    /// The global step size `η_g`.
    pub global_rate: f32,
    /// How many computers could take part (`N`), for the control update.
    pub fleet_size: usize,
    pub work: LocalWork,
}

impl ScaffoldServer {
    pub fn new(params: Vec<f32>, fleet_size: usize, work: LocalWork) -> Self {
        let control = vec![0.0; params.len()];
        Self {
            params,
            control,
            global_rate: 1.0,
            fleet_size: fleet_size.max(1),
            work,
        }
    }

    /// Applies the **mean** `Δy` of `participants` computers. The mean comes from the
    /// secure sum, so the fleet never sees any single computer's change.
    pub fn apply_mean(&mut self, mean_delta: &[f32], participants: usize) {
        #[expect(clippy::cast_precision_loss, reason = "fleet sizes are small")]
        let share = participants as f32 / self.fleet_size as f32;
        let span = self.work.span();
        let scaffold = self.work.algorithm == Algorithm::Scaffold;
        for ((param, control), delta) in self
            .params
            .iter_mut()
            .zip(self.control.iter_mut())
            .zip(mean_delta)
        {
            *param += self.global_rate * delta;
            if scaffold {
                // mean Δcᵢ = −c − mean Δyᵢ / (Kη)
                *control += share * (-*control - delta / span);
            }
        }
    }
}

/// The element-wise mean of vectors (the non-private path, for tests).
pub fn mean(deltas: &[Vec<f32>]) -> Vec<f32> {
    let dimension = deltas.first().map_or(0, Vec::len);
    #[expect(clippy::cast_precision_loss, reason = "fleet sizes are small")]
    let n = deltas.len().max(1) as f32;
    (0..dimension)
        .map(|i| deltas.iter().map(|d| d[i]).sum::<f32>() / n)
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// ½ Σ hᵢ (x − aᵢ)²: a department whose optimum is `a` with curvature `h`.
    #[derive(Debug, Clone)]
    pub(crate) struct Quadratic {
        pub(crate) curvature: Vec<f32>,
        pub(crate) optimum: Vec<f32>,
    }

    impl Objective for Quadratic {
        fn dimension(&self) -> usize {
            self.optimum.len()
        }
        fn loss(&self, params: &[f32]) -> f32 {
            params
                .iter()
                .zip(&self.optimum)
                .zip(&self.curvature)
                .map(|((x, a), h)| 0.5 * h * (x - a) * (x - a))
                .sum()
        }
        fn gradient(&self, params: &[f32]) -> Vec<f32> {
            params
                .iter()
                .zip(&self.optimum)
                .zip(&self.curvature)
                .map(|((x, a), h)| h * (x - a))
                .collect()
        }
    }

    /// Three departments that disagree: different optima and curvatures per axis.
    pub(crate) fn departments() -> Vec<Quadratic> {
        vec![
            Quadratic {
                curvature: vec![10.0, 0.1, 1.0],
                optimum: vec![1.0, -4.0, 2.0],
            },
            Quadratic {
                curvature: vec![0.1, 10.0, 1.0],
                optimum: vec![-3.0, 2.0, -1.0],
            },
            Quadratic {
                curvature: vec![1.0, 1.0, 10.0],
                optimum: vec![0.0, 0.0, 5.0],
            },
        ]
    }

    /// The minimiser of the sum: Σ hᵢ aᵢ / Σ hᵢ per axis.
    pub(crate) fn global_optimum(parts: &[Quadratic]) -> Vec<f32> {
        let d = parts[0].dimension();
        (0..d)
            .map(|j| {
                let num: f32 = parts.iter().map(|p| p.curvature[j] * p.optimum[j]).sum();
                let den: f32 = parts.iter().map(|p| p.curvature[j]).sum();
                num / den
            })
            .collect()
    }

    pub(crate) fn distance(a: &[f32], b: &[f32]) -> f32 {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).powi(2))
            .sum::<f32>()
            .sqrt()
    }

    pub(crate) const WORK: LocalWork = LocalWork {
        steps: 20,
        learning_rate: 0.05,
        algorithm: Algorithm::Scaffold,
    };

    fn run(algorithm: Algorithm, rounds: usize) -> Vec<f32> {
        let parts = departments();
        let work = LocalWork { algorithm, ..WORK };
        let mut server = ScaffoldServer::new(vec![0.0; 3], parts.len(), work);
        let mut clients: Vec<ScaffoldClient> =
            parts.iter().map(|_| ScaffoldClient::new(3)).collect();
        for _ in 0..rounds {
            let deltas: Vec<Vec<f32>> = clients
                .iter_mut()
                .zip(&parts)
                .map(|(c, p)| c.train(p, &server.params, &server.control, work))
                .collect();
            server.apply_mean(&mean(&deltas), deltas.len());
        }
        server.params
    }

    #[test]
    fn scaffold_corrects_the_drift_that_breaks_fedavg_on_non_iid_data() {
        let target = global_optimum(&departments());
        let fedavg = distance(&run(Algorithm::FedAvg, 200), &target);
        let scaffold = distance(&run(Algorithm::Scaffold, 200), &target);
        assert!(scaffold < 0.01, "SCAFFOLD ends {scaffold} from the optimum");
        assert!(fedavg > 0.5, "FedAvg should drift, ended {fedavg} away");
    }

    #[test]
    fn the_fleet_derives_the_same_control_update_it_would_have_been_sent() {
        let parts = departments();
        let mut client = ScaffoldClient::new(3);
        let fleet_control = vec![0.3, -0.2, 0.1];
        let before = client.control.clone();
        let delta = client.train(&parts[0], &[0.5, 0.5, 0.5], &fleet_control, WORK);
        let sent: Vec<f32> = client
            .control
            .iter()
            .zip(&before)
            .map(|(a, b)| a - b)
            .collect();
        let derived: Vec<f32> = (0..3)
            .map(|i| -fleet_control[i] - delta[i] / (20.0 * 0.05))
            .collect();
        assert!(distance(&sent, &derived) < 1e-5);
    }
}

//! `GaLore`: gradient low-rank projection (Zhao et al., ICML 2024,
//! <https://arxiv.org/abs/2403.03507>).
//!
//! A weight matrix's gradient `G` (m×n) is projected into an r-dimensional
//! subspace, Adam's two moment buffers live there (r×n instead of m×n), and the
//! normalised step is projected back. The subspace is refreshed every few hundred
//! steps from the gradient's top singular vectors. This is what lets a consumer GPU
//! fine-tune a 7B model: optimizer state drops from tens of gigabytes to one or
//! two ([`optimizer_state_bytes`]).
//!
//! **`FedLore`** is the federated use: every computer in a round builds the *same*
//! random projector from the round's seed ([`Projector::shared`]), so each sends an
//! r×n update instead of m×n, and because projection is linear the masked secure
//! sum of projected updates is the projection of the sum.

use super::RandomSource;
use super::linalg::Matrix;

/// Which side of the gradient the basis acts on. `GaLore` projects the shorter side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// `R = Pᵀ G` with `P` m×r (when m ≤ n).
    Left,
    /// `R = G Q` with `Q` n×r (when m > n).
    Right,
}

/// An orthonormal basis for a gradient's low-rank subspace.
#[derive(Debug, Clone, PartialEq)]
pub struct Projector {
    pub side: Side,
    pub basis: Matrix,
}

fn side_for(rows: usize, cols: usize) -> Side {
    if rows <= cols {
        Side::Left
    } else {
        Side::Right
    }
}

impl Projector {
    /// The top-`rank` singular subspace of `gradient`, by randomized subspace
    /// iteration (Halko, Martinsson & Tropp 2011): a few multiplications instead of
    /// a full SVD.
    pub fn from_gradient(
        gradient: &Matrix,
        rank: usize,
        iterations: usize,
        rng: &mut dyn RandomSource,
    ) -> Self {
        let side = side_for(gradient.rows, gradient.cols);
        // Work on the matrix whose *rows* are the side to keep.
        let a = match side {
            Side::Left => gradient.clone(),
            Side::Right => gradient.transpose(),
        };
        let rank = rank.min(a.rows).min(a.cols).max(1);
        let mut q = a.matmul(&Matrix::gaussian(a.cols, rank, rng));
        q.orthonormalize_columns();
        for _ in 0..iterations {
            let mut z = a.matmul_tn(&q);
            z.orthonormalize_columns();
            q = a.matmul(&z);
            q.orthonormalize_columns();
        }
        Self { side, basis: q }
    }

    /// A random orthonormal basis that every computer derives from the same seed.
    pub fn shared(rows: usize, cols: usize, rank: usize, rng: &mut dyn RandomSource) -> Self {
        let side = side_for(rows, cols);
        let keep = match side {
            Side::Left => rows,
            Side::Right => cols,
        };
        let mut basis = Matrix::gaussian(keep, rank.min(keep).max(1), rng);
        basis.orthonormalize_columns();
        Self { side, basis }
    }

    pub fn rank(&self) -> usize {
        self.basis.cols
    }

    /// Full gradient → low-rank coordinates.
    pub fn project(&self, gradient: &Matrix) -> Matrix {
        match self.side {
            Side::Left => self.basis.matmul_tn(gradient),
            Side::Right => gradient.matmul(&self.basis),
        }
    }

    /// Low-rank coordinates → full shape.
    pub fn project_back(&self, low: &Matrix) -> Matrix {
        match self.side {
            Side::Left => self.basis.matmul(low),
            Side::Right => low.matmul(&self.basis.transpose()),
        }
    }
}

/// Adam whose moments live in a `GaLore` subspace.
#[derive(Debug, Clone)]
pub struct GaloreAdam {
    pub rank: usize,
    /// Steps between subspace refreshes (`GaLore` uses ~200).
    pub refresh_every: u64,
    /// `GaLore`'s `α`, scaling the projected-back step.
    pub scale: f32,
    pub learning_rate: f32,
    pub beta1: f32,
    pub beta2: f32,
    pub epsilon: f32,
    step: u64,
    projector: Option<Projector>,
    m: Option<Matrix>,
    v: Option<Matrix>,
}

impl GaloreAdam {
    pub fn new(rank: usize, learning_rate: f32) -> Self {
        Self {
            rank,
            refresh_every: 200,
            scale: 0.25,
            learning_rate,
            beta1: 0.9,
            beta2: 0.999,
            epsilon: 1e-8,
            step: 0,
            projector: None,
            m: None,
            v: None,
        }
    }

    /// Floats of optimizer state held now (moments plus basis).
    pub fn state_len(&self) -> usize {
        let moment = |m: &Option<Matrix>| m.as_ref().map_or(0, |m| m.data.len());
        moment(&self.m)
            + moment(&self.v)
            + self.projector.as_ref().map_or(0, |p| p.basis.data.len())
    }

    /// One update of `weight` from `gradient`.
    pub fn step(&mut self, weight: &mut Matrix, gradient: &Matrix, rng: &mut dyn RandomSource) {
        if self.projector.is_none() || self.step.is_multiple_of(self.refresh_every) {
            self.projector = Some(Projector::from_gradient(gradient, self.rank, 2, rng));
        }
        self.step += 1;
        let projector = self.projector.as_ref().expect("set just above");
        let low = projector.project(gradient);
        let m = self
            .m
            .get_or_insert_with(|| Matrix::zeros(low.rows, low.cols));
        let v = self
            .v
            .get_or_insert_with(|| Matrix::zeros(low.rows, low.cols));
        let t = i32::try_from(self.step).unwrap_or(i32::MAX);
        let (bc1, bc2) = (1.0 - self.beta1.powi(t), 1.0 - self.beta2.powi(t));
        let mut normalised = Matrix::zeros(low.rows, low.cols);
        for (i, g) in low.data.iter().enumerate() {
            m.data[i] = self.beta1 * m.data[i] + (1.0 - self.beta1) * g;
            v.data[i] = self.beta2 * v.data[i] + (1.0 - self.beta2) * g * g;
            let m_hat = m.data[i] / bc1;
            let v_hat = v.data[i] / bc2;
            normalised.data[i] = m_hat / (v_hat.sqrt() + self.epsilon);
        }
        let update = projector.project_back(&normalised);
        for (w, u) in weight.data.iter_mut().zip(&update.data) {
            *w -= self.learning_rate * self.scale * u;
        }
    }
}

/// One kind of weight matrix in a model and how many there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeightShape {
    pub rows: u64,
    pub cols: u64,
    pub count: u64,
}

/// The 2-D weights of a Llama-style 7B model (32 layers, width 4096, MLP 11008,
/// vocabulary 32000): about 6.7 billion parameters.
pub fn llama_7b() -> Vec<WeightShape> {
    let (hidden, mlp, vocab, layers) = (4096, 11008, 32000, 32);
    vec![
        WeightShape {
            rows: vocab,
            cols: hidden,
            count: 2,
        },
        WeightShape {
            rows: hidden,
            cols: hidden,
            count: 4 * layers,
        },
        WeightShape {
            rows: hidden,
            cols: mlp,
            count: 2 * layers,
        },
        WeightShape {
            rows: mlp,
            cols: hidden,
            count: layers,
        },
        WeightShape {
            rows: 1,
            cols: hidden,
            count: 2 * layers + 1,
        },
    ]
}

pub fn parameter_count(shapes: &[WeightShape]) -> u64 {
    shapes.iter().map(|s| s.rows * s.cols * s.count).sum()
}

/// Bytes of Adam optimizer state: two moments per parameter, or with `GaLore` at
/// `rank`, two r×long moments and an r×short basis for each matrix it can project.
pub fn optimizer_state_bytes(
    shapes: &[WeightShape],
    rank: Option<u64>,
    bytes_per_value: u64,
) -> u64 {
    shapes
        .iter()
        .map(|s| {
            let (short, long) = (s.rows.min(s.cols), s.rows.max(s.cols));
            let per = match rank {
                Some(r) if short > r => 2 * r * long + r * short,
                _ => 2 * s.rows * s.cols,
            };
            per * s.count * bytes_per_value
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::testing::SeededRandom;

    const GB: u64 = 1_000_000_000;

    #[test]
    fn galore_brings_7b_optimizer_state_under_four_gigabytes() {
        let shapes = llama_7b();
        let params = parameter_count(&shapes);
        assert!((6_400_000_000..7_000_000_000).contains(&params), "{params}");
        let full = optimizer_state_bytes(&shapes, None, 2);
        let galore = optimizer_state_bytes(&shapes, Some(128), 2);
        assert!(full > 24 * GB, "full Adam needs {full} bytes");
        assert!(galore < 4 * GB, "GaLore needs {galore} bytes");
    }

    #[test]
    fn a_low_rank_gradient_survives_projection_exactly() {
        let mut rng = SeededRandom(1);
        // G = U Vᵀ with rank 3.
        let u = Matrix::gaussian(12, 3, &mut rng);
        let v = Matrix::gaussian(30, 3, &mut rng);
        let g = u.matmul(&v.transpose());
        let projector = Projector::from_gradient(&g, 3, 2, &mut rng);
        let back = projector.project_back(&projector.project(&g));
        assert!(back.sub(&g).frobenius() / g.frobenius() < 1e-3);
        // A tall gradient projects on the right.
        let tall = g.transpose();
        let projector = Projector::from_gradient(&tall, 3, 2, &mut rng);
        assert_eq!(projector.side, Side::Right);
        let back = projector.project_back(&projector.project(&tall));
        assert!(back.sub(&tall).frobenius() / tall.frobenius() < 1e-3);
    }

    #[test]
    fn galore_adam_fits_a_matrix_with_small_state() {
        let mut rng = SeededRandom(3);
        let target = Matrix::gaussian(16, 4, &mut rng).matmul(&Matrix::gaussian(4, 48, &mut rng));
        let mut w = Matrix::zeros(16, 48);
        let mut opt = GaloreAdam::new(4, 0.2);
        opt.refresh_every = 25;
        opt.scale = 1.0;
        let start = target.frobenius();
        for _ in 0..400 {
            let grad = w.sub(&target); // ∇ ½‖W − T‖²
            opt.step(&mut w, &grad, &mut rng);
        }
        let end = w.sub(&target).frobenius();
        assert!(end < start * 0.05, "{start} -> {end}");
        assert!(
            opt.state_len() < 16 * 48,
            "state {} vs full {}",
            opt.state_len(),
            2 * 16 * 48
        );
    }

    #[test]
    fn shared_projectors_agree_across_computers_and_are_linear() {
        let a = Projector::shared(8, 40, 4, &mut SeededRandom(99));
        let b = Projector::shared(8, 40, 4, &mut SeededRandom(99));
        assert_eq!(a, b);
        let mut rng = SeededRandom(5);
        let (x, y) = (
            Matrix::gaussian(8, 40, &mut rng),
            Matrix::gaussian(8, 40, &mut rng),
        );
        let sum = Matrix::from_vec(
            8,
            40,
            x.data.iter().zip(&y.data).map(|(p, q)| p + q).collect(),
        );
        let sep = a.project(&x);
        let sep2 = a.project(&y);
        let joined = a.project(&sum);
        for i in 0..joined.data.len() {
            assert!((joined.data[i] - sep.data[i] - sep2.data[i]).abs() < 1e-4);
        }
    }
}

//! The little dense linear algebra the swarm's rules need: row-major `f32`
//! matrices, products and orthonormalisation. Heavy training kernels belong to the
//! compute backend; this is for projections, aggregation and tests.

use super::RandomSource;

/// A row-major dense matrix.
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<f32>,
}

impl Matrix {
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            data: vec![0.0; rows * cols],
        }
    }

    pub fn from_vec(rows: usize, cols: usize, data: Vec<f32>) -> Self {
        assert_eq!(data.len(), rows * cols, "data must fill the matrix");
        Self { rows, cols, data }
    }

    /// Entries drawn from a standard normal distribution.
    pub fn gaussian(rows: usize, cols: usize, rng: &mut dyn RandomSource) -> Self {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "test data and random bases only need f32 precision"
        )]
        let data = (0..rows * cols)
            .map(|_| super::standard_normal(rng) as f32)
            .collect();
        Self { rows, cols, data }
    }

    pub fn at(&self, row: usize, col: usize) -> f32 {
        self.data[row * self.cols + col]
    }

    #[must_use]
    pub fn transpose(&self) -> Self {
        let mut out = Self::zeros(self.cols, self.rows);
        for r in 0..self.rows {
            for c in 0..self.cols {
                out.data[c * self.rows + r] = self.data[r * self.cols + c];
            }
        }
        out
    }

    /// `self × other`.
    #[must_use]
    pub fn matmul(&self, other: &Self) -> Self {
        assert_eq!(self.cols, other.rows, "inner dimensions must agree");
        let mut out = Self::zeros(self.rows, other.cols);
        for r in 0..self.rows {
            for k in 0..self.cols {
                let a = self.data[r * self.cols + k];
                if a == 0.0 {
                    continue;
                }
                let row = &other.data[k * other.cols..(k + 1) * other.cols];
                let dst = &mut out.data[r * other.cols..(r + 1) * other.cols];
                for (d, b) in dst.iter_mut().zip(row) {
                    *d += a * b;
                }
            }
        }
        out
    }

    /// `selfᵀ × other`.
    #[must_use]
    pub fn matmul_tn(&self, other: &Self) -> Self {
        self.transpose().matmul(other)
    }

    #[must_use]
    pub fn sub(&self, other: &Self) -> Self {
        assert_eq!(
            (self.rows, self.cols),
            (other.rows, other.cols),
            "shapes must agree"
        );
        let data = self
            .data
            .iter()
            .zip(&other.data)
            .map(|(a, b)| a - b)
            .collect();
        Self::from_vec(self.rows, self.cols, data)
    }

    pub fn frobenius(&self) -> f32 {
        self.data.iter().map(|x| x * x).sum::<f32>().sqrt()
    }

    /// Makes the columns orthonormal in place (modified Gram–Schmidt). Columns that
    /// collapse to zero are replaced by zero, which keeps projections well defined.
    pub fn orthonormalize_columns(&mut self) {
        for c in 0..self.cols {
            for prev in 0..c {
                let dot: f32 = (0..self.rows)
                    .map(|r| self.at(r, c) * self.at(r, prev))
                    .sum();
                for r in 0..self.rows {
                    let p = self.at(r, prev);
                    self.data[r * self.cols + c] -= dot * p;
                }
            }
            let norm: f32 = (0..self.rows)
                .map(|r| self.at(r, c).powi(2))
                .sum::<f32>()
                .sqrt();
            for r in 0..self.rows {
                let v = &mut self.data[r * self.cols + c];
                *v = if norm > 1e-12 { *v / norm } else { 0.0 };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::testing::SeededRandom;

    #[test]
    fn orthonormal_columns_have_identity_gram() {
        let mut q = Matrix::gaussian(20, 5, &mut SeededRandom(7));
        q.orthonormalize_columns();
        let gram = q.matmul_tn(&q);
        for r in 0..5 {
            for c in 0..5 {
                let want = if r == c { 1.0 } else { 0.0 };
                assert!((gram.at(r, c) - want).abs() < 1e-4, "{r},{c}");
            }
        }
    }

    #[test]
    fn products_match_by_hand() {
        let a = Matrix::from_vec(2, 3, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let b = Matrix::from_vec(3, 1, vec![1.0, 0.0, -1.0]);
        assert_eq!(a.matmul(&b).data, [-2.0, -2.0]);
        assert_eq!(a.matmul_tn(&a).rows, 3);
    }
}

//! Exact H0/H1 of a bounded Euclidean Vietoris–Rips filtration over GF(2).
//! A spanning forest identifies H0 deaths and the positive edges that birth
//! H1. Project each triangle boundary onto those edges and reduce by its
//! youngest pivot. Only independent boundaries occupy the pivot table.

pub const MAX_POINTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    pub birth: f64,
    pub death: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Persistence {
    pub points: usize,
    pub component_deaths: Vec<f64>,
    pub loops: Vec<Interval>,
}

impl Persistence {
    pub fn at(&self, radius: f64) -> (usize, usize) {
        (
            1 + self
                .component_deaths
                .iter()
                .filter(|d| **d > radius)
                .count(),
            self.loops
                .iter()
                .filter(|bar| bar.birth <= radius && radius < bar.death)
                .count(),
        )
    }
}

/// Coordinates in one shared scale. A caller must explicitly select a bounded
/// window for larger data so omitted points never masquerade as the full shape.
pub fn persistence(points: &[Vec<f64>]) -> Result<Persistence, &'static str> {
    let n = points.len();
    if !(2..=MAX_POINTS).contains(&n) {
        return Err("Choose 2–64 points for exact shape analysis.");
    }
    let dimensions = points[0].len();
    if !(1..=16).contains(&dimensions)
        || points
            .iter()
            .any(|p| p.len() != dimensions || p.iter().any(|v| !v.is_finite()))
    {
        return Err("Use equal rows of 1–16 finite coordinates.");
    }
    let mut edges = Vec::with_capacity(n * (n - 1) / 2);
    for (i, left) in points.iter().enumerate() {
        for (j, right) in points[..i].iter().enumerate() {
            let distance = left
                .iter()
                .zip(right)
                .fold(0.0_f64, |norm, (a, b)| norm.hypot(a - b));
            if !distance.is_finite() {
                return Err("These coordinates exceed the supported distance range.");
            }
            edges.push((distance, i, j));
        }
    }
    edges.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then((a.1, a.2).cmp(&(b.1, b.2))));
    let mut edge_id = vec![0; n * n];
    let mut forest: Vec<_> = (0..n).collect();
    let mut generators = vec![None; edges.len()];
    let mut births = Vec::new();
    let mut component_deaths = Vec::with_capacity(n - 1);
    for (id, &(distance, i, j)) in edges.iter().enumerate() {
        edge_id[i * n + j] = id;
        edge_id[j * n + i] = id;
        let a = root(&mut forest, i);
        let b = root(&mut forest, j);
        if a == b {
            generators[id] = Some(births.len());
            births.push(distance);
        } else {
            forest[a] = b;
            component_deaths.push(distance);
        }
    }
    // Activate edges in filtration order. Common neighbors are precisely the
    // triangles introduced by this edge, including distance ties in edge order.
    // A 64-bit neighborhood avoids storing and sorting all n-choose-3 triangles.
    let mut neighbors = vec![0_u64; n];
    let words = births.len().div_ceil(64);
    let mut pivots = vec![None::<Vec<u64>>; births.len()];
    let mut filled = 0;
    let mut column = vec![0_u64; words];
    let mut loops = Vec::new();
    for (last, &(death, i, j)) in edges.iter().enumerate() {
        let mut common = neighbors[i] & neighbors[j];
        neighbors[i] |= 1_u64 << j;
        neighbors[j] |= 1_u64 << i;
        while common != 0 {
            let k = common.trailing_zeros() as usize;
            common &= common - 1;
            let face = [last, edge_id[i * n + k], edge_id[j * n + k]];
            let mut active = 0;
            for index in face.into_iter().filter_map(|e| generators[e]) {
                let word = index / 64;
                active = active.max(word + 1);
                column[word] ^= 1_u64 << (index % 64);
            }
            while active > 0 {
                let word = active - 1;
                let bits = column[word];
                if bits == 0 {
                    active -= 1;
                    continue;
                }
                let pivot = word * 64 + 63 - bits.leading_zeros() as usize;
                if let Some(prior) = &pivots[pivot] {
                    // The pivot guarantees all higher words are already zero.
                    for (a, b) in column[..active].iter_mut().zip(prior) {
                        *a ^= b;
                    }
                } else {
                    pivots[pivot] = Some(column[..active].to_vec());
                    column[..active].fill(0);
                    filled += 1;
                    if births[pivot] < death {
                        loops.push(Interval {
                            birth: births[pivot],
                            death,
                        });
                    }
                    break;
                }
            }
        }
        if filled == births.len() {
            break;
        }
    }
    loops.sort_by(|a, b| (b.death - b.birth).total_cmp(&(a.death - a.birth)));
    Ok(Persistence {
        points: n,
        component_deaths,
        loops,
    })
}

fn root(parents: &mut [usize], mut node: usize) -> usize {
    while parents[node] != node {
        parents[node] = parents[parents[node]];
        node = parents[node];
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ring_and_separated_clouds_at_known_distances() {
        let points = vec![vec![0., 0.], vec![1., 0.], vec![1., 1.], vec![0., 1.]];
        let bars = persistence(&points).unwrap();
        assert_eq!(bars.at(0.5), (4, 0));
        assert_eq!(bars.at(1.01), (1, 1));
        assert_eq!(bars.at(1.42), (1, 0));
        let mut noisy = points;
        noisy[0][0] = 0.01;
        assert_eq!(persistence(&noisy).unwrap().at(1.1), (1, 1));
        let groups = vec![vec![0.], vec![0.1], vec![5.], vec![5.1]];
        assert_eq!(persistence(&groups).unwrap().at(0.2), (2, 0));
        assert!(persistence(&[vec![f64::NAN], vec![1.]]).is_err());
    }

    #[test]
    fn component_and_loop_counts_match_boundary_matrix_ranks_on_tied_point_sets() {
        fn rank(columns: Vec<u64>) -> usize {
            let mut basis = [0_u64; 64];
            for mut col in columns {
                while col != 0 {
                    let pivot = col.trailing_zeros() as usize;
                    if basis[pivot] == 0 {
                        basis[pivot] = col;
                        break;
                    }
                    col ^= basis[pivot];
                }
            }
            basis.into_iter().filter(|col| *col != 0).count()
        }
        for seed in 1..=12_i32 {
            let points: Vec<_> = (0..8_i32)
                .map(|i| {
                    vec![
                        f64::from((seed * i + 3 * i * i) % 9),
                        f64::from((seed * i * i + i) % 7),
                    ]
                })
                .collect();
            let bars = persistence(&points).unwrap();
            for step in 0..20 {
                let distance = f64::from(step) / 2.;
                let mut edges = Vec::new();
                let mut boundary = [[0_u64; 8]; 8];
                let mut triangles = Vec::new();
                for (i, p) in points.iter().enumerate() {
                    for (j, q) in points[..i].iter().enumerate() {
                        if (p[0] - q[0]).hypot(p[1] - q[1]) <= distance {
                            boundary[i][j] = 1 << edges.len();
                            edges.push((1 << i) | (1 << j));
                            for k in 0..j {
                                if boundary[i][k] != 0 && boundary[j][k] != 0 {
                                    triangles
                                        .push(boundary[i][j] | boundary[i][k] | boundary[j][k]);
                                }
                            }
                        }
                    }
                }
                let count = edges.len();
                let edge_rank = rank(edges);
                assert_eq!(
                    bars.at(distance),
                    (8 - edge_rank, count - edge_rank - rank(triangles)),
                    "seed {seed}, distance {distance}"
                );
            }
        }
    }
}

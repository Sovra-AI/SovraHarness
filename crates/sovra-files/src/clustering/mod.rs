//! UMAP adapter and bounded global/local RAPTOR grouping on one CPU worker.

mod reduction;
#[cfg(test)]
mod tests;

use sovra_core::file_index::{
    IndexError,
    clustering::{
        ClusterDiagnostics, ClusterInput, ClusterObserver, ClusterPlan, RAPTOR_CLUSTERING_REVISION,
        RaptorOptions, SummaryClusterer, select_mixture,
    },
};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct RaptorClusterer {
    pool: rayon::ThreadPool,
}

impl RaptorClusterer {
    pub fn new() -> Result<Self, IndexError> {
        // UMAP uses a parallel optimizer internally. Keep it within one owned
        // worker so it cannot consume the host's global Rayon pool.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .thread_name(|_| "sovra-clustering".into())
            .build()
            .map_err(|_| IndexError("Could not start the summary clustering worker.".into()))?;
        Ok(Self { pool })
    }
}

struct Budget<'a> {
    observer: &'a dyn ClusterObserver,
    started: Instant,
}
impl ClusterObserver for Budget<'_> {
    fn cancelled(&self) -> bool {
        self.observer.cancelled() || self.started.elapsed() > Duration::from_secs(20)
    }
}
impl Budget<'_> {
    fn check(&self) -> Result<(), IndexError> {
        if self.cancelled() {
            Err(IndexError("Clustering was cancelled or exceeded its 20-second computation budget. Reduce the build scope.".into()))
        } else {
            Ok(())
        }
    }
}

struct Worker<'a> {
    inputs: &'a [ClusterInput],
    options: &'a RaptorOptions,
    budget: Budget<'a>,
    diagnostics: ClusterDiagnostics,
    stages: usize,
}

impl SummaryClusterer for RaptorClusterer {
    fn cluster(
        &self,
        inputs: &[ClusterInput],
        options: &RaptorOptions,
        observer: &dyn ClusterObserver,
    ) -> Result<ClusterPlan, IndexError> {
        options.validate_inputs(inputs)?;
        self.pool.install(|| {
            let mut worker = Worker {
                inputs,
                options,
                budget: Budget {
                    observer,
                    started: Instant::now(),
                },
                diagnostics: ClusterDiagnostics::default(),
                stages: 0,
            };
            let mut groups = BTreeSet::new();
            worker.partition((0..inputs.len()).collect(), 0, &mut groups)?;
            worker.budget.check()?;
            if (0..inputs.len()).any(|input| !groups.iter().any(|g| g.contains(&input))) {
                return Err(IndexError(
                    "Clustering did not cover every source input. No map was generated.".into(),
                ));
            }
            Ok(ClusterPlan {
                groups: groups.into_iter().collect(),
                diagnostics: worker.diagnostics,
                algorithm_revision: RAPTOR_CLUSTERING_REVISION.into(),
            })
        })
    }
}

impl Worker<'_> {
    fn statistical(&mut self, ids: &[usize], global: bool) -> Result<Vec<Vec<usize>>, IndexError> {
        self.budget.check()?;
        if ids.len() <= 2
            || ids
                .iter()
                .all(|id| self.inputs[*id].embedding == self.inputs[ids[0]].embedding)
        {
            return Ok(vec![ids.to_vec()]);
        }
        self.stages += 1;
        if self.stages > 256 {
            return Err(IndexError(
                "This clustering stage exceeded its 256-reduction budget. Narrow the map scope."
                    .into(),
            ));
        }
        let points = reduction::reduce(self.inputs, ids, self.options, global, &self.budget)?;
        self.diagnostics.reductions += 1;
        let mixture = select_mixture(&points, self.options, &self.budget)?;
        self.diagnostics.mixture_fits += mixture.attempted;
        self.diagnostics.failed_fits += mixture.failed;
        Ok(mixture
            .groups
            .into_iter()
            .map(|group| group.into_iter().map(|i| ids[i]).collect())
            .collect())
    }

    fn hierarchy(&mut self, ids: &[usize]) -> Result<Vec<Vec<usize>>, IndexError> {
        let mut groups = BTreeSet::new();
        for global in self.statistical(ids, true)? {
            for mut local in self.statistical(&global, false)? {
                local.sort_unstable();
                groups.insert(local);
            }
        }
        Ok(groups.into_iter().collect())
    }

    fn fits(&self, ids: &[usize]) -> bool {
        ids.len() <= self.options.max_group_inputs
            && ids
                .iter()
                .map(|i| self.inputs[*i].text_bytes)
                .sum::<usize>()
                <= self.options.max_group_bytes
    }

    fn partition(
        &mut self,
        ids: Vec<usize>,
        depth: u8,
        groups: &mut BTreeSet<Vec<usize>>,
    ) -> Result<(), IndexError> {
        self.budget.check()?;
        // Always apply global/local clustering at the entry stage, even if its
        // text fits one model request: semantic grouping is the purpose of RAPTOR.
        if depth > 0 && self.fits(&ids) {
            groups.insert(ids);
            return Ok(());
        }
        let candidates = if depth < 6 {
            self.hierarchy(&ids)?
        } else {
            vec![ids.clone()]
        };
        for candidate in candidates {
            if self.fits(&candidate) {
                groups.insert(candidate);
            } else if candidate.len() < ids.len() {
                self.partition(candidate, depth + 1, groups)?;
            } else {
                // A degenerate or overlapping mixture may reproduce its parent.
                // Report a semantic resource partition instead of recursing forever.
                self.diagnostics.budget_splits += 1;
                let (left, right) = bisect(self.inputs, &candidate);
                self.partition(left, depth + 1, groups)?;
                self.partition(right, depth + 1, groups)?;
            }
            if groups.len() > 2048 {
                return Err(IndexError(
                    "Overlapping clusters exceeded the 2,048-group budget. Narrow the map scope."
                        .into(),
                ));
            }
        }
        Ok(())
    }
}

fn squared_distance(a: &ClusterInput, b: &ClusterInput) -> f64 {
    a.embedding
        .values()
        .iter()
        .zip(b.embedding.values())
        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
        .sum()
}

fn bisect(inputs: &[ClusterInput], ids: &[usize]) -> (Vec<usize>, Vec<usize>) {
    let farthest = |anchor: usize| {
        *ids.iter()
            .max_by(|a, b| {
                squared_distance(&inputs[anchor], &inputs[**a])
                    .total_cmp(&squared_distance(&inputs[anchor], &inputs[**b]))
                    .then_with(|| b.cmp(a))
            })
            .expect("nonempty source group")
    };
    let left = farthest(ids[0]);
    let right = farthest(left);
    let mut ordered = ids.to_vec();
    ordered.sort_by(|a, b| {
        let score = |id: usize| {
            squared_distance(&inputs[id], &inputs[left])
                - squared_distance(&inputs[id], &inputs[right])
        };
        score(*a).total_cmp(&score(*b)).then_with(|| a.cmp(b))
    });
    let mut right = ordered.split_off(ordered.len() / 2);
    ordered.sort_unstable();
    right.sort_unstable();
    (ordered, right)
}

//! Coalesce notification bursts and bound rescan frequency without trusting an
//! OS event stream to be complete. Times are monotonic milliseconds supplied by runtime.

const QUIET_MS: u64 = 1_500;
const MAX_DELAY_MS: u64 = 10_000;
const COOLDOWN_MS: u64 = 30_000;
const RECONCILE_MS: u64 = 15 * 60 * 1_000;

#[derive(Debug, Clone)]
pub struct RefreshSchedule {
    first_change: Option<u64>,
    last_change: Option<u64>,
    last_scan: Option<u64>,
    reconcile_at: u64,
}

impl RefreshSchedule {
    pub fn new(now: u64, reconcile_on_start: bool) -> Self {
        Self {
            first_change: None,
            last_change: None,
            last_scan: None,
            reconcile_at: if reconcile_on_start {
                now
            } else {
                now.saturating_add(RECONCILE_MS)
            },
        }
    }

    pub fn changed(&mut self, now: u64) {
        self.first_change.get_or_insert(now);
        self.last_change = Some(now);
    }

    pub fn due(&self, now: u64) -> bool {
        if self
            .last_scan
            .is_some_and(|last| now.saturating_sub(last) < COOLDOWN_MS)
        {
            return false;
        }
        now >= self.reconcile_at
            || self
                .first_change
                .zip(self.last_change)
                .is_some_and(|(first, last)| {
                    now.saturating_sub(last) >= QUIET_MS
                        || now.saturating_sub(first) >= MAX_DELAY_MS
                })
    }

    /// Consume pending work when a scan starts, so notifications arriving during
    /// it remain pending for a subsequent scan.
    pub fn started(&mut self, now: u64) {
        self.first_change = None;
        self.last_change = None;
        self.last_scan = Some(now);
        self.reconcile_at = now.saturating_add(RECONCILE_MS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bursts_wait_for_quiet_but_continuous_changes_and_dropped_events_cannot_starve() {
        let mut schedule = RefreshSchedule::new(0, false);
        schedule.changed(100);
        schedule.changed(1_000);
        assert!(!schedule.due(2_499));
        assert!(schedule.due(2_500));
        schedule.started(2_500);
        schedule.changed(3_000);
        assert!(!schedule.due(32_499));
        assert!(schedule.due(32_500));
        schedule.started(32_500);
        for now in 100_000..110_000 {
            schedule.changed(now);
        }
        assert!(!schedule.due(109_999));
        assert!(schedule.due(110_000));
        schedule.started(110_000);
        assert!(!schedule.due(110_001));
        assert!(schedule.due(110_000 + RECONCILE_MS));
        assert!(RefreshSchedule::new(42, true).due(42));
    }
}

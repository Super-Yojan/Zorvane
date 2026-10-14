//! Typed, source-driven gating for expensive latest-value telemetry.
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum Publication {
    Full,
    Heartbeat,
    None,
}

pub struct Gate<T> {
    baseline: Option<(String, T, Duration)>,
    observed: Option<(String, u64)>,
    heartbeat: Option<Duration>,
}

impl<T> Default for Gate<T> {
    fn default() -> Self {
        Self {
            baseline: None,
            observed: None,
            heartbeat: None,
        }
    }
}

impl<T: Clone> Gate<T> {
    pub fn observe(
        &mut self,
        identity: &str,
        content: &T,
        sequence: u64,
        fresh: bool,
        now: Duration,
        equal: impl Fn(&T, &T) -> bool,
    ) -> Publication {
        // A run/sequence identifies immutable source content. Ignore reordered
        // samples and conflicting duplicates rather than publishing revisions
        // that receivers correctly reject.
        if self.observed.as_ref().is_some_and(|(id, previous)| {
            id == identity && sequence <= *previous
        }) {
            return Publication::None;
        }
        self.observed = Some((identity.into(), sequence));
        // Full payloads also confirm freshness at receivers. Never let cached
        // snapshots, including initial/reconnect snapshots, renew sensor life.
        if !fresh {
            return Publication::None;
        }
        if self.baseline.as_ref().is_none_or(|(id, old, at)| {
            id != identity
                || !equal(old, content)
                || now.saturating_sub(*at) >= Duration::from_secs(2)
        }) {
            self.baseline = Some((identity.into(), content.clone(), now));
            self.heartbeat = Some(now);
            return Publication::Full;
        }
        if self
                .heartbeat
                .is_none_or(|at| now.saturating_sub(at) >= Duration::from_millis(200))
        {
            self.heartbeat = Some(now);
            Publication::Heartbeat
        } else {
            Publication::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transitions_recovery_and_source_liveness() {
        let mut gate = Gate::default();
        let cells = vec![-1, 0, 100];
        let mut observe = |run: &str, cells: &Vec<i8>, seq, fresh, ms| {
            gate.observe(run, cells, seq, fresh, Duration::from_millis(ms), |a, b| {
                a == b
            })
        };
        assert_eq!(observe("a", &cells, 1, true, 0), Publication::Full);
        assert_eq!(observe("a", &cells, 1, true, 200), Publication::None);
        assert_eq!(observe("a", &cells, 2, false, 400), Publication::None);
        assert_eq!(observe("a", &cells, 3, true, 600), Publication::Heartbeat);
        assert_eq!(observe("a", &cells, 4, true, 700), Publication::None);
        assert_eq!(
            observe("a", &vec![-1, 1, 100], 4, true, 701),
            Publication::None
        );
        assert_eq!(observe("b", &cells, 4, true, 702), Publication::Full);
        assert_eq!(observe("b", &cells, 4, false, 2702), Publication::None);
        assert_eq!(observe("b", &cells, 5, true, 2802), Publication::Full);
    }

    #[test]
    fn geometry_identity_and_reconnect_force_baselines() {
        let mut gate = Gate::default();
        let mut content = (200, 200, 0.1_f64, 0.0_f64, vec![0_i8; 4]);
        let now = Duration::ZERO;
        assert_eq!(
            gate.observe("run/frame", &content, 1, true, now, |a, b| a == b),
            Publication::Full
        );
        content.3 = 0.1;
        assert_eq!(
            gate.observe("run/frame", &content, 1, true, now, |a, b| a == b),
            Publication::None
        );
        assert_eq!(
            gate.observe("run/frame", &content, 2, true, now, |a, b| a == b),
            Publication::Full
        );
        assert_eq!(
            gate.observe("run/new-frame", &content, 1, true, now, |a, b| a == b),
            Publication::Full
        );
        gate = Gate::default();
        assert_eq!(
            gate.observe("run/new-frame", &content, 1, false, now, |a, b| a == b),
            Publication::None
        );
        assert_eq!(
            gate.observe("run/new-frame", &content, 2, true, now, |a, b| a == b),
            Publication::Full
        );
    }

    #[test]
    fn frozen_sensor_never_renews_liveness_even_after_recovery_intervals() {
        let mut gate = Gate::default();
        let cells = vec![0_i8, 100];
        assert_eq!(
            gate.observe("run", &cells, 1, true, Duration::ZERO, |a, b| a == b),
            Publication::Full
        );
        for ms in [2000, 4000, 6000, 10000] {
            // Even a caller claiming freshness cannot turn a repeated sample
            // into a timer-driven recovery publication.
            for fresh in [true, false] {
                assert_eq!(
                    gate.observe("run", &cells, 1, fresh, Duration::from_millis(ms), |a, b| a == b),
                    Publication::None
                );
            }
        }
        assert_eq!(
            gate.observe("new-run", &vec![100, 100], 1, false, Duration::from_secs(11), |a, b| a == b),
            Publication::None
        );
        assert_eq!(
            gate.observe("new-run", &cells, 2, true, Duration::from_secs(12), |a, b| a == b),
            Publication::Full
        );
    }

    #[test]
    fn reordered_and_conflicting_sequences_are_ignored_until_run_reset() {
        let mut gate = Gate::default();
        let cells = vec![0_i8];
        let changed = vec![100_i8];
        let now = Duration::from_secs(3);
        assert_eq!(gate.observe("run", &cells, 10, true, Duration::ZERO, |a,b| a == b), Publication::Full);
        for sequence in [9, 10, 8] {
            assert_eq!(gate.observe("run", &changed, sequence, true, now, |a,b| a == b), Publication::None);
        }
        assert_eq!(gate.observe("run", &changed, 11, true, now, |a,b| a == b), Publication::Full);
        assert_eq!(gate.observe("next-run", &cells, 1, true, now, |a,b| a == b), Publication::Full);
    }
}

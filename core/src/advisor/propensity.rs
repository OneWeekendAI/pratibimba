//! The propensity of a decision: how likely the policy, as it stood, was to serve the arm it
//! served. A Thompson draw has no closed form for that, so it is estimated by replaying the row's
//! own model version [`DRAWS`] times with seeds `base_seed ^ i` and counting how often the served
//! arm wins. Rows that are not a policy draw get a kind instead of a replay (see [`kind_of`]).

use crate::bandit::LinTs;
use serde_json::Value;

/// Replays per row: the share's standard error is at most 0.5 / sqrt(2000) ~ 0.011.
pub const DRAWS: u32 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Replay,
    Single,
    Holdout,
    Experiment,
    None,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Replay => "replay",
            Kind::Single => "single",
            Kind::Holdout => "holdout",
            Kind::Experiment => "experiment",
            Kind::None => "none",
        }
    }
}

/// Which propensity a row has. A held-out row got the baseline by hash; an on-device or
/// cold-start row was not a seeded draw (round-robin or the client's own); an experiment row was
/// a weighted hash; a one-arm pool had no choice; everything else replays.
pub fn kind_of(policy: &str, allocation: &str, in_holdout: bool, cold_start: bool, pool_len: usize) -> Kind {
    if in_holdout {
        Kind::Holdout
    } else if policy == crate::adapt::POLICY || cold_start || pool_len == 0 {
        Kind::None
    } else if allocation == "experiment" {
        Kind::Experiment
    } else if pool_len == 1 {
        Kind::Single
    } else {
        Kind::Replay
    }
}

/// The share of [`DRAWS`] replays of `model` that pick `chosen` from `pool`, floored at one draw
/// so a weight `1 / p` stays finite; `None` when `features` do not name the bandit's vector.
pub fn of_row(model: &LinTs, policy: &str, features: &Value, pool: &[i64], chosen: i64, base_seed: u64) -> Option<f64> {
    let x = crate::http::decide::from_named(features)?;
    let wins = (0..DRAWS)
        .filter(|i| crate::explain::replay(model, policy, features, &x, pool, base_seed ^ u64::from(*i)).0 == chosen)
        .count();
    Some((wins as f64 / f64::from(DRAWS)).max(1.0 / f64::from(DRAWS)))
}

/// An experiment row's propensity: the arm's weight among the experiment's arms still in the
/// row's pool (the hash reweights over the open arms); `None` if the arm is not among them.
pub fn experiment_weight(arms: &[(i64, f64)], pool: &[i64], chosen: i64) -> Option<f64> {
    let open: Vec<&(i64, f64)> = arms.iter().filter(|a| pool.contains(&a.0) && a.1 > 0.0).collect();
    let total: f64 = open.iter().map(|a| a.1).sum();
    let w = open.iter().find(|a| a.0 == chosen)?.1;
    (total > 0.0).then(|| (w / total).min(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feats() -> Value {
        json!({ "bias": 1.0, "returning": 0.0, "mobile": 1.0, "from_ad": 0.0 })
    }

    /// Arm `good` converts every time and `bad` never, 300 lessons each.
    fn dominant(good: i64, bad: i64) -> LinTs {
        let mut m = LinTs::default();
        let x = [1.0, 0.0, 1.0, 0.0];
        for _ in 0..300 {
            m.update(good, &x, true);
            m.update(bad, &x, false);
        }
        m
    }

    // Mutation: count wins with seed `base_seed` only (no `^ i`) -> red (0 or 1, not ~0.5).
    #[test]
    fn equal_arms_near_half() {
        let p = of_row(&LinTs::default(), crate::bandit::POLICY, &feats(), &[1, 2], 1, 42).unwrap();
        assert!((p - 0.5).abs() <= 0.05, "{p}");
    }

    // Mutation: count wins of `pool[0]` instead of `chosen` -> red (the dominant arm is second).
    #[test]
    fn dominant_arm_near_one() {
        let p = of_row(&dominant(7, 3), crate::bandit::POLICY, &feats(), &[3, 7], 7, 9).unwrap();
        assert!(p > 0.95, "{p}");
    }

    // Mutation: return Kind::Replay for a one-arm pool -> red.
    #[test]
    fn single_arm_pool_is_one() {
        assert_eq!(kind_of(crate::bandit::POLICY, "bandit", false, false, 1), Kind::Single);
    }

    // Mutation: drop the adapt::POLICY test from kind_of -> red.
    #[test]
    fn adapt_policy_is_none() {
        assert_eq!(kind_of(crate::adapt::POLICY, "bandit", false, false, 3), Kind::None);
    }

    // Mutation: drop the `.max(1 / DRAWS)` floor -> red (the losing arm wins no replay).
    #[test]
    fn floor_never_zero() {
        let p = of_row(&dominant(7, 3), crate::bandit::POLICY, &feats(), &[3, 7], 3, 9).unwrap();
        assert_eq!(p, 1.0 / DRAWS as f64);
    }

    // Mutation: check holdout after single (a held row in a one-arm pool reads single) -> red.
    #[test]
    fn holdout_beats_single() {
        assert_eq!(kind_of(crate::bandit::POLICY, "bandit", true, false, 1), Kind::Holdout);
    }

    // Mutation: drop the cold-start test from kind_of -> red.
    #[test]
    fn cold_start_is_none() {
        assert_eq!(kind_of(crate::bandit::POLICY, "bandit", false, true, 3), Kind::None);
    }

    // Mutation: drop the experiment arm of kind_of -> red.
    #[test]
    fn experiment_allocation_is_experiment() {
        assert_eq!(kind_of(crate::bandit::POLICY, "experiment", false, false, 3), Kind::Experiment);
    }

    // Mutation: divide by the sum of every experiment arm, not only those in the pool -> red.
    #[test]
    fn experiment_weight_renormalises_over_open_arms() {
        // Arm 3 is closed to this visitor (not in pool): 1 of (1 + 3), not 1 of 6.
        let w = experiment_weight(&[(1, 1.0), (2, 3.0), (3, 2.0)], &[1, 2], 1).unwrap();
        assert!((w - 0.25).abs() < 1e-9, "{w}");
    }

    // Mutation: drop the `?` on from_named (treat unreadable features as zeros) -> red.
    #[test]
    fn unreadable_features_give_none() {
        assert_eq!(of_row(&LinTs::default(), crate::bandit::POLICY, &json!({ "bias": 1.0 }), &[1, 2], 1, 1), None);
    }
}

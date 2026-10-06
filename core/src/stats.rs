//! Always-valid confidence sequences for conversion rates and lift.
//!
//! A fixed-n confidence interval is only honest if you look once, at a sample size you chose in
//! advance. A dashboard is looked at all the time, so its interval must hold at every n at
//! once. This module uses the asymptotic confidence sequence built on Robbins' normal mixture
//! (Waudby-Smith, Arnold, Wu, Wang and Ramdas, "Time-uniform central limit theory and
//! asymptotic confidence sequences", 2023):
//!
//! ```text
//!   mean ± sqrt( 2 (n σ̂² ρ² + 1) / (n² ρ²) · ln( sqrt(n σ̂² ρ² + 1) / α ) )
//! ```
//!
//! with `σ̂² = max(p̂(1 − p̂), VAR_FLOOR)`. `ρ` only decides where the sequence is tightest; any
//! fixed `ρ > 0` keeps it valid. Lift is treatment minus control in absolute rate (so 0.012 is
//! 1.2 percentage points). Each group gets a sequence at `α/2`, and the lift interval is their
//! Minkowski difference, which by a union bound covers the true lift with probability at least
//! `1 − α` at every n simultaneously.

use serde::Serialize;

/// The dashboard's error rate: 95% sequences.
pub const ALPHA: f64 = 0.05;
/// Keeps the half-width from collapsing at p̂ = 0 or 1 when n is small.
pub const VAR_FLOOR: f64 = 1e-4;
/// The intrinsic time (n · σ²) where the sequence is tightest: about 2,000 visitors at a 5%
/// conversion rate, a typical slot's first few days.
const TIGHTEST_AT: f64 = 100.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Interval {
    pub estimate: f64,
    pub lo: f64,
    pub hi: f64,
}

impl Interval {
    pub fn excludes_zero(&self) -> bool {
        self.lo > 0.0 || self.hi < 0.0
    }
}

fn rho_squared(alpha: f64) -> f64 {
    let l = -2.0 * alpha.ln();
    (l + (l + 1.0).ln()) / TIGHTEST_AT
}

/// Half-width of the sequence for a mean of `n` observations with variance `var`.
pub fn half_width(n: u64, var: f64, alpha: f64) -> f64 {
    let t = n as f64;
    let r2 = rho_squared(alpha);
    let v = t * var * r2 + 1.0;
    (2.0 * v / (t * t * r2) * (v.sqrt() / alpha).ln()).sqrt()
}

/// The sequence for a conversion rate: `successes` out of `n`. `None` with no data.
pub fn rate_cs(successes: u64, n: u64, alpha: f64) -> Option<Interval> {
    if n == 0 {
        return None;
    }
    let p = successes as f64 / n as f64;
    let h = half_width(n, (p * (1.0 - p)).max(VAR_FLOOR), alpha);
    Some(Interval { estimate: p, lo: (p - h).max(0.0), hi: (p + h).min(1.0) })
}

/// The sequence for `treated rate − control rate`, each group given as `(successes, n)`.
pub fn lift_cs(treated: (u64, u64), control: (u64, u64), alpha: f64) -> Option<Interval> {
    let t = rate_cs(treated.0, treated.1, alpha / 2.0)?;
    let c = rate_cs(control.0, control.1, alpha / 2.0)?;
    Some(Interval { estimate: t.estimate - c.estimate, lo: t.lo - c.hi, hi: t.hi - c.lo })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn cs_contains_the_estimate() {
        let i = rate_cs(30, 500, ALPHA).unwrap();
        assert!(i.lo < i.estimate && i.estimate < i.hi, "{i:?}");
        assert!((i.estimate - 0.06).abs() < 1e-12);
    }

    #[test]
    fn cs_narrows_as_n_grows() {
        let small = rate_cs(100, 1_000, ALPHA).unwrap();
        let big = rate_cs(1_000, 10_000, ALPHA).unwrap();
        assert!(big.hi - big.lo < 0.5 * (small.hi - small.lo), "{small:?} vs {big:?}");
    }

    #[test]
    fn cs_is_wider_than_a_fixed_n_interval() {
        // The price of peeking: wider than the 95% interval you could use if you looked once.
        let (n, p) = (1_000u64, 0.1);
        let fixed = 1.96 * (p * (1.0 - p) / n as f64).sqrt();
        let i = rate_cs(100, n, ALPHA).unwrap();
        assert!((i.hi - i.lo) / 2.0 > 1.2 * fixed, "half {} vs fixed {fixed}", (i.hi - i.lo) / 2.0);
    }

    #[test]
    fn no_data_gives_no_interval() {
        assert!(rate_cs(0, 0, ALPHA).is_none());
        assert!(lift_cs((1, 10), (0, 0), ALPHA).is_none());
    }

    /// One A/A run with a peek every `every` visitors per group: did the lift interval ever
    /// exclude zero?
    fn ever_rejects(rng: &mut ChaCha8Rng, p: f64, n: u64, every: u64) -> bool {
        let (mut st, mut sc) = (0u64, 0u64);
        for i in 1..=n {
            st += (rng.random::<f64>() < p) as u64;
            sc += (rng.random::<f64>() < p) as u64;
            if i % every == 0 && lift_cs((st, i), (sc, i), ALPHA).unwrap().excludes_zero() {
                return true;
            }
        }
        false
    }

    #[test]
    fn peeking_under_the_null_stays_below_alpha() {
        let mut rng = ChaCha8Rng::seed_from_u64(20261006);
        let runs = 300;
        let rejected = (0..runs).filter(|_| ever_rejects(&mut rng, 0.05, 4_000, 25)).count();
        assert!(rejected as f64 / runs as f64 <= ALPHA, "false positives under continuous peeking: {rejected}/{runs}");
    }

    fn simulate(seed: u64, p_t: f64, p_c: f64, n: u64) -> Interval {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let st = (0..n).filter(|_| rng.random::<f64>() < p_t).count() as u64;
        let sc = (0..n).filter(|_| rng.random::<f64>() < p_c).count() as u64;
        lift_cs((st, n), (sc, n), ALPHA).unwrap()
    }

    #[test]
    fn a_real_lift_is_detected() {
        let i = simulate(1, 0.08, 0.05, 8_000);
        assert!(i.lo > 0.0, "a 3 pp lift over 8,000 per group was not detected: {i:?}");
    }

    #[test]
    fn a_real_loss_is_detected() {
        let i = simulate(2, 0.05, 0.08, 8_000);
        assert!(i.hi < 0.0, "a 3 pp loss over 8,000 per group was not detected: {i:?}");
    }
}

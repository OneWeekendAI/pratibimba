//! A1: the doubly robust uplift of one arm against the slot's baseline.
//!
//! Only rows where both arms were open to the visitor (`arm` and `baseline` in `pool`) count, so
//! an arm approved last week is compared on last week's visitors only. The direct model is each
//! arm's mean reward, cross-fitted over two folds split by a hash of the row's position: a row is
//! scored with the means of the *other* fold. Per row, for arm `a` with stored propensity `p`
//! (the probability the policy gave the arm it served):
//!
//! `s_a = m_a + 1{A = a} (Y - m_a) / p`
//!
//! and the same for the baseline. The uplift is `mean(s_a - s_b)` per 100 visits, its interval
//! `mean +/- 1.96 sd / sqrt(n)`, and `p` the two-sided normal p-value of a difference from zero.
//! A constant direct model makes this the inverse-propensity estimate with the arm mean as a
//! control variate: unbiased whenever the propensities are right, and lower variance than plain
//! weighting. The covariate-aware direct model is A2's tree.

use chrono::{DateTime, Utc};

/// One settled, exposed decision with a recomputed propensity.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The variant served.
    pub arm: i64,
    /// The arms open to the visitor (after share and audience gating).
    pub pool: Vec<i64>,
    /// The probability the policy gave `arm`.
    pub propensity: f64,
    /// The conversion credit: `credit` (1 under last touch) when converted, else 0.
    pub reward: f64,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    pub arm: i64,
    /// Rows that served `arm` (both arms open).
    pub n_arm: u64,
    /// Rows that served the baseline (both arms open).
    pub n_base: u64,
    /// Every row compared (both arms open, whatever was served).
    pub n: u64,
    /// Per 100 visits, against the baseline.
    pub uplift: f64,
    /// Standard error of `uplift`, per 100.
    pub se: f64,
    pub lo: f64,
    pub hi: f64,
    /// Two-sided p-value of a difference from zero.
    pub p: f64,
}

const Z95: f64 = 1.959_963_984_540_054;

/// The standard normal CDF (Abramowitz and Stegun 7.1.26 via erfc; error below 1.5e-7).
pub fn phi(z: f64) -> f64 {
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t * (0.254_829_592 + t * (-0.284_496_736 + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erfc = poly * (-x * x).exp();
    if z >= 0.0 { 1.0 - 0.5 * erfc } else { 0.5 * erfc }
}

/// Which of the two folds a row's position falls in (a splitmix64 bit, so interleaved rows of
/// one arm do not all land in one fold).
fn fold(i: usize) -> usize {
    let mut z = (i as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) & 1) as usize
}

/// The doubly robust uplift of `arm` over `baseline`; `None` when either arm served fewer than
/// `min_rows` of the compared rows.
pub fn dr(rows: &[Row], arm: i64, baseline: i64, min_rows: u64) -> Option<Estimate> {
    let both: Vec<(usize, &Row)> = rows.iter().enumerate().filter(|(_, r)| r.pool.contains(&arm) && r.pool.contains(&baseline)).collect();
    let n_arm = both.iter().filter(|(_, r)| r.arm == arm).count() as u64;
    let n_base = both.iter().filter(|(_, r)| r.arm == baseline).count() as u64;
    if n_arm < min_rows || n_base < min_rows || n_arm == 0 || n_base == 0 {
        return None;
    }
    // Per fold, per arm (0 = arm, 1 = baseline): reward sum and count.
    let mut sums = [[(0.0f64, 0u64); 2]; 2];
    for (i, r) in &both {
        let k = if r.arm == arm { 0 } else if r.arm == baseline { 1 } else { continue };
        let s = &mut sums[fold(*i)][k];
        s.0 += r.reward;
        s.1 += 1;
    }
    let mean = |f: usize, k: usize| {
        let (o, m) = (sums[1 - f][k], sums[f][k]);
        if o.1 > 0 { o.0 / o.1 as f64 } else { (o.0 + m.0) / (o.1 + m.1).max(1) as f64 }
    };
    let diffs: Vec<f64> = both
        .iter()
        .map(|(i, r)| {
            let f = fold(*i);
            let (ma, mb) = (mean(f, 0), mean(f, 1));
            let sa = ma + if r.arm == arm { (r.reward - ma) / r.propensity } else { 0.0 };
            let sb = mb + if r.arm == baseline { (r.reward - mb) / r.propensity } else { 0.0 };
            sa - sb
        })
        .collect();
    let n = diffs.len() as f64;
    let m = diffs.iter().sum::<f64>() / n;
    let var = diffs.iter().map(|d| (d - m).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    let se = (var / n).sqrt() * 100.0;
    let uplift = m * 100.0;
    let p = if se > 0.0 { 2.0 * (1.0 - phi((uplift / se).abs())) } else if uplift == 0.0 { 1.0 } else { 0.0 };
    Some(Estimate { arm, n_arm, n_base, n: diffs.len() as u64, uplift, se, lo: uplift - Z95 * se, hi: uplift + Z95 * se, p })
}

/// The naive difference of observed means, per 100 (the comparison `dr` corrects).
pub fn naive(rows: &[Row], arm: i64, baseline: i64) -> Option<f64> {
    let mean = |a: i64| {
        let r: Vec<f64> = rows.iter().filter(|r| r.arm == a && r.pool.contains(&arm) && r.pool.contains(&baseline)).map(|r| r.reward).collect();
        (!r.is_empty()).then(|| r.iter().sum::<f64>() / r.len() as f64)
    };
    Some((mean(arm)? - mean(baseline)?) * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;

    const A: i64 = 2;
    const B: i64 = 1;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_760_000_000, 0).unwrap()
    }

    /// `n` rows; the arm is served with probability `p_arm(x)`, and converts at `rate(x, arm)`,
    /// for a covariate `x` that is 1 half the time.
    fn simulate(n: usize, seed: u64, p_arm: impl Fn(bool) -> f64, rate: impl Fn(bool, bool) -> f64) -> Vec<Row> {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        (0..n)
            .map(|_| {
                let x = rng.random_bool(0.5);
                let pa = p_arm(x);
                let served_arm = rng.random_bool(pa);
                let y = rng.random_bool(rate(x, served_arm));
                Row {
                    arm: if served_arm { A } else { B },
                    pool: vec![B, A],
                    propensity: if served_arm { pa } else { 1.0 - pa },
                    reward: if y { 1.0 } else { 0.0 },
                    at: at(),
                }
            })
            .collect()
    }

    // Mutation: drop the `* 100.0` scaling of the uplift -> red.
    #[test]
    fn planted_uplift_recovered() {
        let rows = simulate(20_000, 7, |_| 0.3, |_, arm| if arm { 0.13 } else { 0.10 });
        let e = dr(&rows, A, B, 300).unwrap();
        assert!((e.uplift - 3.0).abs() < 1.0, "{e:?}");
        assert!(e.lo < 3.0 && 3.0 < e.hi, "{e:?}");
        assert_eq!(e.n, 20_000);
    }

    // Mutation: drop the `/p` weighting -> red.
    #[test]
    fn confounded_naive_mean_is_wrong_dr_is_right() {
        // Visitors with x = 1 convert more AND are served the arm more often.
        let rows = simulate(20_000, 11, |x| if x { 0.7 } else { 0.3 }, |x, arm| (if x { 0.20 } else { 0.05 }) + if arm { 0.03 } else { 0.0 });
        let naive = naive(&rows, A, B).unwrap();
        let e = dr(&rows, A, B, 300).unwrap();
        assert!((naive - 3.0).abs() > 2.0, "the naive difference should be confounded: {naive}");
        assert!((e.uplift - 3.0).abs() < 1.0, "{e:?}");
    }

    // Mutation: drop the `pool.contains` filter -> red.
    #[test]
    fn arm_absent_from_pool_rows_excluded() {
        let mut rows = simulate(4_000, 3, |_| 0.5, |_, _| 0.10);
        let before = dr(&rows, A, B, 300).unwrap();
        // Before the arm was approved: baseline only, and a busier season (every visitor converts).
        rows.extend((0..2_000).map(|_| Row { arm: B, pool: vec![B], propensity: 1.0, reward: 1.0, at: at() }));
        let after = dr(&rows, A, B, 300).unwrap();
        assert_eq!(after.n, 4_000);
        assert!((after.uplift - before.uplift).abs() < 1e-9, "{before:?} vs {after:?}");
    }

    // Mutation: drop the `n_arm < min_rows` check -> red.
    #[test]
    fn below_minimum_is_none() {
        let mut rows: Vec<Row> = (0..400).map(|_| Row { arm: B, pool: vec![B, A], propensity: 0.5, reward: 0.0, at: at() }).collect();
        rows.extend((0..299).map(|_| Row { arm: A, pool: vec![B, A], propensity: 0.5, reward: 0.0, at: at() }));
        assert_eq!(dr(&rows, A, B, 300), None);
    }

    // Mutation: compare against the constant 300 instead of `min_rows` -> red.
    #[test]
    fn min_rows_setting_moves_the_floor() {
        let rows = simulate(400, 5, |_| 0.5, |_, _| 0.10);
        assert!(dr(&rows, A, B, 300).is_none(), "about 200 rows a side is under the default");
        assert!(dr(&rows, A, B, 100).is_some());
    }

    // Mutation: a one-sided p (drop the factor 2) -> red.
    #[test]
    fn p_value_is_two_sided() {
        let e = Estimate { arm: A, n_arm: 1, n_base: 1, n: 2, uplift: 1.959_963_984_540_054, se: 1.0, lo: 0.0, hi: 0.0, p: 0.0 };
        let p = 2.0 * (1.0 - phi((e.uplift / e.se).abs()));
        assert!((p - 0.05).abs() < 1e-6, "{p}");
        let rows = simulate(4_000, 9, |_| 0.5, |_, _| 0.10);
        let e = dr(&rows, A, B, 300).unwrap();
        let z = e.uplift / e.se;
        assert!((e.p - 2.0 * (1.0 - phi(z.abs()))).abs() < 1e-12, "{e:?}");
        assert!(e.p > 0.0 && e.p <= 1.0);
    }

    // Mutation: phi returns 1 - phi for negative z flipped (sign error) -> red.
    #[test]
    fn phi_matches_known_values() {
        assert!((phi(0.0) - 0.5).abs() < 1e-7);
        assert!((phi(1.959_963_984_540_054) - 0.975).abs() < 1e-6);
        assert!((phi(-1.0) - 0.158_655_253_9).abs() < 1e-6);
    }
}

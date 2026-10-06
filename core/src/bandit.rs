//! LinTS: Thompson sampling over a Bayesian logistic regression per arm.
//!
//! Each arm holds a Gaussian posterior `N(mean, cov)` over the weights of a logistic model
//! `P(convert | x) = sigmoid(w . x)`. The prior is `N(0, I)`. An update absorbs one
//! observation with an exact single-observation Laplace step: the MAP of
//! `prior * likelihood(x, y)` lies on the line `w = m + t * cov x`, so it reduces to a
//! monotone scalar equation solved by Newton's method, and the new covariance follows by a
//! Sherman-Morrison rank-one downdate. No IO lives here; the worker and the decision handler
//! own persistence.
//!
//! Sampling is replayable: a decision stores its `u64` seed, and the same seed with the same
//! state always draws the same weights (ChaCha8, arms visited in ascending id order).

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, StandardNormal};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Bias plus the three Phase 0 context features.
pub const DIM: usize = 4;
/// Feature names in vector order. Decisions store features by name, never by position.
pub const FEATURE_NAMES: [&str; DIM] = ["bias", "returning", "mobile", "from_ad"];
pub const POLICY: &str = "lints";

pub type Vector = [f64; DIM];
pub type Matrix = [[f64; DIM]; DIM];

pub fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

fn dot(a: &Vector, b: &Vector) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn mat_vec(m: &Matrix, v: &Vector) -> Vector {
    let mut out = [0.0; DIM];
    for (i, row) in m.iter().enumerate() {
        out[i] = dot(row, v);
    }
    out
}

/// Lower-triangular `L` with `L * L^T = a`. A tiny diagonal jitter keeps a covariance that
/// has collapsed numerically along one direction from failing outright.
pub fn cholesky(a: &Matrix) -> Matrix {
    let mut l = [[0.0; DIM]; DIM];
    for i in 0..DIM {
        for j in 0..=i {
            let mut sum = a[i][j];
            for k in 0..j {
                sum -= l[i][k] * l[j][k];
            }
            if i == j {
                l[i][j] = sum.max(1e-12).sqrt();
            } else {
                l[i][j] = sum / l[j][j];
            }
        }
    }
    l
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arm {
    pub mean: Vector,
    pub cov: Matrix,
}

impl Default for Arm {
    fn default() -> Self {
        Self::prior()
    }
}

impl Arm {
    pub fn prior() -> Self {
        let mut cov = [[0.0; DIM]; DIM];
        for (i, row) in cov.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        Arm { mean: [0.0; DIM], cov }
    }

    /// Absorb one observation: `y = true` means the visitor converted.
    pub fn update(&mut self, x: &Vector, y: bool) {
        let y = if y { 1.0 } else { 0.0 };
        let sx = mat_vec(&self.cov, x);
        let s = dot(x, &sx);
        let a = dot(&self.mean, x);
        // Solve t = y - sigmoid(a + t s). g(t) = t - y + sigmoid(a + t s) is strictly
        // increasing, so Newton from t = 0 converges; the clamp guards the first step.
        let mut t = 0.0;
        for _ in 0..50 {
            let p = sigmoid(a + t * s);
            let g = t - y + p;
            let dg = 1.0 + p * (1.0 - p) * s;
            let step = g / dg;
            t -= step;
            t = t.clamp(-1.0, 1.0);
            if step.abs() < 1e-12 {
                break;
            }
        }
        for i in 0..DIM {
            self.mean[i] += t * sx[i];
        }
        let p = sigmoid(dot(&self.mean, x));
        let c = p * (1.0 - p);
        let denom = 1.0 + c * s;
        for i in 0..DIM {
            for j in 0..DIM {
                self.cov[i][j] -= c * sx[i] * sx[j] / denom;
            }
        }
    }

    /// One Thompson draw of `P(convert | x)`.
    pub fn sample_score(&self, x: &Vector, rng: &mut ChaCha8Rng) -> f64 {
        let l = cholesky(&self.cov);
        let mut z = [0.0; DIM];
        for zi in z.iter_mut() {
            *zi = StandardNormal.sample(rng);
        }
        let mut w = self.mean;
        for i in 0..DIM {
            for k in 0..=i {
                w[i] += l[i][k] * z[k];
            }
        }
        sigmoid(dot(&w, x))
    }
}

/// The persisted state of one slot's bandit. Arms are keyed by variant id; an arm with no
/// entry is at the prior, so a newly approved variant needs no migration.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LinTs {
    #[serde(default)]
    pub arms: BTreeMap<i64, Arm>,
    /// Observations absorbed. Read-modify-written by the worker, so a lost update shows here.
    #[serde(default)]
    pub n: u64,
    /// The stated-preference extension (see [`crate::stated`]); absent until a decision with
    /// `stated` features settles, so a site without pickers stores exactly what it did before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stated: Option<crate::stated::StatedModel>,
}

impl LinTs {
    pub fn arm(&self, id: i64) -> Arm {
        self.arms.get(&id).cloned().unwrap_or_default()
    }

    pub fn update(&mut self, arm: i64, x: &Vector, y: bool) {
        self.arms.entry(arm).or_default().update(x, y);
        self.n += 1;
    }

    /// Score every arm in `arm_ids` with one seeded draw each and return the winner and the
    /// scores, in ascending id order. `arm_ids` must be non-empty.
    pub fn choose(&self, arm_ids: &[i64], x: &Vector, seed: u64) -> (i64, Vec<(i64, f64)>) {
        let mut ids = arm_ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let scores: Vec<(i64, f64)> =
            ids.iter().map(|&id| (id, self.arm(id).sample_score(x, &mut rng))).collect();
        let best = scores
            .iter()
            .copied()
            .fold(None::<(i64, f64)>, |acc, (id, s)| match acc {
                Some((_, bs)) if bs >= s => acc,
                _ => Some((id, s)),
            })
            .expect("choose needs at least one arm")
            .0;
        (best, scores)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const X_RET: Vector = [1.0, 1.0, 0.0, 0.0];
    const X_NEW: Vector = [1.0, 0.0, 0.0, 0.0];

    fn quad(m: &Matrix, x: &Vector) -> f64 {
        dot(x, &mat_vec(m, x))
    }

    /// Solve `a z = b` for SPD `a` via Cholesky.
    fn solve(a: &Matrix, b: &Vector) -> Vector {
        let l = cholesky(a);
        let mut y = [0.0; DIM];
        for i in 0..DIM {
            let mut s = b[i];
            for k in 0..i {
                s -= l[i][k] * y[k];
            }
            y[i] = s / l[i][i];
        }
        let mut z = [0.0; DIM];
        for i in (0..DIM).rev() {
            let mut s = y[i];
            for k in i + 1..DIM {
                s -= l[k][i] * z[k];
            }
            z[i] = s / l[i][i];
        }
        z
    }

    /// Bernoulli draws from a planted logistic model, deterministic per seed.
    fn train(state: &mut LinTs, arm: i64, w_true: &Vector, xs: &[Vector], n: usize, seed: u64) {
        use rand::Rng;
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        for i in 0..n {
            let x = xs[i % xs.len()];
            let y = rng.random::<f64>() < sigmoid(dot(w_true, &x));
            state.update(arm, &x, y);
        }
    }

    #[test]
    fn sigmoid_is_half_at_zero_and_symmetric() {
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-15);
        assert!((sigmoid(-2.0) - (1.0 - sigmoid(2.0))).abs() < 1e-15);
        assert!(sigmoid(5.0) > 0.99);
    }

    #[test]
    fn cholesky_reconstructs_matrix() {
        let a: Matrix = [
            [4.0, 2.0, 0.4, 0.2],
            [2.0, 5.0, 1.0, 0.3],
            [0.4, 1.0, 3.0, 0.5],
            [0.2, 0.3, 0.5, 2.0],
        ];
        let l = cholesky(&a);
        for i in 0..DIM {
            for j in 0..DIM {
                let v: f64 = (0..DIM).map(|k| l[i][k] * l[j][k]).sum();
                assert!((v - a[i][j]).abs() < 1e-12, "({i},{j}) {v} vs {}", a[i][j]);
            }
        }
    }

    #[test]
    fn positive_update_raises_mean_along_x() {
        let mut arm = Arm::prior();
        arm.update(&X_RET, true);
        assert!(dot(&arm.mean, &X_RET) > 0.1, "{:?}", arm.mean);
    }

    #[test]
    fn negative_update_lowers_mean_along_x() {
        let mut arm = Arm::prior();
        arm.update(&X_RET, false);
        assert!(dot(&arm.mean, &X_RET) < -0.1, "{:?}", arm.mean);
    }

    #[test]
    fn update_shrinks_variance_along_x() {
        let mut arm = Arm::prior();
        let before = quad(&arm.cov, &X_RET);
        arm.update(&X_RET, true);
        let after = quad(&arm.cov, &X_RET);
        assert!(after < before * 0.95, "{before} -> {after}");
    }

    #[test]
    fn update_satisfies_map_stationarity() {
        // At the MAP: cov_old^-1 (w - m_old) = (y - sigmoid(w.x)) x.
        let mut arm = Arm::prior();
        arm.update(&X_NEW, true);
        arm.update(&X_RET, false);
        let old = arm.clone();
        let x: Vector = [1.0, 1.0, 1.0, 0.0];
        arm.update(&x, true);
        let diff: Vector = std::array::from_fn(|i| arm.mean[i] - old.mean[i]);
        let lhs = solve(&old.cov, &diff);
        let r = 1.0 - sigmoid(dot(&arm.mean, &x));
        for i in 0..DIM {
            assert!((lhs[i] - r * x[i]).abs() < 1e-9, "component {i}: {} vs {}", lhs[i], r * x[i]);
        }
    }

    #[test]
    fn same_seed_same_scores() {
        let mut s = LinTs::default();
        s.update(1, &X_RET, true);
        assert_eq!(s.choose(&[1, 2], &X_RET, 42), s.choose(&[1, 2], &X_RET, 42));
    }

    #[test]
    fn different_seed_different_scores() {
        let s = LinTs::default();
        assert_ne!(s.choose(&[1, 2], &X_RET, 1).1, s.choose(&[1, 2], &X_RET, 2).1);
    }

    #[test]
    fn posterior_recovers_planted_weights() {
        let w_true: Vector = [-1.0, 1.5, -0.8, 0.0];
        let xs: Vec<Vector> = (0..8)
            .map(|k| [1.0, (k & 1) as f64, ((k >> 1) & 1) as f64, ((k >> 2) & 1) as f64])
            .collect();
        let mut s = LinTs::default();
        train(&mut s, 7, &w_true, &xs, 4000, 11);
        let m = s.arm(7).mean;
        for i in 0..DIM {
            assert!((m[i] - w_true[i]).abs() < 0.35, "weight {i}: {} vs {}", m[i], w_true[i]);
        }
    }

    #[test]
    fn choose_prefers_clearly_better_arm() {
        let mut s = LinTs::default();
        let xs = [X_NEW];
        train(&mut s, 1, &[sigmoid_inv(0.30), 0.0, 0.0, 0.0], &xs, 400, 1);
        train(&mut s, 2, &[sigmoid_inv(0.05), 0.0, 0.0, 0.0], &xs, 400, 2);
        let wins = (0..200).filter(|&seed| s.choose(&[1, 2], &X_NEW, seed).0 == 1).count();
        assert!(wins >= 180, "better arm won {wins}/200");
    }

    #[test]
    fn choose_uses_context() {
        // Arm 1 converts returning visitors, arm 2 converts new ones.
        let mut s = LinTs::default();
        let xs = [X_RET, X_NEW];
        let hi = sigmoid_inv(0.30);
        let lo = sigmoid_inv(0.03);
        train(&mut s, 1, &[lo, hi - lo, 0.0, 0.0], &xs, 1200, 3);
        train(&mut s, 2, &[hi, lo - hi, 0.0, 0.0], &xs, 1200, 4);
        let ret = (0..200).filter(|&seed| s.choose(&[1, 2], &X_RET, seed).0 == 1).count();
        let new = (0..200).filter(|&seed| s.choose(&[1, 2], &X_NEW, seed).0 == 2).count();
        assert!(ret >= 170, "returning chose arm 1 {ret}/200");
        assert!(new >= 170, "new chose arm 2 {new}/200");
    }

    #[test]
    fn state_roundtrips_through_json() {
        let mut s = LinTs::default();
        s.update(3, &X_RET, true);
        s.update(3, &X_NEW, false);
        let back: LinTs = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(back.n, 2);
        assert_eq!(s.choose(&[3, 4], &X_RET, 9), back.choose(&[3, 4], &X_RET, 9));
    }

    fn sigmoid_inv(p: f64) -> f64 {
        (p / (1.0 - p)).ln()
    }
}

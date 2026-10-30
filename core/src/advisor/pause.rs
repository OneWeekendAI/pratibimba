//! A1's one card: "pause a slot that makes no difference".
//!
//! A slot is a candidate once every approved non-baseline arm has an estimate (each compared arm
//! served at least `min_rows` training rows). It is **confident** when every arm's 95% training
//! interval sits inside `±margin` orders per 100, and **early** otherwise (enough data, no
//! equivalence claim yet). A confident slot is proposed only if the held-out window (the newest
//! `held_out_days`, not used for training) agrees: every arm's held-out point estimate inside
//! `±2·margin`.
//!
//! "No difference" is an equivalence claim, so the false-discovery control runs on each slot's
//! TOST p-value (per arm the larger of the two one-sided p-values against `±margin`; per slot the
//! largest over its arms), Benjamini-Hochberg at `fdr_q` across one night's candidates. Then the
//! caps: no card for a slot with a card still open or rejected within `quiet_days`, and at most
//! `cards_per_week` per site per rolling week, smallest p first.
//!
//! The thresholds are per-site settings ([`Settings`], `sites.advisor_settings`); the constants
//! below are their defaults (owner decision 2026-10-23).

use super::uplift::{Estimate, Row, phi};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

pub const MIN_ROWS: u64 = 300;
pub const HELD_OUT_DAYS: i64 = 14;
pub const MARGIN: f64 = 0.3;
pub const FDR_Q: f64 = 0.10;
pub const CARDS_PER_WEEK: usize = 3;
pub const QUIET_DAYS: i64 = 30;

pub const MIN_ROWS_BOUNDS: (u64, u64) = (50, 1_000_000);
pub const HELD_OUT_DAYS_BOUNDS: (i64, i64) = (1, 90);
pub const MARGIN_BOUNDS: (f64, f64) = (0.01, 50.0);
pub const FDR_Q_BOUNDS: (f64, f64) = (0.001, 0.5);
pub const CARDS_PER_WEEK_BOUNDS: (usize, usize) = (0, 50);
pub const QUIET_DAYS_BOUNDS: (i64, i64) = (0, 365);

/// The advisor's per-site thresholds. Stored in `sites.advisor_settings` (NULL = all defaults);
/// set by the site file's `advisor_settings:` block or `POST /api/sites/{site}/advisor-settings`.
/// An absent field is its default.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Exposed settled training rows each compared arm needs (default 300).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_rows: Option<u64>,
    /// "No difference" means within this many orders per 100 of the baseline (default 0.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin: Option<f64>,
    /// The newest days kept out of training and used to check the claim (default 14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_out_days: Option<i64>,
    /// Benjamini-Hochberg false discovery rate across one night's candidates (default 0.10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fdr_q: Option<f64>,
    /// Cards per site per rolling week (default 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cards_per_week: Option<usize>,
    /// Days a slot whose card was rejected stays quiet (default 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_days: Option<i64>,
}

fn within<T: PartialOrd + std::fmt::Display + Copy>(name: &str, v: Option<T>, (lo, hi): (T, T)) -> Result<(), String> {
    match v {
        Some(v) if !(v >= lo && v <= hi) => Err(format!("{name} is {lo} to {hi}")),
        _ => Ok(()),
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        within("min_rows", self.min_rows, MIN_ROWS_BOUNDS)?;
        within("margin", self.margin, MARGIN_BOUNDS)?;
        within("held_out_days", self.held_out_days, HELD_OUT_DAYS_BOUNDS)?;
        within("fdr_q", self.fdr_q, FDR_Q_BOUNDS)?;
        within("cards_per_week", self.cards_per_week, CARDS_PER_WEEK_BOUNDS)?;
        within("quiet_days", self.quiet_days, QUIET_DAYS_BOUNDS)
    }

    pub fn min_rows(&self) -> u64 {
        self.min_rows.unwrap_or(MIN_ROWS)
    }
    pub fn margin(&self) -> f64 {
        self.margin.unwrap_or(MARGIN)
    }
    pub fn held_out_days(&self) -> i64 {
        self.held_out_days.unwrap_or(HELD_OUT_DAYS)
    }
    pub fn fdr_q(&self) -> f64 {
        self.fdr_q.unwrap_or(FDR_Q)
    }
    pub fn cards_per_week(&self) -> usize {
        self.cards_per_week.unwrap_or(CARDS_PER_WEEK)
    }
    pub fn quiet_days(&self) -> i64 {
        self.quiet_days.unwrap_or(QUIET_DAYS)
    }

    /// The settings with every default filled in, as the API shows them.
    pub fn effective(&self) -> Value {
        json!({
            "min_rows": self.min_rows(), "margin": self.margin(), "held_out_days": self.held_out_days(),
            "fdr_q": self.fdr_q(), "cards_per_week": self.cards_per_week(), "quiet_days": self.quiet_days(),
            "defaults": { "min_rows": MIN_ROWS, "margin": MARGIN, "held_out_days": HELD_OUT_DAYS, "fdr_q": FDR_Q,
                          "cards_per_week": CARDS_PER_WEEK, "quiet_days": QUIET_DAYS },
            "bounds": { "min_rows": [MIN_ROWS_BOUNDS.0, MIN_ROWS_BOUNDS.1], "margin": [MARGIN_BOUNDS.0, MARGIN_BOUNDS.1],
                        "held_out_days": [HELD_OUT_DAYS_BOUNDS.0, HELD_OUT_DAYS_BOUNDS.1], "fdr_q": [FDR_Q_BOUNDS.0, FDR_Q_BOUNDS.1],
                        "cards_per_week": [CARDS_PER_WEEK_BOUNDS.0, CARDS_PER_WEEK_BOUNDS.1], "quiet_days": [QUIET_DAYS_BOUNDS.0, QUIET_DAYS_BOUNDS.1] },
        })
    }

    /// The stored column: NULL when everything is a default.
    pub fn stored(&self) -> Option<Value> {
        (*self != Settings::default()).then(|| serde_json::to_value(self).expect("settings serialise"))
    }

    pub fn parse(v: Option<&Value>) -> Settings {
        v.and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    NotEnoughData,
    Early,
    Confident,
}

impl State {
    pub fn as_str(&self) -> &'static str {
        match self {
            State::NotEnoughData => "not_enough_data",
            State::Early => "early",
            State::Confident => "confident",
        }
    }
}

/// One arm's TOST p-value against `±margin`: the larger of the two one-sided p-values.
pub fn tost_p(e: &Estimate, margin: f64) -> f64 {
    if e.se <= 0.0 {
        return if e.uplift.abs() < margin { 0.0 } else { 1.0 };
    }
    let below = 1.0 - phi((e.uplift + margin) / e.se);
    let above = phi((e.uplift - margin) / e.se);
    below.max(above)
}

/// The slot's state and whether to propose a pause. `train` and `held` hold one entry per
/// approved non-baseline arm, in the same order; `None` = under the minimum.
pub fn judge(train: &[Option<Estimate>], held: &[Option<Estimate>], s: &Settings) -> (State, bool) {
    let m = s.margin();
    if train.is_empty() || train.iter().any(Option::is_none) {
        return (State::NotEnoughData, false);
    }
    let confident = train.iter().flatten().all(|e| e.lo >= -m && e.hi <= m);
    if !confident {
        return (State::Early, false);
    }
    let holds = held.len() == train.len() && held.iter().all(|h| h.is_some_and(|h| h.uplift.abs() <= 2.0 * m));
    (State::Confident, holds)
}

/// The slot's TOST p-value: the largest over its arms (every arm must be equivalent).
pub fn slot_p(train: &[Option<Estimate>], s: &Settings) -> f64 {
    train.iter().map(|e| e.map_or(1.0, |e| tost_p(&e, s.margin()))).fold(0.0, f64::max)
}

/// Benjamini-Hochberg step-up at level `q`: which hypotheses are rejected (discoveries).
pub fn bh(ps: &[f64], q: f64) -> Vec<bool> {
    let m = ps.len();
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|a, b| ps[*a].total_cmp(&ps[*b]));
    let k = order.iter().enumerate().filter(|(rank, i)| ps[**i] <= (*rank + 1) as f64 * q / m as f64).map(|(rank, _)| rank + 1).max().unwrap_or(0);
    let mut out = vec![false; m];
    for i in &order[..k] {
        out[*i] = true;
    }
    out
}

/// One slot's night: its verdict and the numbers the cap ranks by.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub slot_id: i64,
    pub state: State,
    pub propose: bool,
    /// The slot's TOST p-value.
    pub p: f64,
    /// Training rows compared.
    pub rows: u64,
}

/// Slots that may not get a card now: a rejected card decided within `quiet_days`.
pub fn quiet(rejected: &[(i64, DateTime<Utc>)], now: DateTime<Utc>, s: &Settings) -> HashSet<i64> {
    rejected.iter().filter(|(_, at)| now - *at < Duration::days(s.quiet_days())).map(|(slot, _)| *slot).collect()
}

/// The cards to write tonight: BH over every candidate with enough data, then the proposed
/// discoveries not in `blocked` (open or quiet slots), best evidence first (smallest p, then most
/// rows), at most `cards_per_week - recent` of them.
pub fn cap(cands: &[Candidate], recent: usize, blocked: &HashSet<i64>, s: &Settings) -> Vec<Candidate> {
    let tested: Vec<&Candidate> = cands.iter().filter(|c| c.state != State::NotEnoughData).collect();
    let found = bh(&tested.iter().map(|c| c.p).collect::<Vec<_>>(), s.fdr_q());
    let mut keep: Vec<Candidate> =
        tested.iter().zip(found).filter(|(c, f)| *f && c.propose && !blocked.contains(&c.slot_id)).map(|(c, _)| (*c).clone()).collect();
    keep.sort_by(|a, b| a.p.total_cmp(&b.p).then(b.rows.cmp(&a.rows)));
    keep.truncate(s.cards_per_week().saturating_sub(recent));
    keep
}

/// Training rows (older than the held-out window) and held-out rows (the newest `held_out_days`).
pub fn split(rows: Vec<Row>, now: DateTime<Utc>, s: &Settings) -> (Vec<Row>, Vec<Row>) {
    let cut = now - Duration::days(s.held_out_days());
    rows.into_iter().partition(|r| r.at < cut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;

    fn est(uplift: f64, se: f64, n: u64) -> Option<Estimate> {
        Some(Estimate { arm: 1, n_arm: n, n_base: n, n: 2 * n, uplift, se, lo: uplift - 1.96 * se, hi: uplift + 1.96 * se, p: 0.5 })
    }

    fn def() -> Settings {
        Settings::default()
    }

    fn cand(slot_id: i64, p: f64, rows: u64) -> Candidate {
        Candidate { slot_id, state: State::Confident, propose: true, p, rows }
    }

    // Mutation: require `e.hi <= -m` (sign slip in the interval test) -> red.
    #[test]
    fn flat_slot_proposes_pause() {
        let train = [est(0.02, 0.05, 5000), est(-0.05, 0.06, 5000)];
        let held = [est(0.1, 0.2, 900), est(-0.2, 0.2, 900)];
        assert_eq!(judge(&train, &held, &def()), (State::Confident, true));
    }

    // Mutation: test only the first arm's interval (`train.iter().take(1)`) -> red.
    #[test]
    fn one_arm_helps_no_pause() {
        let train = [est(0.0, 0.05, 5000), est(3.0, 0.5, 5000)];
        let held = [est(0.0, 0.2, 900), est(3.0, 0.5, 900)];
        assert_eq!(judge(&train, &held, &def()), (State::Early, false));
    }

    // Mutation: skip the held-out comparison (`holds = true`) -> red.
    #[test]
    fn train_flat_held_out_moves_no_pause() {
        let train = [est(0.0, 0.05, 5000)];
        let held = [est(1.5, 0.3, 900)];
        assert_eq!(judge(&train, &held, &def()), (State::Confident, false));
    }

    // Mutation: a held-out arm under the minimum counts as holding (`is_none_or`) -> red.
    #[test]
    fn held_out_without_data_no_pause() {
        let train = [est(0.0, 0.05, 5000)];
        assert_eq!(judge(&train, &[None], &def()), (State::Confident, false));
    }

    // Mutation: treat a missing estimate as flat (drop the `any(is_none)` check) -> red.
    #[test]
    fn below_min_rows_not_enough_data() {
        let train = [est(0.0, 0.05, 5000), None];
        assert_eq!(judge(&train, &[est(0.0, 0.1, 900), None], &def()).0, State::NotEnoughData);
    }

    // Mutation: judge uses the constant MARGIN instead of the site's margin -> red.
    #[test]
    fn margin_setting_widens_no_difference() {
        let train = [est(0.4, 0.2, 5000)];
        let held = [est(0.5, 0.4, 900)];
        assert_eq!(judge(&train, &held, &def()).0, State::Early);
        let wide = Settings { margin: Some(1.0), ..def() };
        assert_eq!(judge(&train, &held, &wide), (State::Confident, true));
    }

    // Mutation: BH compares each p with q (no rank correction) -> red.
    #[test]
    fn bh_rejects_noise_among_twenty() {
        // 20 null slots per night, p uniform: over 2,000 nights, BH at q = 0.10 should discover
        // something on about 10% of nights (it controls FWER at q under the global null); an
        // uncorrected test at 0.10 discovers something on 1 - 0.9^20 = 88% of nights.
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let nights = 2_000;
        let noisy = (0..nights)
            .filter(|_| {
                let ps: Vec<f64> = (0..20).map(|_| rng.random::<f64>()).collect();
                bh(&ps, 0.10).iter().any(|r| *r)
            })
            .count();
        let rate = noisy as f64 / nights as f64;
        assert!(rate < 0.13, "false-discovery nights {rate}");
        // And real signals among the noise still get through.
        let mut ps: Vec<f64> = (0..17).map(|_| rng.random::<f64>().max(0.05)).collect();
        ps.extend([0.0001, 0.0002, 0.0004]);
        let found = bh(&ps, 0.10);
        assert_eq!(found.iter().filter(|r| **r).count(), 3, "{ps:?}");
        assert!(found[17] && found[18] && found[19]);
    }

    // Mutation: bh uses the constant FDR_Q instead of the site's fdr_q -> red.
    #[test]
    fn fdr_setting_changes_what_passes() {
        let cands = [cand(1, 0.03, 900), cand(2, 0.04, 900)];
        assert_eq!(cap(&cands, 0, &HashSet::new(), &def()).len(), 2, "0.04 <= 2 x 0.10 / 2");
        let strict = Settings { fdr_q: Some(0.01), ..def() };
        assert!(cap(&cands, 0, &HashSet::new(), &strict).is_empty());
    }

    // Mutation: run BH over the proposed candidates only (the early ones' p left out) -> red.
    #[test]
    fn bh_counts_every_tested_slot() {
        // One proposed slot at p = 0.04, nine early ones: alone it passes, among ten it does not.
        let mut cands = vec![cand(1, 0.04, 900)];
        cands.extend((2..11).map(|i| Candidate { state: State::Early, propose: false, ..cand(i, 0.9, 900) }));
        assert!(cap(&cands, 0, &HashSet::new(), &def()).is_empty());
    }

    // Mutation: cap at 4 -> red.
    #[test]
    fn cap_keeps_best_three() {
        let cands: Vec<Candidate> = (1..=5).map(|i| cand(i, 0.001 * i as f64, 900)).collect();
        let kept: Vec<i64> = cap(&cands, 0, &HashSet::new(), &def()).iter().map(|c| c.slot_id).collect();
        assert_eq!(kept, vec![1, 2, 3]);
    }

    // Mutation: ignore `recent` (cards already written this week) -> red.
    #[test]
    fn cap_counts_this_weeks_cards() {
        let cands: Vec<Candidate> = (1..=5).map(|i| cand(i, 0.001 * i as f64, 900)).collect();
        assert_eq!(cap(&cands, 2, &HashSet::new(), &def()).len(), 1);
    }

    // Mutation: cap uses the constant CARDS_PER_WEEK -> red.
    #[test]
    fn cards_per_week_setting_changes_cap() {
        let cands: Vec<Candidate> = (1..=5).map(|i| cand(i, 0.001 * i as f64, 900)).collect();
        let one = Settings { cards_per_week: Some(1), ..def() };
        assert_eq!(cap(&cands, 0, &HashSet::new(), &one).len(), 1);
    }

    // Mutation: quiet compares with `>` (the 30 days counted the wrong way) -> red.
    #[test]
    fn rejected_slot_quiet_thirty_days() {
        let now = DateTime::from_timestamp(1_760_000_000, 0).unwrap();
        let rejected = [(7, now - Duration::days(29)), (8, now - Duration::days(31))];
        let q = quiet(&rejected, now, &def());
        assert_eq!(q, HashSet::from([7]));
        let cands = [cand(7, 0.001, 900), cand(8, 0.002, 900)];
        assert_eq!(cap(&cands, 0, &q, &def()).iter().map(|c| c.slot_id).collect::<Vec<_>>(), vec![8]);
    }

    // Mutation: quiet uses the constant QUIET_DAYS -> red.
    #[test]
    fn quiet_days_setting_shortens_quiet() {
        let now = DateTime::from_timestamp(1_760_000_000, 0).unwrap();
        let rejected = [(7, now - Duration::days(10))];
        assert_eq!(quiet(&rejected, now, &Settings { quiet_days: Some(7), ..def() }), HashSet::new());
    }

    // Mutation: TOST takes the smaller one-sided p (min instead of max) -> red.
    #[test]
    fn tost_p_is_the_larger_side() {
        // 0.25 +/- 0.1 against 0.3: below is ~0, above is P(Z > 0.5) ~ 0.31.
        let e = est(0.25, 0.1, 5000).unwrap();
        assert!((tost_p(&e, 0.3) - (1.0 - phi(0.5))).abs() < 1e-9, "{}", tost_p(&e, 0.3));
    }

    // Mutation: slot_p takes the smallest arm p -> red.
    #[test]
    fn slot_p_is_the_worst_arm() {
        let train = [est(0.0, 0.05, 5000), est(0.25, 0.1, 5000)];
        assert!((slot_p(&train, &def()) - tost_p(&train[1].unwrap(), 0.3)).abs() < 1e-12);
    }

    fn row_at(at: DateTime<Utc>) -> Row {
        Row { arm: 1, pool: vec![1], propensity: 1.0, reward: 0.0, at }
    }

    // Mutation: split at a constant 14 days instead of the site's held_out_days -> red.
    #[test]
    fn held_out_days_setting_moves_the_split() {
        let now = DateTime::from_timestamp(1_760_000_000, 0).unwrap();
        let rows = vec![row_at(now - Duration::days(20)), row_at(now - Duration::days(10)), row_at(now - Duration::days(2))];
        let (train, held) = split(rows.clone(), now, &def());
        assert_eq!((train.len(), held.len()), (1, 2));
        let (train, held) = split(rows, now, &Settings { held_out_days: Some(7), ..def() });
        assert_eq!((train.len(), held.len()), (2, 1));
    }

    // Mutation: validate skips the margin bound -> red.
    #[test]
    fn settings_out_of_bounds_refused() {
        assert_eq!(Settings { margin: Some(0.0), ..def() }.validate(), Err("margin is 0.01 to 50".into()));
        assert!(Settings { fdr_q: Some(0.6), ..def() }.validate().is_err());
        assert_eq!(def().stored(), None);
    }
}

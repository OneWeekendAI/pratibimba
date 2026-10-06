//! One decision, replayed and told in plain English.
//!
//! The replay re-runs the policy against the model **as it was at the decision's version**
//! (`model_versions`) with the stored seed and pool, and reports whether it reproduces the
//! stored scores exactly. The counterfactual flips the holdout flag: a treated visitor's
//! counterfactual is the baseline; a holdout visitor's is what the policy picked with the same
//! seed. Both carry the model's inferred conversion rate for this visitor's features.

use crate::bandit::{self, LinTs, Vector, sigmoid};
use crate::http::decide::from_named;
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Converted { after: Duration },
    Missed,
    Open { closes: DateTime<Utc> },
    NeverShown,
    NotShownYet,
}

/// Everything the sentence needs, and nothing it does not.
#[derive(Clone, Debug)]
pub struct Facts {
    pub variant: String,
    pub subject: String,
    pub at: DateTime<Utc>,
    pub returning: bool,
    pub mobile: bool,
    pub from_ad: bool,
    pub policy: String,
    pub version: i64,
    /// (variant key, score), highest first.
    pub scores: Vec<(String, f64)>,
    pub in_holdout: bool,
    /// For a holdout visitor: what the policy picked with the same seed.
    pub policy_pick: Option<String>,
    pub metric: String,
    pub window_mins: i64,
    pub outcome: Outcome,
}

fn short(subject: &str) -> String {
    if subject.chars().count() > 12 { format!("{}…", subject.chars().take(10).collect::<String>()) } else { subject.to_string() }
}

fn ago(d: Duration) -> String {
    let s = d.num_seconds().max(0);
    match s {
        0..60 => format!("{s} second{}", if s == 1 { "" } else { "s" }),
        60..3600 => {
            let m = s / 60;
            format!("{m} minute{}", if m == 1 { "" } else { "s" })
        }
        _ => {
            let h = s / 3600;
            format!("{h} hour{}", if h == 1 { "" } else { "s" })
        }
    }
}

fn policy_name(policy: &str) -> String {
    if policy == bandit::POLICY { "LinTS".into() } else { policy.to_string() }
}

pub fn sentence(f: &Facts) -> String {
    let mut out = format!("We showed `{}` to visitor `{}` on {}.", f.variant, short(&f.subject), f.at.format("%Y-%m-%d %H:%M UTC"));
    out += &format!(
        " They were {}, on {}, {}.",
        if f.returning { "a returning visitor" } else { "a new visitor" },
        if f.mobile { "mobile" } else { "desktop" },
        if f.from_ad { "arriving from an ad" } else { "not from an ad" }
    );
    let others: Vec<String> =
        f.scores.iter().filter(|(k, _)| *k != f.variant).take(3).map(|(k, s)| format!("{s:.2} for `{k}`")).collect();
    let own = f.scores.iter().find(|(k, _)| *k == f.variant).map(|(_, s)| *s);
    if f.in_holdout {
        let pick = f.policy_pick.as_deref().unwrap_or("?");
        let pick_score = f.scores.iter().find(|(k, _)| k == pick).map(|(_, s)| format!(" (it scored {s:.2})")).unwrap_or_default();
        out += &format!(
            " They were in the holdout, so they saw the baseline whatever the policy thought; the {} v{} policy would have shown `{pick}`{pick_score}.",
            policy_name(&f.policy),
            f.version
        );
    } else {
        match own {
            Some(s) if !others.is_empty() => {
                out += &format!(" The {} v{} policy scored it {s:.2} against {}.", policy_name(&f.policy), f.version, others.join(", "))
            }
            Some(s) => out += &format!(" The {} v{} policy scored it {s:.2}; it was the only eligible variant.", policy_name(&f.policy), f.version),
            None => {}
        }
        out += " They were not in the holdout.";
    }
    out += &match &f.outcome {
        Outcome::Converted { after } => {
            let lesson = if f.in_holdout { "it was recorded for the holdout and not learned from" } else { "the model learned from it" };
            format!(" They converted (`{}`) {} later, and {lesson}.", f.metric, ago(*after))
        }
        Outcome::Missed => {
            let lesson = if f.in_holdout { "recorded for the holdout" } else { "which taught the model a miss" };
            format!(" They did not `{}` within the {}-minute window, {lesson}.", f.metric, f.window_mins)
        }
        Outcome::Open { closes } => {
            format!(" Their {}-minute window for `{}` is still open; it closes at {}.", f.window_mins, f.metric, closes.format("%H:%M UTC"))
        }
        Outcome::NeverShown => " The page never reported showing it, so it taught the model nothing.".to_string(),
        Outcome::NotShownYet => " The page has not reported showing it yet.".to_string(),
    };
    out
}

#[derive(Clone, Debug, Serialize)]
pub struct Score {
    pub variant: String,
    pub variant_id: i64,
    pub score: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Counterfactual {
    /// `baseline` for a treated visitor, `policy` for a holdout visitor.
    pub kind: &'static str,
    pub variant: String,
    pub payload: Option<String>,
    pub is_baseline: bool,
    /// The model's posterior-mean conversion rate for this visitor's features under that variant.
    pub inferred_rate: Option<f64>,
    /// The same for the variant actually shown.
    pub shown_inferred_rate: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TimelineEvent {
    pub at: DateTime<Utc>,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Detail {
    pub id: Uuid,
    pub site: String,
    pub slot: String,
    pub metric: String,
    pub subject: String,
    pub variant: String,
    pub payload: Option<String>,
    pub is_baseline: bool,
    pub created_at: DateTime<Utc>,
    pub exposed_at: Option<DateTime<Utc>>,
    pub outcome: Option<i16>,
    pub outcome_at: Option<DateTime<Utc>>,
    pub settled_at: Option<DateTime<Utc>>,
    pub window_closes_at: DateTime<Utc>,
    pub in_holdout: bool,
    pub policy: String,
    pub policy_version: i64,
    pub seed: String,
    pub features: Value,
    pub scores: Vec<Score>,
    /// Re-running the policy on the stored version, seed and pool gives the stored scores.
    pub replay_matches: Option<bool>,
    pub counterfactual: Option<Counterfactual>,
    pub timeline: Vec<TimelineEvent>,
    /// What reviewers wrote on the shown variant at the gate: approval, comments, rejection.
    pub review: Vec<ReviewNote>,
    pub sentence: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReviewNote {
    pub action: String,
    pub actor: String,
    pub comment: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    site_id: String,
    slot_id: i64,
    key: String,
    metric: String,
    attribution_window_secs: i32,
    subject: String,
    variant_id: i64,
    pool: Vec<i64>,
    features: Value,
    scores: Value,
    policy: String,
    policy_version: i64,
    seed: i64,
    in_holdout: bool,
    created_at: DateTime<Utc>,
    exposed_at: Option<DateTime<Utc>>,
    outcome: Option<i16>,
    outcome_at: Option<DateTime<Utc>>,
    settled_at: Option<DateTime<Utc>>,
}

pub async fn detail(pool: &PgPool, id: Uuid, now: DateTime<Utc>) -> Result<Option<Detail>> {
    let row: Option<Row> = sqlx::query_as(
        "SELECT d.id, d.site_id, d.slot_id, s.key, s.metric, s.attribution_window_secs, d.subject, d.variant_id, d.pool,
                d.features, d.scores, d.policy, d.policy_version, d.seed, d.in_holdout, d.created_at, d.exposed_at,
                d.outcome, d.outcome_at, d.settled_at
           FROM decisions d JOIN slots s ON s.id = d.slot_id WHERE d.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    let Some(Row {
        id,
        site_id: site,
        slot_id,
        key: slot,
        metric,
        attribution_window_secs: window,
        subject,
        variant_id,
        pool: arm_pool,
        features,
        scores: stored,
        policy,
        policy_version: version,
        seed,
        in_holdout: held,
        created_at: created,
        exposed_at: exposed,
        outcome,
        outcome_at,
        settled_at: settled,
    }) = row
    else {
        return Ok(None);
    };
    let variants: Vec<(i64, String, Option<String>, bool)> =
        sqlx::query_as("SELECT id, key, payload, is_baseline FROM variants WHERE slot_id = $1 ORDER BY id").bind(slot_id).fetch_all(pool).await?;
    let key = |vid: i64| variants.iter().find(|v| v.0 == vid).map(|v| v.1.clone()).unwrap_or_else(|| format!("#{vid}"));
    let shown = variants.iter().find(|v| v.0 == variant_id).cloned().unwrap_or((variant_id, key(variant_id), None, false));

    let mut scores: Vec<Score> = stored
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some(Score { variant_id: k.parse().ok()?, variant: key(k.parse().ok()?), score: v.as_f64()? }))
                .collect()
        })
        .unwrap_or_default();
    scores.sort_by(|a, b| b.score.total_cmp(&a.score));

    let x: Option<Vector> = from_named(&features);
    let model: Option<LinTs> = sqlx::query_scalar::<_, Value>("SELECT state FROM model_versions WHERE slot_id = $1 AND version = $2")
        .bind(slot_id)
        .bind(version)
        .fetch_optional(pool)
        .await?
        .and_then(|s| serde_json::from_value(s).ok());

    let (replay_matches, policy_pick) = match (&model, x) {
        // An on-device decision has no seeded draw to replay.
        (Some(m), Some(x)) if !arm_pool.is_empty() && policy != crate::adapt::POLICY => {
            let said = crate::stated::ext_from_features(&features);
            let (pick, replayed) = if crate::stated::is_extended(&policy) {
                let keys: Vec<String> = said.keys().cloned().collect();
                crate::stated::choose(m, &keys, &arm_pool, &x, &said, seed as u64)
            } else {
                m.choose(&arm_pool, &x, seed as u64)
            };
            let matches = replayed.len() == scores.len()
                && replayed.iter().all(|(vid, s)| scores.iter().any(|o| o.variant_id == *vid && o.score == *s));
            (Some(matches), Some(pick))
        }
        _ => (None, None),
    };
    let rate = |vid: i64| -> Option<f64> {
        let (m, x) = (model.as_ref()?, x?);
        if !crate::stated::ext_from_features(&features).is_empty() {
            return Some(crate::stated::mean_score(m, vid, &x, &features));
        }
        let mean = m.arm(vid).mean;
        Some(sigmoid(mean.iter().zip(&x).map(|(a, b)| a * b).sum()))
    };
    let counterfactual = if held {
        policy_pick.and_then(|p| variants.iter().find(|v| v.0 == p)).map(|v| Counterfactual {
            kind: "policy",
            variant: v.1.clone(),
            payload: v.2.clone(),
            is_baseline: v.3,
            inferred_rate: rate(v.0),
            shown_inferred_rate: rate(variant_id),
        })
    } else {
        variants.iter().find(|v| v.3).map(|v| Counterfactual {
            kind: "baseline",
            variant: v.1.clone(),
            payload: v.2.clone(),
            is_baseline: true,
            inferred_rate: rate(v.0),
            shown_inferred_rate: rate(variant_id),
        })
    };

    let window_closes_at = created + Duration::seconds(i64::from(window));
    let timeline: Vec<TimelineEvent> = sqlx::query_as::<_, (DateTime<Utc>, String)>(
        // Events in one batch share received_at; the exposure is what made the rest possible.
        "SELECT received_at, name FROM (
           SELECT received_at, name FROM events WHERE decision_id = $1
           UNION ALL
           SELECT received_at, name FROM events
            WHERE site_id = $2 AND subject = $3 AND decision_id IS NULL AND received_at BETWEEN $4 AND $5
         ) t ORDER BY received_at, name <> 'exposure', name",
    )
    .bind(id)
    .bind(&site)
    .bind(&subject)
    .bind(created)
    .bind(window_closes_at)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(at, name)| TimelineEvent { at, name })
    .collect();

    let review: Vec<ReviewNote> = sqlx::query_as::<_, (String, String, Option<String>, DateTime<Utc>)>(
        "SELECT action, actor, comment, created_at FROM approvals WHERE variant_id = $1 ORDER BY created_at",
    )
    .bind(variant_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(action, actor, comment, at)| ReviewNote { action, actor, comment, at })
    .collect();

    let outcome_kind = match (outcome, exposed) {
        (Some(1), _) => Outcome::Converted { after: outcome_at.unwrap_or(created) - created },
        (Some(_), _) => Outcome::Missed,
        (None, Some(_)) => Outcome::Open { closes: window_closes_at },
        (None, None) if settled.is_some() || now > window_closes_at => Outcome::NeverShown,
        (None, None) => Outcome::NotShownYet,
    };
    let fval = |n: &str| features.get(n).and_then(Value::as_f64).unwrap_or(0.0) > 0.5;
    let facts = Facts {
        variant: shown.1.clone(),
        subject: subject.clone(),
        at: created,
        returning: fval("returning"),
        mobile: fval("mobile"),
        from_ad: fval("from_ad"),
        policy: policy.clone(),
        version,
        scores: scores.iter().map(|s| (s.variant.clone(), s.score)).collect(),
        in_holdout: held,
        policy_pick: policy_pick.map(key),
        metric: metric.clone(),
        window_mins: i64::from(window) / 60,
        outcome: outcome_kind,
    };
    Ok(Some(Detail {
        id,
        site,
        slot,
        metric,
        subject,
        variant: shown.1,
        payload: shown.2,
        is_baseline: shown.3,
        created_at: created,
        exposed_at: exposed,
        outcome,
        outcome_at,
        settled_at: settled,
        window_closes_at,
        in_holdout: held,
        policy,
        policy_version: version,
        seed: (seed as u64).to_string(),
        features,
        scores,
        replay_matches,
        counterfactual,
        timeline,
        review,
        sentence: sentence(&facts),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn facts(outcome: Outcome, in_holdout: bool) -> Facts {
        Facts {
            variant: if in_holdout { "baseline".into() } else { "gift".into() },
            subject: "v4-ab98c0ffee1234".into(),
            at: Utc.with_ymd_and_hms(2026, 10, 7, 14, 3, 0).unwrap(),
            returning: true,
            mobile: false,
            from_ad: false,
            policy: "lints".into(),
            version: 12,
            scores: vec![("gift".into(), 0.63), ("baseline".into(), 0.41)],
            in_holdout,
            policy_pick: Some("gift".into()),
            metric: "click".into(),
            window_mins: 30,
            outcome,
        }
    }

    #[test]
    fn sentence_for_a_conversion() {
        let s = sentence(&facts(Outcome::Converted { after: Duration::minutes(4) }, false));
        assert!(s.starts_with("We showed `gift` to visitor `v4-ab98c0f…` on 2026-10-07 14:03 UTC."), "{s}");
        assert!(s.contains("a returning visitor, on desktop, not from an ad"), "{s}");
        assert!(s.contains("The LinTS v12 policy scored it 0.63 against 0.41 for `baseline`."), "{s}");
        assert!(s.contains("They were not in the holdout."), "{s}");
        assert!(s.contains("They converted (`click`) 4 minutes later, and the model learned from it."), "{s}");
    }

    #[test]
    fn sentence_for_a_miss() {
        let s = sentence(&facts(Outcome::Missed, false));
        assert!(s.contains("They did not `click` within the 30-minute window, which taught the model a miss."), "{s}");
    }

    #[test]
    fn sentence_for_an_open_window() {
        let closes = Utc.with_ymd_and_hms(2026, 10, 7, 14, 33, 0).unwrap();
        let s = sentence(&facts(Outcome::Open { closes }, false));
        assert!(s.contains("still open; it closes at 14:33 UTC"), "{s}");
    }

    #[test]
    fn sentence_for_a_decision_never_shown() {
        let s = sentence(&facts(Outcome::NeverShown, false));
        assert!(s.contains("never reported showing it, so it taught the model nothing"), "{s}");
    }

    #[test]
    fn sentence_for_a_holdout_visitor() {
        let s = sentence(&facts(Outcome::Converted { after: Duration::seconds(90) }, true));
        assert!(s.contains("They were in the holdout, so they saw the baseline"), "{s}");
        assert!(s.contains("would have shown `gift` (it scored 0.63)"), "{s}");
        assert!(s.contains("recorded for the holdout and not learned from"), "{s}");
        assert!(!s.contains("not in the holdout"), "{s}");
    }
}

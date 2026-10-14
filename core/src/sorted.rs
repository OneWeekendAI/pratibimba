//! F2: the `sorted` slot kind. A sorted slot's variants are items, and a decision serves an
//! order of them, best first (backend.md, "Sorted slots").
//!
//! - **Choosing.** Each item is an arm of the slot's LinTS model; the decision's Thompson draws
//!   (the scores every slot already records) are sorted, highest first, ties to the default order.
//! - **Holdout and pause.** The declared default order: the order the file lists the items in.
//! - **Learning (cascade).** At close, the topmost position whose item a metric event from the
//!   subject names in its `items` is the acted-on one: every item above it learns a miss and it
//!   learns the reward; items below learn nothing. Nothing acted on: the first `visible` items
//!   each learn a miss.

use crate::site_file::SlotSpec;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub const KIND: &str = "sorted";
pub const MIN_ITEMS: usize = 2;
pub const MAX_ITEMS: usize = 50;
pub const DEFAULT_VISIBLE: u32 = 4;

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SortedSpec {
    /// How many positions a visitor is assumed to read when acting on none (default 4).
    #[serde(default)]
    pub visible: Option<u32>,
}

/// The slot's stored `sorted` column: `{visible, order}`, or `None` for another kind.
pub fn stored(slot: &SlotSpec) -> Option<Value> {
    (slot.kind.as_deref() == Some(KIND)).then(|| {
        let visible = slot.sorted.as_ref().and_then(|s| s.visible).unwrap_or(DEFAULT_VISIBLE).min(slot.variants.len() as u32);
        json!({ "visible": visible, "order": slot.variants.iter().map(|v| &v.key).collect::<Vec<_>>() })
    })
}

/// A sorted slot's own rules; the caller has checked keys are unique slugs.
pub fn validate_slot(slot: &SlotSpec) -> Result<()> {
    let k = &slot.key;
    if slot.kind.as_deref() != Some(KIND) {
        if slot.sorted.is_some() {
            bail!("slot {k:?}: only a kind \"sorted\" slot takes sorted options");
        }
        return Ok(());
    }
    if slot.in_session || slot.picker.is_some() {
        bail!("slot {k:?}: a sorted slot cannot be in_session or carry picker options");
    }
    if !(MIN_ITEMS..=MAX_ITEMS).contains(&slot.variants.len()) {
        bail!("slot {k:?}: a sorted slot needs {MIN_ITEMS}-{MAX_ITEMS} items");
    }
    if let Some(v) = slot.sorted.as_ref().and_then(|s| s.visible)
        && (v == 0 || v as usize > slot.variants.len())
    {
        bail!("slot {k:?}: sorted.visible must be 1 to the number of items");
    }
    for v in &slot.variants {
        if v.baseline || v.html.is_some() {
            bail!("slot {k:?}: item {:?} takes no html and no baseline flag; the first item listed is the default order's head", v.key);
        }
    }
    Ok(())
}

/// The declared default order as arm ids, kept to `pool` (gated or unshared items drop out).
/// `order` is the stored item keys; `arms` is `(id, key)`. An approved item the file no longer
/// lists (not possible through `apply`, which retires it) goes last, by id.
pub fn default_order(order: &[String], arms: &[(i64, String)], pool: &[i64]) -> Vec<i64> {
    let pos = |id: i64| arms.iter().find(|a| a.0 == id).and_then(|a| order.iter().position(|k| *k == a.1)).unwrap_or(usize::MAX);
    let mut out: Vec<i64> = pool.to_vec();
    out.sort_by_key(|id| (pos(*id), *id));
    out
}

/// The served order: `pool` by the decision's score, highest first, ties to the default order.
pub fn rank(scores: &[(i64, f64)], default: &[i64]) -> Vec<i64> {
    let score = |id: i64| scores.iter().find(|s| s.0 == id).map_or(f64::NEG_INFINITY, |s| s.1);
    let mut out = default.to_vec();
    // Stable: equal scores keep the default order.
    out.sort_by(|a, b| score(*b).total_cmp(&score(*a)));
    out
}

/// The cascade's lessons for one closed decision: `(arm, reward)`. `acted` is the position of
/// the topmost item acted on, if any; `reward` is what that item learns.
pub fn lessons(ranking: &[i64], acted: Option<usize>, visible: usize, reward: f64) -> Vec<(i64, f64)> {
    match acted {
        Some(p) => ranking.iter().take(p).map(|id| (*id, 0.0)).chain(ranking.get(p).map(|id| (*id, reward))).collect(),
        None => ranking.iter().take(visible).map(|id| (*id, 0.0)).collect(),
    }
}

/// Teach the model one closed sorted decision (see [`lessons`]). `reward` is what the acted-on
/// item learns: 1, or the revenue goal's capped share.
pub async fn learn(tx: &mut Transaction<'_, Postgres>, decision: Uuid, slot: i64, features: &Value, reward: f64) -> Result<()> {
    let (ranking, visible, keys): (Vec<i64>, i32, Vec<String>) = sqlx::query_as(
        "SELECT d.ranking, (s.sorted->>'visible')::int,
                ARRAY(SELECT v.key FROM unnest(d.ranking) WITH ORDINALITY r(id, n) JOIN variants v ON v.id = r.id ORDER BY r.n)
           FROM decisions d JOIN slots s ON s.id = d.slot_id WHERE d.id = $1 AND d.ranking IS NOT NULL",
    )
    .bind(decision)
    .fetch_one(&mut **tx)
    .await?;
    // Every item id the subject's metric events named inside the window, counting only events that name
    // this decision or none (read through live_events, so an erased subject teaches nothing).
    let named: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT it->>'id'
           FROM decisions d JOIN slots s ON s.id = d.slot_id
           JOIN live_events e ON e.site_id = d.site_id AND e.subject = d.subject AND e.name = s.metric
           CROSS JOIN LATERAL jsonb_array_elements(CASE WHEN jsonb_typeof(e.items) = 'array' THEN e.items ELSE '[]' END) it
          WHERE d.id = $1 AND (e.decision_id IS NULL OR e.decision_id = d.id)
            AND e.received_at >= d.created_at
            AND e.received_at <= d.created_at + make_interval(secs => s.attribution_window_secs)",
    )
    .bind(decision)
    .fetch_all(&mut **tx)
    .await?;
    let acted = keys.iter().position(|k| named.contains(k));
    let todo = lessons(&ranking, acted, visible.max(0) as usize, reward);
    if todo.is_empty() {
        return Ok(());
    }
    let x = crate::http::decide::from_named(features).ok_or_else(|| anyhow::anyhow!("decision features are missing a named feature"))?;
    let state: Value = sqlx::query_scalar("SELECT state FROM models WHERE slot_id = $1 FOR UPDATE").bind(slot).fetch_one(&mut **tx).await?;
    let mut model: crate::bandit::LinTs = serde_json::from_value(state)?;
    for (arm, y) in todo {
        crate::stated::learn_credit(&mut model, arm, &x, features, y);
    }
    crate::worker::write_model(tx, slot, &serde_json::to_value(&model)?, "learn: ranking").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_sorts_by_score_and_ties_keep_the_default_order() {
        assert_eq!(rank(&[(1, 0.2), (2, 0.9), (3, 0.5)], &[1, 2, 3]), vec![2, 3, 1]);
        assert_eq!(rank(&[(1, 0.5), (2, 0.5), (3, 0.9)], &[2, 1, 3]), vec![3, 2, 1]);
    }

    #[test]
    fn default_order_follows_the_file_not_the_ids() {
        let arms = vec![(10, "a".to_string()), (11, "b".to_string()), (12, "c".to_string())];
        let order = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        assert_eq!(default_order(&order, &arms, &[10, 11, 12]), vec![12, 10, 11]);
        assert_eq!(default_order(&order, &arms, &[11, 10]), vec![10, 11]);
    }

    #[test]
    fn cascade_misses_above_the_act_and_nothing_below() {
        assert_eq!(lessons(&[7, 8, 9, 6], Some(2), 3, 1.0), vec![(7, 0.0), (8, 0.0), (9, 1.0)]);
        assert_eq!(lessons(&[7, 8, 9, 6], Some(0), 3, 0.4), vec![(7, 0.4)]);
        assert_eq!(lessons(&[7, 8, 9, 6], None, 3, 1.0), vec![(7, 0.0), (8, 0.0), (9, 0.0)]);
    }
}

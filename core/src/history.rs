//! Visitor history (F3, backend.md "Visitor history"): facts about a returning visitor worked out
//! from their own stored events (F1 orders, item events), read by the learner and by audience rules.
//!
//! The facts are computed inside decide's one snapshot statement ([`SQL`]), and only for an
//! identified request on a site whose `fields_allowed` admits `history`; otherwise they are absent,
//! and an audience rule that reads them matches nobody. In the learner they are declared-feature
//! dimensions (`feature:history.*`), so they replay, learn and show as declared features do.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The model key prefix of a history feature.
pub const PREFIX: &str = "feature:history.";
/// The `fields_allowed` name that admits history.
pub const FIELD: &str = "history";
/// Longest look-back of a view, in days.
pub const MAX_VIEW_DAYS: u32 = 90;
/// How many recent item events a decide reads.
pub const MAX_TOUCH_EVENTS: i64 = 50;
/// How far back an `add_to_cart` counts, in days.
pub const CART_LOOKBACK_DAYS: f64 = 30.0;
pub const DEFAULT_LTV_EDGES: [f64; 2] = [100.0, 1000.0];
pub const DEFAULT_CART_WINDOW_HOURS: u32 = 24;
pub const MAX_VIEWS: usize = 16;
/// The predicate keys that read history.
pub const KEYS: [&str; 5] = ["days_since_last_purchase", "purchase_count", "lifetime_value", "cart_abandoned", "viewed"];
pub const BUCKETS: [&str; 4] = ["none", "low", "mid", "high"];

/// The site file's `history:` block.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// `false` turns history off for the learner (audience rules still read it).
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub ltv_edges: Option<[f64; 2]>,
    #[serde(default)]
    pub cart_window_hours: Option<u32>,
    #[serde(default)]
    pub views: Vec<View>,
}

fn yes() -> bool {
    true
}

/// "Viewed item or category X in the last N days".
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct View {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub item: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    pub days: u32,
}

impl View {
    fn check(&self) -> Result<(), String> {
        if self.item.is_some() == self.category.is_some() {
            return Err("a view names an item or a category, not both".into());
        }
        if self.item.as_deref().or(self.category.as_deref()).is_some_and(|s| s.is_empty() || s.len() > 256) {
            return Err("a view's item or category is 1-256 characters".into());
        }
        if !(1..=MAX_VIEW_DAYS).contains(&self.days) {
            return Err(format!("a view's days is 1 to {MAX_VIEW_DAYS}"));
        }
        Ok(())
    }

    fn seen(&self, f: &Facts) -> bool {
        f.touches.iter().any(|(item, cat, days)| {
            *days <= f64::from(self.days)
                && match (&self.item, &self.category) {
                    (Some(i), _) => item == i,
                    (_, Some(c)) => cat.as_deref() == Some(c.as_str()),
                    _ => false,
                }
        })
    }
}

impl Spec {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some([a, b]) = self.ltv_edges {
            if !(a.is_finite() && b.is_finite() && 0.0 < a && a < b) {
                anyhow::bail!("history.ltv_edges: two increasing positive values, e.g. [100, 1000]");
            }
        }
        if self.cart_window_hours.is_some_and(|h| !(1..=720).contains(&h)) {
            anyhow::bail!("history.cart_window_hours: 1 to 720");
        }
        if self.views.len() > MAX_VIEWS {
            anyhow::bail!("history.views: at most {MAX_VIEWS}");
        }
        let mut seen = std::collections::BTreeSet::new();
        for v in &self.views {
            if !crate::site_file::is_slug(&v.key) {
                anyhow::bail!("history.views {:?}: key must be a lowercase slug", v.key);
            }
            v.check().map_err(|e| anyhow::anyhow!("history.views {:?}: {e}", v.key))?;
            if !seen.insert(&v.key) {
                anyhow::bail!("history.views {:?} is named twice", v.key);
            }
        }
        Ok(())
    }

    pub fn parse(v: Option<&Value>) -> Spec {
        v.and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or(Spec { enabled: false, ltv_edges: None, cart_window_hours: None, views: vec![] })
    }
}

/// One visitor's history, as of the site's clock.
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    pub days_since_last_purchase: Option<f64>,
    pub purchase_count: i64,
    /// Index into [`BUCKETS`].
    pub ltv_bucket: u8,
    pub cart_abandoned: bool,
    /// `(item id, category, days ago)` of the recent non-order item events.
    pub touches: Vec<(String, Option<String>, f64)>,
}

/// The history subquery of decide's snapshot: `{orders, last_order_days, ltv, last_cart_days, last_cart_ordered, touches}`
/// for the subject `subj` names, or NULL. It expects the `t` (with `currency`, `at`), `subj` CTEs and
/// `$1` site; `needed` is the SQL condition that the site or slot reads history.
pub fn sql(needed: &str) -> String {
    format!(
        "CASE WHEN (t.privacy_mode = 'full' OR (t.privacy_mode = 'consent_aware' AND $5::bool IS TRUE))
                AND (t.fields_allowed IS NULL OR '{FIELD}' = ANY(t.fields_allowed)) AND ({needed}) THEN
           (SELECT jsonb_build_object(
                'orders', count(*) FILTER (WHERE h.order_id IS NOT NULL),
                'last_order_days', extract(epoch FROM t.at - max(h.received_at) FILTER (WHERE h.order_id IS NOT NULL)) / 86400,
                'ltv', COALESCE(sum(h.value) FILTER (WHERE h.order_id IS NOT NULL AND h.currency = t.currency), 0)::float8,
                'last_cart_days', extract(epoch FROM t.at - max(h.received_at) FILTER (WHERE h.name = 'add_to_cart')) / 86400,
                'touches', (SELECT COALESCE(jsonb_agg(jsonb_build_array(it->>'id', it->>'category', extract(epoch FROM t.at - r.received_at) / 86400)), '[]')
                              FROM (SELECT items, received_at FROM live_events
                                     WHERE site_id = $1 AND subject = (SELECT v FROM subj) AND order_id IS NULL AND jsonb_typeof(items) = 'array'
                                       AND received_at <= t.at AND received_at > t.at - interval '{MAX_VIEW_DAYS} days'
                                     ORDER BY received_at DESC LIMIT {MAX_TOUCH_EVENTS}) r, jsonb_array_elements(r.items) it
                             WHERE jsonb_typeof(it) = 'object'))
              FROM live_events h WHERE h.site_id = $1 AND h.subject = (SELECT v FROM subj) AND h.received_at <= t.at)
         END"
    )
}

impl Facts {
    /// Read the snapshot's history column under the site's spec.
    pub fn from_row(v: &Value, spec: &Spec) -> Option<Facts> {
        let f = |k: &str| v.get(k).and_then(Value::as_f64);
        let purchase_count = v.get("orders")?.as_i64()?;
        let last_order = f("last_order_days");
        let ltv = f("ltv").unwrap_or(0.0);
        let [lo, hi] = spec.ltv_edges.unwrap_or(DEFAULT_LTV_EDGES);
        let ltv_bucket = if purchase_count == 0 || ltv <= 0.0 {
            0
        } else if ltv < lo {
            1
        } else if ltv < hi {
            2
        } else {
            3
        };
        let window = f64::from(spec.cart_window_hours.unwrap_or(DEFAULT_CART_WINDOW_HOURS)) / 24.0;
        let cart_abandoned = match f("last_cart_days") {
            // Abandoned: the cart is past its window, inside the look-back, and no order came after it.
            Some(c) => c >= window && c <= CART_LOOKBACK_DAYS && last_order.is_none_or(|o| o > c),
            None => false,
        };
        let touches = v
            .get("touches")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|t| Some((t.get(0)?.as_str()?.to_string(), t.get(1).and_then(Value::as_str).map(String::from), t.get(2)?.as_f64()?)))
                    .collect()
            })
            .unwrap_or_default();
        Some(Facts { days_since_last_purchase: last_order.map(f64::floor), purchase_count, ltv_bucket, cart_abandoned, touches })
    }

    /// The learner dimensions of these facts under `spec`, by model key.
    pub fn values(&self, spec: &Spec) -> BTreeMap<String, f64> {
        let b = |x: bool| if x { 1.0 } else { 0.0 };
        let mut out = BTreeMap::from([
            (format!("{PREFIX}days_since_last_purchase"), self.days_since_last_purchase.map_or(1.0, |d| d.clamp(0.0, 365.0) / 365.0)),
            (format!("{PREFIX}purchase_count"), self.purchase_count.clamp(0, 10) as f64 / 10.0),
            (format!("{PREFIX}lifetime_value"), f64::from(self.ltv_bucket) / 3.0),
            (format!("{PREFIX}cart_abandoned"), b(self.cart_abandoned)),
        ]);
        for v in &spec.views {
            out.insert(format!("{PREFIX}viewed.{}", v.key), b(v.seen(self)));
        }
        out
    }
}

/// One history condition of a segment predicate.
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    DaysSincePurchase { gte: Option<u32>, lte: Option<u32> },
    PurchaseCount { gte: Option<u32>, lte: Option<u32> },
    LifetimeValue(u8),
    CartAbandoned(bool),
    Viewed(View),
}

fn range(k: &str, v: &Value) -> Result<(Option<u32>, Option<u32>), String> {
    let o = v.as_object().filter(|o| !o.is_empty() && o.keys().all(|k| k == "gte" || k == "lte"));
    let err = || format!("{k} is {{gte?, lte?}} with whole numbers, e.g. {{\"gte\": 30}}");
    let o = o.ok_or_else(err)?;
    let n = |x: &str| o.get(x).map(|v| v.as_u64().filter(|n| *n <= 100_000).map(|n| n as u32).ok_or_else(err)).transpose();
    Ok((n("gte")?, n("lte")?))
}

impl Cond {
    pub fn parse(k: &str, v: &Value) -> Result<Cond, String> {
        Ok(match k {
            "days_since_last_purchase" => range(k, v).map(|(gte, lte)| Cond::DaysSincePurchase { gte, lte })?,
            "purchase_count" => range(k, v).map(|(gte, lte)| Cond::PurchaseCount { gte, lte })?,
            "lifetime_value" => {
                let i = v.as_str().and_then(|s| BUCKETS.iter().position(|b| *b == s)).ok_or_else(|| format!("lifetime_value is one of {}", BUCKETS.join(", ")))?;
                Cond::LifetimeValue(i as u8)
            }
            "cart_abandoned" => Cond::CartAbandoned(v.as_bool().ok_or("cart_abandoned takes true or false")?),
            "viewed" => {
                let view: View = serde_json::from_value(v.clone()).map_err(|_| "viewed is {item or category, days}".to_string())?;
                view.check()?;
                Cond::Viewed(view)
            }
            _ => return Err(format!("{k:?} is not a history condition")),
        })
    }

    pub fn key(&self) -> &'static str {
        match self {
            Cond::DaysSincePurchase { .. } => KEYS[0],
            Cond::PurchaseCount { .. } => KEYS[1],
            Cond::LifetimeValue(_) => KEYS[2],
            Cond::CartAbandoned(_) => KEYS[3],
            Cond::Viewed(_) => KEYS[4],
        }
    }

    pub fn to_json(&self) -> Value {
        let r = |gte: &Option<u32>, lte: &Option<u32>| {
            let mut m = serde_json::Map::new();
            if let Some(g) = gte {
                m.insert("gte".into(), json!(g));
            }
            if let Some(l) = lte {
                m.insert("lte".into(), json!(l));
            }
            Value::Object(m)
        };
        match self {
            Cond::DaysSincePurchase { gte, lte } | Cond::PurchaseCount { gte, lte } => r(gte, lte),
            Cond::LifetimeValue(b) => json!(BUCKETS[*b as usize]),
            Cond::CartAbandoned(b) => json!(b),
            Cond::Viewed(v) => match (&v.item, &v.category) {
                (Some(i), _) => json!({ "item": i, "days": v.days }),
                (_, c) => json!({ "category": c, "days": v.days }),
            },
        }
    }

    /// Absent facts match nothing.
    pub fn matches(&self, f: Option<&Facts>) -> bool {
        let Some(f) = f else { return false };
        let within = |x: f64, gte: &Option<u32>, lte: &Option<u32>| gte.is_none_or(|g| x >= f64::from(g)) && lte.is_none_or(|l| x <= f64::from(l));
        match self {
            Cond::DaysSincePurchase { gte, lte } => f.days_since_last_purchase.is_some_and(|d| within(d, gte, lte)),
            Cond::PurchaseCount { gte, lte } => within(f.purchase_count as f64, gte, lte),
            Cond::LifetimeValue(b) => f.ltv_bucket == *b,
            Cond::CartAbandoned(b) => f.cart_abandoned == *b,
            Cond::Viewed(v) => v.seen(f),
        }
    }

    pub fn describe(&self) -> String {
        let r = |what: &str, gte: &Option<u32>, lte: &Option<u32>| match (gte, lte) {
            (Some(g), Some(l)) => format!("{what} {g}-{l}"),
            (Some(g), None) => format!("{what} {g}+"),
            (None, Some(l)) => format!("{what} at most {l}"),
            _ => what.to_string(),
        };
        match self {
            Cond::DaysSincePurchase { gte, lte } => r("last order days ago", gte, lte),
            Cond::PurchaseCount { gte, lte } => r("orders", gte, lte),
            Cond::LifetimeValue(b) => format!("{} lifetime value", BUCKETS[*b as usize]),
            Cond::CartAbandoned(true) => "cart-abandoning".into(),
            Cond::CartAbandoned(false) => "no abandoned cart".into(),
            Cond::Viewed(v) => format!("viewed {} in {} days", v.item.as_deref().or(v.category.as_deref()).unwrap_or(""), v.days),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Spec {
        serde_json::from_value(json!({ "views": [{ "key": "filters", "category": "filters", "days": 30 }] })).unwrap()
    }

    fn row(orders: i64, last_order: Option<f64>, ltv: f64, last_cart: Option<f64>) -> Value {
        json!({ "orders": orders, "last_order_days": last_order, "ltv": ltv, "last_cart_days": last_cart,
                "touches": [["f1", "filters", 1.0], ["k1", "kaapi", 40.0]] })
    }

    #[test]
    fn facts_of_the_exemplar() {
        // Asha: two orders, 600 in all, last 41.5 days ago; a cart yesterday with no order after it.
        let f = Facts::from_row(&row(2, Some(41.5), 600.0, Some(1.2)), &spec()).unwrap();
        assert_eq!(f.days_since_last_purchase, Some(41.0));
        assert_eq!((f.purchase_count, f.ltv_bucket, f.cart_abandoned), (2, 2, true));
        let v = f.values(&spec());
        assert_eq!(v["feature:history.viewed.filters"], 1.0);
        assert_eq!(v["feature:history.lifetime_value"], 2.0 / 3.0);
        assert_eq!(v["feature:history.purchase_count"], 0.2);
    }

    #[test]
    fn a_cart_followed_by_an_order_or_inside_its_window_is_not_abandoned() {
        assert!(!Facts::from_row(&row(1, Some(0.5), 10.0, Some(1.2)), &spec()).unwrap().cart_abandoned, "ordered after the cart");
        assert!(!Facts::from_row(&row(0, None, 0.0, Some(0.5)), &spec()).unwrap().cart_abandoned, "still inside the 24h window");
        assert!(!Facts::from_row(&row(0, None, 0.0, Some(31.0)), &spec()).unwrap().cart_abandoned, "past the look-back");
        assert!(Facts::from_row(&row(0, None, 0.0, Some(2.0)), &spec()).unwrap().cart_abandoned, "never ordered");
    }

    #[test]
    fn ltv_buckets_follow_the_sites_edges() {
        let b = |ltv: f64, s: &Spec| Facts::from_row(&row(1, Some(1.0), ltv, None), s).unwrap().ltv_bucket;
        assert_eq!((b(99.0, &spec()), b(100.0, &spec()), b(1000.0, &spec())), (1, 2, 3));
        let mut s = spec();
        s.ltv_edges = Some([10.0, 50.0]);
        assert_eq!(b(60.0, &s), 3);
    }

    #[test]
    fn conditions_match_and_fail_closed() {
        let f = Facts::from_row(&row(2, Some(41.0), 600.0, Some(1.2)), &spec()).unwrap();
        let c = |k: &str, v: Value| Cond::parse(k, &v).unwrap();
        assert!(c("days_since_last_purchase", json!({ "gte": 30 })).matches(Some(&f)));
        assert!(!c("days_since_last_purchase", json!({ "gte": 42 })).matches(Some(&f)));
        assert!(!c("days_since_last_purchase", json!({ "lte": 40 })).matches(Some(&f)));
        assert!(c("purchase_count", json!({ "gte": 2, "lte": 2 })).matches(Some(&f)));
        assert!(c("lifetime_value", json!("mid")).matches(Some(&f)));
        assert!(!c("lifetime_value", json!("high")).matches(Some(&f)));
        assert!(c("cart_abandoned", json!(true)).matches(Some(&f)));
        assert!(c("viewed", json!({ "item": "f1", "days": 7 })).matches(Some(&f)));
        assert!(!c("viewed", json!({ "category": "kaapi", "days": 30 })).matches(Some(&f)), "40 days ago is outside 30");
        assert!(!c("cart_abandoned", json!(false)).matches(None), "absent facts match nothing");
        let never = Facts::from_row(&row(0, None, 0.0, None), &spec()).unwrap();
        assert!(!c("days_since_last_purchase", json!({ "gte": 30 })).matches(Some(&never)), "no last order is not 30+ days ago");
        for bad in [json!({ "gte": -1 }), json!({}), json!({ "gt": 3 }), json!(30)] {
            assert!(Cond::parse("days_since_last_purchase", &bad).is_err(), "{bad}");
        }
        assert!(Cond::parse("viewed", &json!({ "item": "a", "category": "b", "days": 3 })).is_err());
        assert!(Cond::parse("viewed", &json!({ "item": "a", "days": 91 })).is_err());
    }
}

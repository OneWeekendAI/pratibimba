//! Commerce events (F1): an order's value, currency and items on an event, and how its revenue is
//! shared among the decisions that may have caused it.
//!
//! **One order, one row.** An event that names an `order_id` gets its id from `(site, order id)`,
//! whatever id the client sent, so a reloaded thank-you page, a retried request, or the browser
//! and the server both reporting the order are absorbed by the `events` primary key.
//!
//! **Revenue is attributed per slot, whatever the slot's metric.** Each slot's touches are the
//! subject's exposed decisions in it whose attribution window contains the order; the slot's
//! attributor ([`crate::attribution`]) shares the order among them, every touch counting as open
//! (revenue is not one-per-decision: a second order in the window adds to the first). The shares
//! are applied in ten-thousandths with the remainder on the last share, so the revenue one order
//! hands a slot sums to its value exactly. An order in a currency other than the site's is stored
//! and reported (`other_currency_orders` in results) but not summed; a browser order for a goal
//! a platform is authoritative for is a hint, as for conversions, and takes no revenue.

use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Largest order value accepted (in the currency's major unit).
pub const MAX_VALUE: f64 = 1e12;
pub const MAX_ITEMS: usize = 100;
pub const MAX_ITEMS_BYTES: usize = 16_384;
/// Values are kept to four decimal places.
pub const SCALE: f64 = 10_000.0;

/// The event id for an order: the same `(site, order id)` always maps to the same id.
pub fn order_event_id(site: &str, order_id: &str) -> Uuid {
    let h = Sha256::new().chain_update(site).chain_update([0]).chain_update("order").chain_update([0]).chain_update(order_id).finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    uuid::Builder::from_custom_bytes(b).into_uuid()
}

/// ISO 4217 shape: three letters, stored upper-case.
pub fn currency(c: &str) -> Option<String> {
    (c.len() == 3 && c.bytes().all(|b| b.is_ascii_alphabetic())).then(|| c.to_ascii_uppercase())
}

pub fn valid_order_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && s.bytes().all(|b| (0x20..0x7f).contains(&b))
}

/// Check an event's commerce fields; returns the currency upper-cased.
pub fn check(order_id: Option<&str>, value: Option<f64>, cur: Option<&str>, items: Option<&Value>) -> Result<Option<String>, String> {
    if order_id.is_some_and(|o| !valid_order_id(o)) {
        return Err("order_id must be 1-256 printable ASCII characters".into());
    }
    let cur = match (value, cur) {
        (None, None) => None,
        (Some(v), Some(c)) => {
            if !v.is_finite() || !(0.0..=MAX_VALUE).contains(&v) {
                return Err(format!("value must be a number from 0 to {MAX_VALUE}"));
            }
            Some(currency(c).ok_or("currency must be a three-letter ISO 4217 code, e.g. USD")?)
        }
        _ => return Err("value and currency go together: send both or neither".into()),
    };
    if let Some(items) = items {
        let Some(list) = items.as_array() else { return Err("items must be an array of {id, price?, quantity?}".into()) };
        if list.len() > MAX_ITEMS || items.to_string().len() > MAX_ITEMS_BYTES {
            return Err(format!("items: at most {MAX_ITEMS} items and {MAX_ITEMS_BYTES} bytes"));
        }
        for it in list {
            let ok_id = it.get("id").and_then(Value::as_str).is_some_and(|s| !s.is_empty() && s.len() <= 256);
            let ok_price = it.get("price").is_none_or(|p| p.as_f64().is_some_and(|p| p.is_finite() && p >= 0.0));
            let ok_qty = it.get("quantity").is_none_or(|q| q.as_i64().is_some_and(|q| (1..=1_000_000).contains(&q)));
            if !it.is_object() || !ok_id || !ok_price || !ok_qty {
                return Err("each item needs an id (1-256 chars); price is a number >= 0, quantity a whole number >= 1".into());
            }
        }
    }
    Ok(cur)
}

/// A value in ten-thousandths of the major unit.
pub fn units(value: f64) -> i64 {
    (value * SCALE).round() as i64
}

/// Split `total` units by `shares` (summing to at most 1): each share rounded, the last taking
/// whatever makes the parts sum to the rounded whole. Exact: the parts sum to `total * sum(shares)`
/// rounded, never a unit more or less.
pub fn split(total: i64, shares: &[f64]) -> Vec<i64> {
    let whole = (total as f64 * shares.iter().sum::<f64>()).round() as i64;
    let mut out: Vec<i64> = shares.iter().map(|s| (total as f64 * s).round() as i64).collect();
    if let Some(last) = out.len().checked_sub(1) {
        let others: i64 = out[..last].iter().sum();
        out[last] = whole - others;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_sums_exactly() {
        let s = split(1_000_000, &[1.0 / 3.0; 3]);
        assert_eq!(s.iter().sum::<i64>(), 1_000_000);
        assert_eq!(s, vec![333_333, 333_333, 333_334]);
        assert_eq!(split(5, &[1.0]), vec![5]);
        assert!(split(5, &[]).is_empty());
    }

    #[test]
    fn order_ids_are_per_site() {
        assert_eq!(order_event_id("a", "1"), order_event_id("a", "1"));
        assert_ne!(order_event_id("a", "1"), order_event_id("b", "1"));
        assert_ne!(order_event_id("a", "1"), order_event_id("a", "2"));
    }
}

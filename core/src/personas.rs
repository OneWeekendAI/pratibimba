//! Preview personas (P1): a few made-up visitors the Preview room and the approval cards show a
//! templated version to, so an owner sees "Your usual rajma-chawal" before any real visitor does.
//!
//! Synthetic only: no real visitor id is ever accepted in their place (owner decision), so a
//! preview never reads anyone's history.

use crate::history::Facts;

pub struct Persona {
    pub key: &'static str,
    /// A short name for a chip.
    pub label: &'static str,
    /// One sentence on who this is, for a tooltip.
    pub about: &'static str,
    /// `None`: a visitor with no history at all.
    pub facts: Option<Facts>,
}

#[allow(clippy::too_many_arguments)]
fn facts(orders: i64, days: Option<f64>, bucket: u8, cart: bool, top: Option<(&str, i64)>, last: &[&str], visits: i64, viewed: &[&str]) -> Option<Facts> {
    Some(Facts {
        days_since_last_purchase: days,
        purchase_count: orders,
        ltv_bucket: bucket,
        cart_abandoned: cart,
        touches: viewed.iter().map(|v| (v.to_string(), None, 1.0)).collect(),
        top_item: top.map(|(k, n)| (k.to_string(), n)),
        last_order_items: last.iter().map(|s| s.to_string()).collect(),
        visit_count: visits,
    })
}

/// Every persona, in the order the room shows them; the first is a first visit.
pub fn all() -> Vec<Persona> {
    vec![
        Persona { key: "first_visit", label: "First visit", about: "No history at all", facts: None },
        Persona {
            key: "three_orders_40d",
            label: "3 orders",
            about: "Three orders, the last one 40 days ago; masala dosa twice",
            facts: facts(3, Some(40.0), 1, false, Some(("masala-dosa", 2)), &["masala-dosa", "filter-coffee"], 5, &[]),
        },
        Persona {
            key: "usual_rajma",
            label: "Regular",
            about: "Ten orders, nine of them rajma chawal; 11 visits",
            facts: facts(10, Some(2.0), 2, false, Some(("rajma-chawal", 9)), &["rajma-chawal"], 11, &[]),
        },
        Persona { key: "cart_yesterday", label: "Left a cart", about: "No orders; left paneer tikka in the cart yesterday", facts: facts(0, None, 0, true, None, &[], 2, &["paneer-tikka"]) },
    ]
}

/// The persona named `key`, if there is one.
pub fn find(key: &str) -> Option<Persona> {
    all().into_iter().find(|p| p.key == key)
}

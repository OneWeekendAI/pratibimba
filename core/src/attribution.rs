//! Attributors: how one conversion's credit is shared among the decisions that may have caused it.
//!
//! The worker gathers the **touches** of one slot: the subject's exposed decisions in that slot
//! whose attribution window contains the conversion, oldest first. A touch is **open** when it can
//! still take credit (unsettled, or a settled miss a platform conversion may correct). The
//! attributor turns the touches, and the decision the event named if any, into `(decision,
//! share)` pairs. The shares one conversion hands a slot sum to at most 1, which is what keeps
//! results consistent: one conversion counts once whichever attributor ran.
//!
//! The site file picks one per site (`attribution`) and may override it per slot.

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Touch {
    pub id: Uuid,
    pub open: bool,
}

pub trait Attributor: Send + Sync {
    fn name(&self) -> &'static str;
    /// `(decision, share)` pairs for one conversion in one slot. `touches` are oldest first.
    fn credit(&self, touches: &[Touch], named: Option<Uuid>) -> Vec<(Uuid, f64)>;
}

/// The default: the named decision, or else the latest touch, takes the whole conversion. A touch
/// that is no longer open simply fails to take it (the worker's guards decide), exactly as before
/// the attributor was pluggable.
pub struct LastTouch;

impl Attributor for LastTouch {
    fn name(&self) -> &'static str {
        "last_touch"
    }
    fn credit(&self, touches: &[Touch], named: Option<Uuid>) -> Vec<(Uuid, f64)> {
        match named {
            Some(n) => touches.iter().filter(|t| t.id == n).map(|t| (t.id, 1.0)).collect(),
            None => touches.last().map(|t| (t.id, 1.0)).into_iter().collect(),
        }
    }
}

/// Multi-touch: every open touch takes an equal share. A named decision only picks the slot (and
/// must itself be a touch); it gets no more than the others.
pub struct Linear;

impl Attributor for Linear {
    fn name(&self) -> &'static str {
        "linear"
    }
    fn credit(&self, touches: &[Touch], named: Option<Uuid>) -> Vec<(Uuid, f64)> {
        if named.is_some_and(|n| !touches.iter().any(|t| t.id == n)) {
            return vec![];
        }
        let open: Vec<&Touch> = touches.iter().filter(|t| t.open).collect();
        let share = 1.0 / open.len() as f64;
        open.into_iter().map(|t| (t.id, share)).collect()
    }
}

pub const DEFAULT: &str = "last_touch";
pub const NAMES: &[&str] = &["last_touch", "linear"];

pub fn by_name(name: &str) -> Option<&'static dyn Attributor> {
    match name {
        "last_touch" => Some(&LastTouch),
        "linear" => Some(&Linear),
        _ => None,
    }
}

//! The visitor's local clock as a context feature (backend.md, "Local time as a context signal").
//!
//! **Where the offset comes from.** The SDK reports `tz_offset` (minutes east of UTC, i.e.
//! `-new Date().getTimezoneOffset()`) with each event batch. The spec prefers IP geolocation,
//! but Core is self-hosted and MIT and ships no GeoIP database (MaxMind's needs a licence key
//! and a refresh job), and owners proxy `/_prb/*` through their own origin, so the client IP
//! Core sees is often the proxy's. The browser's offset is the only signal every install has.
//! It is coarse (one of ~40 values, not identifying), it is DST-correct at the moment of the
//! event, and spoofing it only mislabels the spoofer's own hour. An event with no offset falls
//! back to UTC and is counted out of `tz_known`, so a chart can say how much of it is a guess.
//!
//! **Server-side SDKs.** `@pratibimba/next` and `@pratibimba/edge` render on a server with no
//! browser clock. They send `ctx.tz`, an IANA zone name, taken from the edge's IP geolocation
//! header (Vercel's `x-vercel-ip-timezone`, Cloudflare's `request.cf.timezone` or `cf-timezone`,
//! CloudFront's `cloudfront-viewer-time-zone`) or, failing that, from a first-party cookie the
//! browser half sets. Core resolves the zone at the decision's own clock ([`zone_offset`]), and
//! a named zone is preferred over a raw offset when both are sent.

use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use serde::Serialize;

/// Valid offsets: UTC-12:00 to UTC+14:00.
pub const MIN_OFFSET: i16 = -720;
pub const MAX_OFFSET: i16 = 840;

pub fn valid_offset(m: i16) -> bool {
    (MIN_OFFSET..=MAX_OFFSET).contains(&m)
}

/// The longest IANA zone name accepted (the longest real one is 32 characters).
pub const MAX_ZONE_LEN: usize = 64;

/// A named IANA time zone (`Asia/Kolkata`), or `None` for a name the tz database lacks.
pub fn parse_zone(name: &str) -> Option<chrono_tz::Tz> {
    if name.is_empty() || name.len() > MAX_ZONE_LEN {
        return None;
    }
    name.parse().ok()
}

/// A zone's offset (minutes east of UTC) at the instant `at`: daylight saving is the zone's
/// rule at that instant, so a sandbox's simulated July is BST even when the real clock says GMT.
pub fn zone_offset(zone: chrono_tz::Tz, at: DateTime<Utc>) -> i16 {
    use chrono::{Offset, TimeZone};
    (zone.offset_from_utc_datetime(&at.naive_utc()).fix().local_minus_utc() / 60) as i16
}

/// The site's night window, `[start, end)` in local hours; wraps midnight when start > end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NightWindow {
    pub start: u8,
    pub end: u8,
}

impl Default for NightWindow {
    fn default() -> Self {
        NightWindow { start: 22, end: 5 }
    }
}

impl NightWindow {
    pub fn contains(&self, hour: u8) -> bool {
        if self.start <= self.end { hour >= self.start && hour < self.end } else { hour >= self.start || hour < self.end }
    }
}

/// The feature set, named as the decision detail shows it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LocalTime {
    pub hour_local: u8,
    /// 0 = Sunday.
    pub dow_local: u8,
    /// Cyclic encoding, so 23:00 sits next to 00:00 rather than at the far end of the scale.
    pub sin_hour: f64,
    pub cos_hour: f64,
    pub is_weekend_local: bool,
    pub is_night_local: bool,
    /// Monday-Friday, 09:00-17:00 local.
    pub is_business_hours_local: bool,
}

impl LocalTime {
    pub fn at(utc: DateTime<Utc>, offset_min: Option<i16>, night: NightWindow) -> LocalTime {
        let off = offset_min.filter(|m| valid_offset(*m)).unwrap_or(0);
        let local = utc + Duration::minutes(off as i64);
        Self::from_parts(local.hour() as u8, local.weekday().num_days_from_sunday() as u8, night)
    }

    /// The features of a local hour and weekday (what a rollup row keeps).
    pub fn from_parts(hour_local: u8, dow_local: u8, night: NightWindow) -> LocalTime {
        let angle = 2.0 * std::f64::consts::PI * hour_local as f64 / 24.0;
        let weekend = dow_local == 0 || dow_local == 6;
        LocalTime {
            hour_local,
            dow_local,
            sin_hour: angle.sin(),
            cos_hour: angle.cos(),
            is_weekend_local: weekend,
            is_night_local: night.contains(hour_local),
            is_business_hours_local: !weekend && (9..17).contains(&hour_local),
        }
    }
}

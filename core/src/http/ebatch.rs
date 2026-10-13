//! `POST /e/batch`: what the edge Worker's queue consumer drains, in one call (edge-cache.md,
//! EC3). A batch holds items of two kinds:
//!
//! - `{"kind": "events", "received_at", "request"}`: a browser's `/e` request body, as the Worker
//!   received it, and when. Picks and rescores never come this way (the Worker sends them to
//!   `/e` directly, because the next decide reads them); a batch refuses them.
//! - `{"kind": "decisions", "request"}`: an edge decision record, the body `/edge/decisions`
//!   takes, checked and stored exactly as there ([`super::edge::ingest`]).
//!
//! The Worker presents `PRB_EDGE_TOKEN`; with none configured the route answers 404. The answer
//! is per id: `accepted` (written now), `duplicate` (already written: the queue delivers at
//! least once, and every id is a client-made UUID with `ON CONFLICT DO NOTHING`), or `refused`
//! with a reason (never written, and retrying will not change that). Only a failure of Core
//! itself is a non-2xx, so the consumer retries exactly what Core did not take.
//!
//! **What stays as `/e` does it.** Each event is stored under the subject Core would store it
//! under (the raw id, or the pseudonym of the day the edge received it, so it joins the decision
//! it names), with only the fields the site collects, and with the time the edge received it
//! (held to Core's clock if it is ahead), so the attribution window sees the click when it
//! happened, not when the queue delivered it. **The tombstone is checked per subject**: every
//! subject's shared erase lock is taken first, then the tombstones are read in a separate
//! statement (READ COMMITTED takes a statement's snapshot before an in-statement lock waits), so
//! an erase either finishes first and its subject's events are refused (`erased`), or waits for
//! the batch and then deletes what it wrote.
//!
//! **Why it is cheaper than `/e`.** All the events of a batch are written in one transaction: one
//! statement for the locks, one for the tombstones, one insert for every event and its outbox
//! row, one for the visitor summaries, one NOTIFY. `/e` spends a transaction of about eight
//! statements on every request.

use super::events::{EventIn, EventsRequest, MAX_EVENTS};
use super::{ApiError, AppState, edge, valid_subject};
use crate::{privacy, site_file::is_event_name};
use axum::{Json, extract::State, http::HeaderMap};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// Most items in one batch (a Cloudflare Queue batch is at most 100 messages).
pub const MAX_ITEMS: usize = 100;
/// How old a queued decision may be: a day covers every retry and a dead-letter replay.
pub const MAX_DECISION_AGE_SECS: i64 = 86_400;

#[derive(Default)]
struct Answer {
    accepted: Vec<Uuid>,
    duplicate: Vec<Uuid>,
    refused: Vec<Value>,
}

impl Answer {
    fn refuse(&mut self, id: Option<Uuid>, reason: impl Into<String>) {
        self.refused.push(json!({ "id": id, "reason": reason.into() }));
    }

    /// Refuse a whole item: each id it carries, or the item itself when none can be read.
    fn refuse_item(&mut self, item: &Value, list: &str, reason: &str) {
        let ids: Vec<Uuid> = item["request"][list]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()))
            .collect();
        if ids.is_empty() {
            self.refuse(None, reason);
        }
        for id in ids {
            self.refuse(Some(id), reason);
        }
    }
}

pub async fn handler(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    if !state.edge.has_token() {
        return Err(ApiError::NotFound("the event batch is off (no PRB_EDGE_TOKEN)".into()));
    }
    let presented = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    if !state.edge.token_matches(presented) {
        return Err(ApiError::Unauthorized);
    }
    let Some(items) = body["items"].as_array().filter(|i| !i.is_empty() && i.len() <= MAX_ITEMS) else {
        return Err(ApiError::BadRequest(format!("send {{\"items\": [...]}} with 1-{MAX_ITEMS} items")));
    };
    let mut answer = Answer::default();
    let mut events = vec![];
    // Decisions first: an event in this batch may name one.
    for item in items {
        match item["kind"].as_str() {
            Some("decisions") => decisions(&state, item, &mut answer).await?,
            Some("events") => {
                let at = item["received_at"].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|t| t.with_timezone(&Utc));
                match (at, serde_json::from_value::<EventsRequest>(item["request"].clone())) {
                    (Some(at), Ok(req)) => events.push((at, req)),
                    (None, _) => answer.refuse_item(item, "events", "received_at is not an RFC 3339 time"),
                    (_, Err(e)) => answer.refuse_item(item, "events", &format!("not an /e request: {e}")),
                }
            }
            _ => answer.refuse(None, format!("unknown item kind {:?}", item["kind"])),
        }
    }
    write_events(&state, events, &mut answer).await?;
    Ok(Json(json!({ "accepted": answer.accepted, "duplicate": answer.duplicate, "refused": answer.refused })))
}

async fn decisions(state: &AppState, item: &Value, answer: &mut Answer) -> Result<(), ApiError> {
    let req: edge::Ingest = match serde_json::from_value(item["request"].clone()) {
        Ok(r) => r,
        Err(e) => {
            answer.refuse_item(item, "decisions", &format!("not an edge decision record: {e}"));
            return Ok(());
        }
    };
    if !valid_subject(&req.subject) || req.decisions.is_empty() || req.decisions.len() > edge::MAX_DECISIONS {
        answer.refuse_item(item, "decisions", "a record carries a valid subject and 1-20 decisions");
        return Ok(());
    }
    let Some(outcomes) = edge::ingest(state, &req, MAX_DECISION_AGE_SECS).await? else {
        answer.refuse_item(item, "decisions", &format!("unknown site {:?}", req.site));
        return Ok(());
    };
    for (id, o) in outcomes {
        match o {
            edge::Outcome::Stored => answer.accepted.push(id),
            edge::Outcome::Duplicate => answer.duplicate.push(id),
            edge::Outcome::Gone => answer.refuse(Some(id), "erased"),
            edge::Outcome::Rejected(r) => answer.refuse(Some(id), r),
        }
    }
    Ok(())
}

/// A site as the batch needs it: what `/e` reads per request, read once per batch.
struct Site {
    policy: privacy::Policy,
    sim: bool,
    slots: HashMap<String, i64>,
}

/// One event ready to write.
struct Row {
    id: Uuid,
    site: String,
    raw: String,
    stored: String,
    name: String,
    slot: Option<i64>,
    decision: Option<Uuid>,
    at: DateTime<Utc>,
    tz: Option<i16>,
}

fn is_special(e: &EventIn) -> bool {
    e.name == "pick" || e.name == "rescore" || e.picks.is_some() || e.variant.is_some() || e.features.is_some() || e.score.is_some()
}

async fn write_events(state: &AppState, items: Vec<(DateTime<Utc>, EventsRequest)>, answer: &mut Answer) -> Result<(), ApiError> {
    let mut sites: HashMap<String, Option<Site>> = HashMap::new();
    let mut salts: HashMap<(String, NaiveDate), String> = HashMap::new();
    let mut rows: Vec<Row> = vec![];
    let mut seen: HashSet<Uuid> = HashSet::new();
    for (received, req) in items {
        let ids: Vec<Uuid> = req.events.iter().map(|e| e.id).collect();
        let refuse_all = |answer: &mut Answer, why: &str| ids.iter().for_each(|id| answer.refuse(Some(*id), why));
        if !valid_subject(&req.subject) {
            refuse_all(answer, "subject must be 1-256 printable ASCII characters");
            continue;
        }
        if req.events.is_empty() || req.events.len() > MAX_EVENTS {
            refuse_all(answer, &format!("send 1-{MAX_EVENTS} events per request"));
            continue;
        }
        if req.events.iter().any(|e| !is_event_name(&e.name)) {
            refuse_all(answer, "event name must match [a-z0-9_:]{1,64}");
            continue;
        }
        if req.tz_offset.is_some_and(|m| !crate::insights::localtime::valid_offset(m)) {
            refuse_all(answer, "tz_offset is minutes east of UTC, -720 to 840");
            continue;
        }
        if !sites.contains_key(&req.site) {
            let s = load_site(state, &req.site).await?;
            sites.insert(req.site.clone(), s);
        }
        let Some(site) = sites[&req.site].as_ref() else {
            refuse_all(answer, &format!("unknown site {:?}", req.site));
            continue;
        };
        // The edge's clock, held to Core's; a sandbox runs on its own clock, as at `/e`.
        let at = if site.sim { site.policy.at } else { received.min(site.policy.at) };
        let identified = site.policy.identified(req.consent);
        let stored = if identified {
            req.subject.clone()
        } else {
            let key = (req.site.clone(), privacy::day_of(at));
            if !salts.contains_key(&key) {
                let s = privacy::salt(&state.pool, &req.site, key.1).await?;
                salts.insert(key.clone(), s);
            }
            privacy::pseudonym(&salts[&key], &req.site, &req.subject)
        };
        let tz = site.policy.tz(req.tz_offset);
        let (special, plain): (Vec<EventIn>, Vec<EventIn>) = req.events.into_iter().partition(is_special);
        for e in special {
            answer.refuse(Some(e.id), "picks and rescores go to /e: the next decide reads them");
        }
        let kept = site.policy.events(plain);
        for e in kept {
            if !seen.insert(e.id) {
                answer.duplicate.push(e.id);
                continue;
            }
            rows.push(Row {
                id: e.id,
                site: req.site.clone(),
                raw: req.subject.clone(),
                stored: stored.clone(),
                slot: e.slot.as_ref().and_then(|k| site.slots.get(k)).copied(),
                name: e.name,
                decision: e.decision_id,
                at,
                tz,
            });
        }
    }
    if rows.is_empty() {
        return Ok(());
    }

    let mut tx = state.pool.begin().await?;
    // Every subject's shared erase lock, in one order; then, in a later statement, the tombstones.
    let pairs: BTreeSet<(String, String)> = rows.iter().map(|r| (r.site.clone(), r.raw.clone())).collect();
    let mut keys: Vec<i64> = pairs.iter().map(|(s, r)| privacy::lock_key(s, r)).collect();
    keys.sort_unstable();
    keys.dedup();
    sqlx::query("SELECT count(pg_advisory_xact_lock_shared(k)) FROM (SELECT unnest($1::bigint[]) AS k ORDER BY 1) l").bind(&keys).execute(&mut *tx).await?;
    let hash_sites: Vec<&str> = pairs.iter().map(|p| p.0.as_str()).collect();
    let hashes: Vec<String> = pairs.iter().map(|(s, r)| privacy::subject_hash(s, r)).collect();
    let erased: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT e.site_id, e.subject_hash FROM erasures e
           JOIN unnest($1::text[], $2::text[]) AS t(s, h) ON e.site_id = t.s AND e.subject_hash = t.h",
    )
    .bind(&hash_sites)
    .bind(&hashes)
    .fetch_all(&mut *tx)
    .await?;
    let erased: HashSet<(String, String)> = erased.into_iter().collect();
    let (gone, rows): (Vec<Row>, Vec<Row>) = rows.into_iter().partition(|r| erased.contains(&(r.site.clone(), privacy::subject_hash(&r.site, &r.raw))));
    for r in gone {
        answer.refuse(Some(r.id), "erased");
    }
    if rows.is_empty() {
        tx.commit().await?;
        return Ok(());
    }

    let inserted: Vec<Uuid> = sqlx::query_scalar(
        "WITH ins AS (
             INSERT INTO events (id, site_id, subject, name, slot_id, decision_id, received_at, tz_offset_min)
             SELECT * FROM unnest($1::uuid[], $2::text[], $3::text[], $4::text[], $5::bigint[], $6::uuid[], $7::timestamptz[], $8::smallint[])
             ON CONFLICT (id) DO NOTHING
             RETURNING id),
              outbox AS (INSERT INTO pending_events (event_id) SELECT id FROM ins)
         SELECT id FROM ins",
    )
    .bind(rows.iter().map(|r| r.id).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.site.as_str()).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.stored.as_str()).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.slot).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.decision).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.at).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.tz).collect::<Vec<_>>())
    .fetch_all(&mut *tx)
    .await?;
    let inserted: HashSet<Uuid> = inserted.into_iter().collect();
    // The visitor summaries count only what was new, in the order it was sent.
    let mut visits: Vec<(String, String, Vec<String>, DateTime<Utc>)> = vec![];
    for r in &rows {
        if !inserted.contains(&r.id) {
            answer.duplicate.push(r.id);
            continue;
        }
        answer.accepted.push(r.id);
        match visits.iter_mut().find(|v| v.0 == r.site && v.1 == r.stored) {
            Some(v) => {
                v.2.push(r.name.clone());
                v.3 = v.3.max(r.at);
            }
            None => visits.push((r.site.clone(), r.stored.clone(), vec![r.name.clone()], r.at)),
        }
    }
    privacy::record_visits(&mut tx, &visits).await?;
    if !inserted.is_empty() {
        sqlx::query("SELECT pg_notify($1, '')").bind(super::events::CHANNEL).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn load_site(state: &AppState, site: &str) -> Result<Option<Site>, ApiError> {
    let row: Option<(String, Option<Vec<String>>, DateTime<Utc>, bool)> =
        sqlx::query_as("SELECT privacy_mode, fields_allowed, COALESCE(sim_now, now()), sim_now IS NOT NULL FROM sites WHERE id = $1")
            .bind(site)
            .fetch_optional(&state.pool)
            .await?;
    let Some((mode, fields, at, sim)) = row else { return Ok(None) };
    let slots: Vec<(String, i64)> = sqlx::query_as("SELECT key, id FROM slots WHERE site_id = $1").bind(site).fetch_all(&state.pool).await?;
    Ok(Some(Site { policy: privacy::Policy { mode, fields, at }, sim, slots: slots.into_iter().collect() }))
}

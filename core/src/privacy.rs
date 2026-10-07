//! Privacy (backend.md, "Privacy"): consent modes, minimum collection, the visitor summary and
//! right-to-erase.
//!
//! - **Consent modes**, per site. `full` stores the subject exactly as the client sent it.
//!   `cookieless` never stores it: the stored subject is a pseudonym, sha256 of the site's salt
//!   for the current UTC day, the site and the client's id. The salt is deleted when the next
//!   day's is made, so a visitor's days can never be joined, and `returning` (which only a
//!   persistent cookie can know) is dropped. `consent_aware` is `full` for a request whose
//!   `consent` is `true` and `cookieless` for every other request.
//! - **Minimum collection.** `fields_allowed` names the optional request fields a site
//!   collects ([`FIELDS`]). Anything else is dropped at ingest, before it is written. A `pick`
//!   without its picks, or a `rescore` without its parts, is dropped whole. Unknown names are
//!   still refused at the boundary (`deny_unknown_fields`), whatever the list says.
//! - **Visitors.** One row per stored subject: first and last seen, an event count and the last
//!   20 event names, written in the ingest commit.
//! - **Erase.** One call tombstones the subject (only its hash is kept), schedules its events
//!   for removal, deletes its visitor row and retention weeks, scrubs it from webhook payloads, and severs its decisions:
//!   they keep their variant, features and outcome (so results, rollups and replays still add
//!   up) under one random token that nothing links back to the subject. An open decision is
//!   settled with no outcome, so it teaches the model nothing: the visitor's later events are
//!   refused, so a miss would be a guess. What the models already learned stays: a bandit's
//!   state is an aggregate, and no single visitor can be read out of it.
//! - **Scheduled removal.** The events are not deleted in the erase: every stored form of the
//!   subject goes into `erased_subjects`, which hides its events from `live_events`, the view
//!   every read path uses, in the same commit. The worker deletes them once `purge_after`
//!   (the erase time plus [`purge_delay`], a day by default) has passed, then forgets the forms.
//!   The erasure row stays as the tombstone; `erasures` shows the removal pending or done.
//! - **Tombstones.** Every write path (decide, events, platform conversions) takes a shared
//!   advisory lock on (site, subject) and checks the tombstone inside its transaction; erase
//!   takes the same lock exclusively. An in-flight write therefore either lands before the
//!   erase (and is erased) or sees the tombstone (and is refused).

use crate::http::decide::Ctx;
use crate::http::events::EventIn;
use crate::http::{ApiError, valid_subject};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};

pub const MODES: [&str; 3] = ["cookieless", "consent_aware", "full"];
/// The optional fields `fields_allowed` may name. `tz_offset` covers the visitor's clock in every
/// form: the decide context's offset and named zone (`tz`), and the event batch's offset; `rescore` covers a rescore event's variant, features and score.
pub const FIELDS: [&str; 9] = ["returning", "mobile", "from_ad", "stated", "tz_offset", "slot", "decision_id", "picks", "rescore"];
/// The prefix of the token an erased subject's decisions are filed under.
pub const ERASED: &str = "erased:";
/// The prefix of a cookieless pseudonym.
pub const PSEUDONYM: &str = "p1_";
/// How many event names a visitor row keeps.
pub const RECENT: usize = 20;

/// A site's privacy settings, read once per request.
#[derive(Debug, Clone)]
pub struct Policy {
    pub mode: String,
    pub fields: Option<Vec<String>>,
    /// The site's clock (a sandbox's simulated time).
    pub at: DateTime<Utc>,
}

impl Policy {
    /// Whether this request may be tied to the client's own id.
    pub fn identified(&self, consent: Option<bool>) -> bool {
        match self.mode.as_str() {
            "full" => true,
            "consent_aware" => consent == Some(true),
            _ => false,
        }
    }

    pub fn allows(&self, field: &str) -> bool {
        self.fields.as_ref().is_none_or(|f| f.iter().any(|x| x == field))
    }

    /// The decide context with every field this site does not collect dropped.
    pub fn ctx(&self, ctx: &Ctx, identified: bool) -> Ctx {
        Ctx {
            returning: ctx.returning && identified && self.allows("returning"),
            mobile: ctx.mobile && self.allows("mobile"),
            from_ad: ctx.from_ad && self.allows("from_ad"),
            stated: if self.allows("stated") { ctx.stated.clone() } else { vec![] },
            tz_offset: ctx.tz_offset.filter(|_| self.allows("tz_offset")),
            tz: ctx.tz.clone().filter(|_| self.allows("tz_offset")),
            consent: ctx.consent,
            // Declared features are filtered in decide, which knows which are PII.
            features: ctx.features.clone(),
        }
    }

    pub fn tz(&self, tz: Option<i16>) -> Option<i16> {
        tz.filter(|_| self.allows("tz_offset"))
    }

    /// The events with every field this site does not collect dropped. A pick or rescore that
    /// lost a part it cannot work without is dropped whole; one that arrived malformed is left
    /// for the ingest checks to refuse.
    pub fn events(&self, events: Vec<EventIn>) -> Vec<EventIn> {
        events
            .into_iter()
            .filter_map(|mut e| {
                let (mut slot, mut decision, mut picks, mut rescore) = (false, false, false, false);
                if !self.allows("slot") {
                    slot = e.slot.take().is_some();
                }
                if !self.allows("decision_id") {
                    decision = e.decision_id.take().is_some();
                }
                if !self.allows("picks") {
                    picks = e.picks.take().is_some();
                }
                if !self.allows("rescore") {
                    rescore = e.variant.take().is_some() | e.features.take().is_some() | e.score.take().is_some();
                }
                let gutted = match e.name.as_str() {
                    "pick" => slot || picks,
                    "rescore" => slot || decision || rescore,
                    _ => false,
                };
                (!gutted).then_some(e)
            })
            .collect()
    }
}

/// Check a mode and a field list before they are stored.
pub fn validate(mode: Option<&str>, fields: Option<&[String]>) -> Result<(), String> {
    if let Some(m) = mode.filter(|m| !MODES.contains(m)) {
        return Err(format!("privacy_mode {m:?} must be one of {}", MODES.join(", ")));
    }
    if let Some(f) = fields.into_iter().flatten().find(|f| !FIELDS.contains(&f.as_str())) {
        return Err(format!("fields_allowed: {f:?} is not a field; the fields are {}", FIELDS.join(", ")));
    }
    Ok(())
}

/// The site's policy; `None` for an unknown site.
pub async fn policy(pool: &PgPool, site: &str) -> Result<Option<Policy>, ApiError> {
    let row: Option<(String, Option<Vec<String>>, DateTime<Utc>)> =
        sqlx::query_as("SELECT privacy_mode, fields_allowed, COALESCE(sim_now, now()) FROM sites WHERE id = $1").bind(site).fetch_optional(pool).await?;
    Ok(row.map(|(mode, fields, at)| Policy { mode, fields, at }))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// What a tombstone keeps: sha256(site, 0, subject).
pub fn subject_hash(site: &str, subject: &str) -> String {
    hex(&Sha256::new().chain_update(site).chain_update([0]).chain_update(subject).finalize())
}

pub fn pseudonym(salt: &str, site: &str, subject: &str) -> String {
    let h = Sha256::new().chain_update(salt).chain_update([0]).chain_update(site).chain_update([0]).chain_update(subject).finalize();
    format!("{PSEUDONYM}{}", &hex(&h)[..32])
}

pub fn day_of(at: DateTime<Utc>) -> NaiveDate {
    at.date_naive()
}

/// The site's salt for `day`, made on first use. Making a day's salt deletes every older one.
/// A request still on a day whose salt is gone (it straddled midnight) gets the newest salt.
pub async fn salt(pool: &PgPool, site: &str, day: NaiveDate) -> Result<String, ApiError> {
    for _ in 0..3 {
        let fresh = hex(&rand::random::<[u8; 32]>());
        let got: Option<String> = sqlx::query_scalar(
            "WITH old AS (DELETE FROM privacy_salts WHERE site_id = $1 AND day < $2),
                  ins AS (INSERT INTO privacy_salts (site_id, day, salt)
                          SELECT $1, $2, $3 WHERE NOT EXISTS (SELECT 1 FROM privacy_salts WHERE site_id = $1 AND day > $2)
                          ON CONFLICT DO NOTHING RETURNING salt)
             SELECT salt FROM ins
             UNION ALL (SELECT salt FROM privacy_salts WHERE site_id = $1 AND day >= $2 ORDER BY day LIMIT 1)
             LIMIT 1",
        )
        .bind(site)
        .bind(day)
        .bind(&fresh)
        .fetch_optional(pool)
        .await?;
        // `None` only when another request made the salt in this instant: read it again.
        if let Some(s) = got {
            return Ok(s);
        }
    }
    Err(ApiError::Internal(anyhow::anyhow!("could not read the privacy salt for {site:?}")))
}

/// The subject as stored: the client's id when the request is identified, else the day's
/// pseudonym of it.
pub async fn stored_subject(pool: &PgPool, policy: &Policy, site: &str, subject: &str, identified: bool) -> Result<String, ApiError> {
    if identified {
        return Ok(subject.to_string());
    }
    let s = salt(pool, site, day_of(policy.at)).await?;
    Ok(pseudonym(&s, site, subject))
}

/// A platform conversion carries the client's id but says nothing about consent. `full`: the id;
/// `cookieless`: today's pseudonym; `consent_aware`: the id when the visitor is already known
/// by it (they consented), else today's pseudonym.
pub async fn platform_subject(pool: &PgPool, site: &str, subject: &str) -> Result<String, ApiError> {
    let Some(p) = policy(pool, site).await? else { return Err(ApiError::NotFound(format!("unknown site {site:?}"))) };
    let identified = match p.mode.as_str() {
        "full" => true,
        "consent_aware" => {
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM visitors WHERE site_id = $1 AND subject = $2)").bind(site).bind(subject).fetch_one(pool).await?
        }
        _ => false,
    };
    stored_subject(pool, &p, site, subject, identified).await
}

fn lock_key(site: &str, subject: &str) -> i64 {
    crate::hash::fnv1a64(&[b"prb-erase", site.as_bytes(), subject.as_bytes()]) as i64
}

/// Whether the subject was erased, under the shared lock an erase waits for. Call it inside the
/// transaction that writes for the subject; `subject` is the client's id, before any pseudonym.
pub async fn erased(tx: &mut Transaction<'_, Postgres>, site: &str, subject: &str) -> Result<bool, sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock_shared($1)").bind(lock_key(site, subject)).execute(&mut **tx).await?;
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM erasures WHERE site_id = $1 AND subject_hash = $2)")
        .bind(site)
        .bind(subject_hash(site, subject))
        .fetch_one(&mut **tx)
        .await
}

/// [`erased`] as a refusal.
pub async fn admit(tx: &mut Transaction<'_, Postgres>, site: &str, subject: &str) -> Result<(), ApiError> {
    if erased(tx, site, subject).await? {
        return Err(ApiError::Gone("this subject was erased; start a new one".into()));
    }
    Ok(())
}

/// Fold accepted events into the visitor's summary row.
pub async fn record_visit(tx: &mut Transaction<'_, Postgres>, site: &str, subject: &str, names: &[String], at: DateTime<Utc>) -> Result<(), sqlx::Error> {
    if names.is_empty() {
        return Ok(());
    }
    let tail: Vec<&String> = names.iter().skip(names.len().saturating_sub(RECENT)).collect();
    sqlx::query(
        "INSERT INTO visitors AS v (site_id, subject, first_seen, last_seen, events, recent) VALUES ($1, $2, $3, $3, $4, $5)
         ON CONFLICT (site_id, subject) DO UPDATE SET
             last_seen = GREATEST(v.last_seen, EXCLUDED.last_seen),
             events = v.events + EXCLUDED.events,
             recent = (v.recent || EXCLUDED.recent)[GREATEST(1, cardinality(v.recent || EXCLUDED.recent) - $6 + 1):]",
    )
    .bind(site)
    .bind(subject)
    .bind(at)
    .bind(names.len() as i64)
    .bind(&tail)
    .bind(RECENT as i32)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Every form the subject may be stored under: the id itself, and its pseudonym under each salt
/// the site still holds (normally only today's). Older pseudonyms are unlinkable by design.
async fn stored_forms(tx: &mut Transaction<'_, Postgres>, site: &str, subject: &str) -> Result<Vec<String>, sqlx::Error> {
    let salts: Vec<String> = sqlx::query_scalar("SELECT salt FROM privacy_salts WHERE site_id = $1 ORDER BY day").bind(site).fetch_all(&mut **tx).await?;
    Ok(std::iter::once(subject.to_string()).chain(salts.iter().map(|s| pseudonym(s, site, subject))).collect())
}

/// How long an erased subject's events wait, hidden, before the worker deletes them.
pub const DEFAULT_PURGE_DELAY_SECS: i64 = 86_400;
/// The longest delay accepted: a month, GDPR's outer bound for acting on a request (Art. 12(3)).
pub const MAX_PURGE_DELAY_SECS: i64 = 30 * 86_400;
pub const PURGE_DELAY_ENV: &str = "PRB_ERASE_PURGE_AFTER_SECS";

/// The purge delay from its setting (seconds): a day when unset, at most thirty days.
pub fn purge_delay_from(v: Option<&str>) -> Result<chrono::Duration, String> {
    let Some(v) = v.map(str::trim).filter(|v| !v.is_empty()) else { return Ok(chrono::Duration::seconds(DEFAULT_PURGE_DELAY_SECS)) };
    match v.parse::<i64>() {
        Ok(s) if (0..=MAX_PURGE_DELAY_SECS).contains(&s) => Ok(chrono::Duration::seconds(s)),
        _ => Err(format!("{PURGE_DELAY_ENV} must be whole seconds between 0 and {MAX_PURGE_DELAY_SECS}, not {v:?}")),
    }
}

/// [`purge_delay_from`] the environment.
pub fn purge_delay() -> Result<chrono::Duration, String> {
    purge_delay_from(std::env::var(PURGE_DELAY_ENV).ok().as_deref())
}

/// Right-to-erase with the configured [`purge_delay`]. Returns the audit row.
pub async fn erase(pool: &PgPool, site: &str, subject: &str, actor: &str, reason: Option<&str>) -> Result<Value, ApiError> {
    let delay = purge_delay().map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
    erase_after(pool, site, subject, actor, reason, delay).await
}

/// Right-to-erase whose events are removed `delay` after it. Returns the audit row.
pub async fn erase_after(pool: &PgPool, site: &str, subject: &str, actor: &str, reason: Option<&str>, delay: chrono::Duration) -> Result<Value, ApiError> {
    if !valid_subject(subject) {
        return Err(ApiError::BadRequest("subject must be 1-256 printable ASCII characters".into()));
    }
    let actor = actor.trim();
    if actor.is_empty() || actor.chars().count() > 128 {
        return Err(ApiError::BadRequest("an erasure needs a named actor (1-128 characters)".into()));
    }
    if reason.is_some_and(|r| r.chars().count() > 2000) {
        return Err(ApiError::BadRequest("reason is at most 2000 characters".into()));
    }
    let mut tx = pool.begin().await?;
    let at: Option<DateTime<Utc>> = sqlx::query_scalar("SELECT COALESCE(sim_now, now()) FROM sites WHERE id = $1").bind(site).fetch_optional(&mut *tx).await?;
    let Some(at) = at else { return Err(ApiError::NotFound(format!("unknown site {site:?}"))) };
    sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(lock_key(site, subject)).execute(&mut *tx).await?;
    let forms = stored_forms(&mut tx, site, subject).await?;
    let token = format!("{ERASED}{}", uuid::Uuid::new_v4().simple());

    // The rollups read a decision's offset from its events: keep it before they go.
    sqlx::query(
        "UPDATE decisions d SET tz_offset_min = COALESCE(d.tz_offset_min,
                (SELECT e.tz_offset_min FROM live_events e WHERE e.decision_id = d.id AND e.site_id = d.site_id AND e.tz_offset_min IS NOT NULL
                  ORDER BY (e.name <> 'exposure'), e.received_at LIMIT 1))
          WHERE d.site_id = $1 AND d.subject = ANY($2)",
    )
    .bind(site)
    .bind(&forms)
    .execute(&mut *tx)
    .await?;
    let settled = sqlx::query("UPDATE decisions SET settled_at = $3 WHERE site_id = $1 AND subject = ANY($2) AND settled_at IS NULL")
        .bind(site)
        .bind(&forms)
        .bind(at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let decisions = sqlx::query("UPDATE decisions SET subject = $3 WHERE site_id = $1 AND subject = ANY($2)")
        .bind(site)
        .bind(&forms)
        .bind(&token)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    // Scheduled, not deleted: counted here, hidden below, removed by the worker (`purge_due`).
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM live_events WHERE site_id = $1 AND subject = ANY($2)")
        .bind(site)
        .bind(&forms)
        .fetch_one(&mut *tx)
        .await?;
    let visitors = sqlx::query("DELETE FROM visitors WHERE site_id = $1 AND subject = ANY($2)").bind(site).bind(&forms).execute(&mut *tx).await?.rows_affected();
    let weeks = sqlx::query("DELETE FROM insight_visitor_weeks WHERE site_id = $1 AND subject = ANY($2)")
        .bind(site)
        .bind(&forms)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let deliveries = sqlx::query(
        "UPDATE webhook_deliveries w SET payload = jsonb_set(w.payload, '{data,subject}', 'null')
           FROM webhook_subscriptions s
          WHERE s.id = w.subscription_id AND s.site_id = $1 AND w.payload->'data'->>'subject' = ANY($2)",
    )
    .bind(site)
    .bind(&forms)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let counts = json!({ "events": events, "decisions": decisions, "settled": settled, "visitors": visitors,
                         "visitor_weeks": weeks, "webhook_deliveries": deliveries });
    let (id, created_at, due): (i64, DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO erasures (site_id, subject_hash, actor, reason, counts, created_at, purge_after) VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, created_at, purge_after",
    )
    .bind(site)
    .bind(subject_hash(site, subject))
    .bind(actor)
    .bind(reason)
    .bind(&counts)
    .bind(at)
    .bind(at + delay)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO erased_subjects (erasure_id, site_id, subject) SELECT $1, $2, f FROM UNNEST($3::text[]) f ON CONFLICT DO NOTHING")
        .bind(id)
        .bind(site)
        .bind(&forms)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(json!({ "id": id, "site": site, "subject_hash": subject_hash(site, subject), "actor": actor, "reason": reason,
               "counts": counts, "created_at": created_at, "events_removal": removal(&counts, due, None) }))
}

/// Whether an erasure's events are still waiting for the worker, and when they are (or were) due.
fn removal(counts: &Value, due: DateTime<Utc>, done: Option<DateTime<Utc>>) -> Value {
    json!({ "status": if done.is_some() { "done" } else { "pending" }, "due": due, "done_at": done, "events": counts["events"] })
}

/// The worker's half of an erase: delete the events of every erasure due by `now` (on the sites
/// `scope` selects; see the worker), then forget the forms that hid them. Returns the erasures purged.
pub async fn purge_due(pool: &PgPool, now: DateTime<Utc>, scope_sql: &str, scope: Option<&str>) -> anyhow::Result<u64> {
    let due: Vec<i64> = sqlx::query_scalar(&format!(
        "SELECT r.id FROM erasures r JOIN sites t ON t.id = r.site_id
          WHERE r.purged_at IS NULL AND r.purge_after <= $1 AND {scope_sql} ORDER BY r.purge_after LIMIT 100"
    ))
    .bind(now)
    .bind(scope)
    .fetch_all(pool)
    .await?;
    for id in &due {
        let mut tx = pool.begin().await?;
        let n = sqlx::query(
            "DELETE FROM events e USING erased_subjects x WHERE x.erasure_id = $1 AND e.site_id = x.site_id AND e.subject = x.subject",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query("DELETE FROM erased_subjects WHERE erasure_id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE erasures SET purged_at = $2, counts = counts || jsonb_build_object('events_purged', $3::int8) WHERE id = $1")
            .bind(id)
            .bind(now)
            .bind(n as i64)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    Ok(due.len() as u64)
}

/// The audit log, newest first.
pub async fn erasures(pool: &PgPool, site: &str) -> Result<Vec<Value>, ApiError> {
    type Row = (i64, String, String, Option<String>, Value, DateTime<Utc>, DateTime<Utc>, Option<DateTime<Utc>>);
    let rows: Vec<Row> =
        sqlx::query_as("SELECT id, subject_hash, actor, reason, counts, created_at, purge_after, purged_at FROM erasures WHERE site_id = $1 ORDER BY id DESC LIMIT 500")
            .bind(site)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, h, actor, reason, counts, at, due, done)| {
            let events_removal = removal(&counts, due, done);
            json!({ "id": id, "subject_hash": h, "actor": actor, "reason": reason, "counts": counts, "created_at": at, "events_removal": events_removal })
        })
        .collect())
}

/// Right-of-access: everything held under the subject (its id and today's pseudonym).
pub async fn access(pool: &PgPool, site: &str, subject: &str) -> Result<Value, ApiError> {
    if !valid_subject(subject) {
        return Err(ApiError::BadRequest("subject must be 1-256 printable ASCII characters".into()));
    }
    if policy(pool, site).await?.is_none() {
        return Err(ApiError::NotFound(format!("unknown site {site:?}")));
    }
    let mut tx = pool.begin().await?;
    let erased = erased(&mut tx, site, subject).await?;
    let forms = stored_forms(&mut tx, site, subject).await?;
    let visitors: Vec<(String, DateTime<Utc>, DateTime<Utc>, i64, Vec<String>)> =
        sqlx::query_as("SELECT subject, first_seen, last_seen, events, recent FROM visitors WHERE site_id = $1 AND subject = ANY($2) ORDER BY first_seen")
            .bind(site)
            .bind(&forms)
            .fetch_all(&mut *tx)
            .await?;
    let events: Vec<(uuid::Uuid, String, String, Option<uuid::Uuid>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, subject, name, decision_id, received_at FROM live_events WHERE site_id = $1 AND subject = ANY($2) ORDER BY received_at LIMIT 1000",
    )
    .bind(site)
    .bind(&forms)
    .fetch_all(&mut *tx)
    .await?;
    let decisions: Vec<(uuid::Uuid, String, String, String, Value, DateTime<Utc>, Option<i16>)> = sqlx::query_as(
        "SELECT d.id, d.subject, s.key, v.key, d.features, d.created_at, d.outcome
           FROM decisions d JOIN slots s ON s.id = d.slot_id JOIN variants v ON v.id = d.variant_id
          WHERE d.site_id = $1 AND d.subject = ANY($2) ORDER BY d.created_at LIMIT 1000",
    )
    .bind(site)
    .bind(&forms)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(json!({
        "site": site,
        "erased": erased,
        "visitors": visitors.into_iter().map(|(s, f, l, n, r)| json!({ "subject": s, "first_seen": f, "last_seen": l, "events": n, "recent": r })).collect::<Vec<_>>(),
        "events": events.into_iter().map(|(id, s, name, d, at)| json!({ "id": id, "subject": s, "name": name, "decision_id": d, "received_at": at })).collect::<Vec<_>>(),
        "decisions": decisions.into_iter().map(|(id, s, slot, variant, f, at, o)| json!({ "id": id, "subject": s, "slot": slot, "variant": variant, "features": f, "created_at": at, "outcome": o })).collect::<Vec<_>>(),
    }))
}

/// Change a site's mode and/or field list. `fields: Some(None)` clears the list (every field).
pub async fn set(pool: &PgPool, site: &str, mode: Option<&str>, fields: Option<Option<Vec<String>>>) -> Result<Value, ApiError> {
    validate(mode, fields.as_ref().and_then(|f| f.as_deref())).map_err(ApiError::BadRequest)?;
    let row: Option<(String, Option<Vec<String>>)> = sqlx::query_as(
        "UPDATE sites SET privacy_mode = COALESCE($2, privacy_mode),
                          fields_allowed = CASE WHEN $3 THEN $4 ELSE fields_allowed END
          WHERE id = $1 RETURNING privacy_mode, fields_allowed",
    )
    .bind(site)
    .bind(mode)
    .bind(fields.is_some())
    .bind(fields.flatten())
    .fetch_optional(pool)
    .await?;
    let Some((mode, fields)) = row else { return Err(ApiError::NotFound(format!("unknown site {site:?}"))) };
    Ok(json!({ "site": site, "privacy_mode": mode, "fields_allowed": fields }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(mode: &str, fields: Option<&[&str]>) -> Policy {
        Policy { mode: mode.into(), fields: fields.map(|f| f.iter().map(|s| s.to_string()).collect()), at: Utc::now() }
    }

    #[test]
    fn full_mode_is_identified_without_consent() {
        assert!(p("full", None).identified(None));
    }

    #[test]
    fn cookieless_mode_ignores_a_consent_claim() {
        assert!(!p("cookieless", None).identified(Some(true)));
    }

    #[test]
    fn consent_aware_is_identified_only_with_consent() {
        assert!(p("consent_aware", None).identified(Some(true)));
    }

    #[test]
    fn consent_aware_without_consent_is_not_identified() {
        assert!(!p("consent_aware", None).identified(None));
    }

    #[test]
    fn pseudonym_depends_on_the_salt() {
        assert_ne!(pseudonym("a", "demo", "alice"), pseudonym("b", "demo", "alice"));
    }

    #[test]
    fn pseudonym_depends_on_the_subject() {
        assert_ne!(pseudonym("a", "demo", "alice"), pseudonym("a", "demo", "bob"));
    }

    #[test]
    fn pseudonym_is_a_valid_subject() {
        let s = pseudonym("a", "demo", "alice");
        assert!(s.starts_with(PSEUDONYM) && valid_subject(&s) && s.len() == PSEUDONYM.len() + 32, "{s}");
    }

    #[test]
    fn subject_hash_separates_site_and_subject() {
        assert_ne!(subject_hash("ab", "c"), subject_hash("a", "bc"));
    }

    #[test]
    fn validate_accepts_a_known_mode_and_fields() {
        assert_eq!(validate(Some("cookieless"), Some(&["returning".to_string(), "rescore".to_string()])), Ok(()));
    }

    #[test]
    fn validate_refuses_an_unknown_mode() {
        assert!(validate(Some("anonymous"), None).is_err());
    }

    fn ev(name: &str, picks: bool, rescore: bool) -> EventIn {
        EventIn {
            id: uuid::Uuid::new_v4(),
            name: name.into(),
            slot: Some("s".into()),
            decision_id: Some(uuid::Uuid::new_v4()),
            picks: picks.then(|| vec!["kitchen".to_string()]),
            variant: rescore.then(|| "gift".to_string()),
            features: rescore.then(|| json!({})),
            score: rescore.then_some(0.5),
        }
    }

    #[test]
    fn a_pick_whose_picks_are_not_collected_is_dropped() {
        assert!(p("full", Some(&["slot"])).events(vec![ev("pick", true, false)]).is_empty());
    }

    #[test]
    fn a_pick_is_kept_when_picks_are_collected() {
        assert_eq!(p("full", Some(&["slot", "picks"])).events(vec![ev("pick", true, false)]).len(), 1);
    }

    #[test]
    fn a_malformed_pick_is_left_for_the_ingest_checks() {
        assert_eq!(p("full", None).events(vec![ev("pick", false, false)]).len(), 1);
    }

    #[test]
    fn a_rescore_is_dropped_when_rescore_is_not_collected() {
        assert!(p("full", Some(&["slot", "decision_id"])).events(vec![ev("rescore", false, true)]).is_empty());
    }

    #[test]
    fn validate_refuses_an_unknown_field() {
        assert!(validate(None, Some(&["gender".to_string()])).is_err());
    }
}

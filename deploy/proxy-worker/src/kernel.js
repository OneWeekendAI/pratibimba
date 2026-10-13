// EC2 (edge-cache.md): Core's `/decide` (core/src/http/decide.rs, `decide`) as a pure function of
// the site's edge snapshot. Given the same snapshot, request, seed and clock it returns the
// decision Core returns: the same arm, payload, holdout flag, experiment arm, cold-start turn,
// audiences and LinTS scores. core/tests/edge_parity.rs holds it to that over thousands of
// generated decisions that Core really made.
//
// It never errors the way Core does. Anything Core would refuse (a bad subject, an unknown slot,
// an undeclared feature, a paused site with no baseline) or anything this snapshot cannot
// answer (no model, an unknown zone) returns `{ ok: false, reason }`, and the caller asks Core,
// which gives the real answer. So a request can never be refused at the edge that Core accepts.
//
// What is NOT here, because it is per visitor or per instant and the snapshot does not hold it:
// the cookieless pseudonym (the caller passes `stored`, learned from Core), the sticky decision
// (the caller passes `sticky`), the visitor's latest picks (`picks`) and the clock (`at`).

import { ChaCha8 } from './rng.js';
import { inHoldout, withinShare, assign } from './hash.js';
import { ZONES, SIN_HOUR, COS_HOUR } from './tables.js';

export const FORMAT = 2;
const FEATURE_NAMES = ['bias', 'returning', 'mobile', 'from_ad'];
const DIM = 4;
const POLICY = 'lints';
const STATED_POLICY = 'lints_stated';
const FEATURES_POLICY = 'lints_features';
const TIME_POLICY = 'lints_time';
const STATED_PREFIX = 'stated:';
const FEATURE_PREFIX = 'feature:';
const TIME_PREFIX = 'time:';
const TIME_FEATURES = ['sin_hour', 'cos_hour', 'is_night_local', 'is_weekend_local', 'is_business_hours_local'];
const CTX_FIELDS = new Set(['returning', 'mobile', 'from_ad', 'stated', 'tz_offset', 'tz', 'consent', 'features']);
const DEMOGRAPHIC = ['gender', 'sex', 'age', 'age_band', 'age_range', 'birth_year', 'income', 'ethnicity', 'race', 'religion', 'nationality',
  'sexual_orientation', 'marital_status', 'disability'];
const BOOL_FEATURES = ['returning', 'mobile', 'from_ad', 'is_night_local', 'is_weekend_local', 'is_business_hours_local'];

const no = (reason) => ({ ok: false, reason });
const isObj = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
const isBool = (v) => typeof v === 'boolean';
/** Rust's BTreeMap order for ASCII keys. */
const byKey = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
/** Rust's f64 `Sum`, which starts from -0.0. */
function sum(xs) {
  let s = -0.0;
  for (const x of xs) s += x;
  return s;
}
const dot = (a, b) => {
  let s = -0.0;
  for (let i = 0; i < a.length && i < b.length; i++) s += a[i] * b[i];
  return s;
};
const sigmoid = (z) => 1.0 / (1.0 + Math.exp(-z));

/** `valid_subject`: 1-256 printable ASCII characters (no space). */
export function validSubject(s) {
  if (typeof s !== 'string' || s.length === 0 || s.length > 256) return false;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c < 0x21 || c > 0x7e) return false;
  }
  return true;
}

/** The request's ctx, typed as Core's `Ctx` deserialises it, or null where Core answers 400. */
export function parseCtx(raw) {
  if (raw === undefined) raw = {};
  if (!isObj(raw)) return null;
  for (const k of Object.keys(raw)) if (!CTX_FIELDS.has(k)) return null;
  const ctx = { returning: false, mobile: false, from_ad: false, stated: [], tz_offset: null, tz: null, consent: null, features: {} };
  for (const k of ['returning', 'mobile', 'from_ad']) {
    if (k in raw) {
      if (!isBool(raw[k])) return null;
      ctx[k] = raw[k];
    }
  }
  if ('stated' in raw) {
    if (!Array.isArray(raw.stated) || !raw.stated.every((s) => typeof s === 'string')) return null;
    ctx.stated = raw.stated.slice();
  }
  if ('tz_offset' in raw && raw.tz_offset !== null) {
    if (!Number.isInteger(raw.tz_offset) || raw.tz_offset < -32768 || raw.tz_offset > 32767) return null;
    ctx.tz_offset = raw.tz_offset;
  }
  if ('tz' in raw && raw.tz !== null) {
    if (typeof raw.tz !== 'string') return null;
    ctx.tz = raw.tz;
  }
  if ('consent' in raw && raw.consent !== null) {
    if (!isBool(raw.consent)) return null;
    ctx.consent = raw.consent;
  }
  if ('features' in raw) {
    if (!isObj(raw.features)) return null;
    for (const [k, v] of Object.entries(raw.features)) {
      if (!isBool(v)) return null;
      ctx.features[k] = v;
    }
  }
  return ctx;
}

/** `privacy::Policy::identified`. */
export function identified(mode, consent) {
  return mode === 'full' || (mode === 'consent_aware' && consent === true);
}

// ---- local time (core/src/insights/localtime.rs) ----

const validOffset = (m) => m >= -720 && m <= 840;
const zoneFormats = new Map();

/** A zone's offset in minutes east of UTC at `atMs`, or null when the runtime cannot name it. */
export function zoneOffset(zone, atMs) {
  let f = zoneFormats.get(zone);
  if (!f) {
    try {
      f = new Intl.DateTimeFormat('en-US', {
        timeZone: zone, hourCycle: 'h23', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric',
      });
    } catch {
      return null;
    }
    zoneFormats.set(zone, f);
  }
  const p = {};
  for (const part of f.formatToParts(new Date(atMs))) p[part.type] = part.value;
  const local = Date.UTC(Number(p.year), Number(p.month) - 1, Number(p.day), Number(p.hour), Number(p.minute), Number(p.second));
  const whole = Math.floor(atMs / 1000) * 1000;
  return Math.trunc((local - whole) / 60000);
}

function nightContains(night, hour) {
  return night.start <= night.end ? hour >= night.start && hour < night.end : hour >= night.start || hour < night.end;
}

export function localTime(atMs, offset, night) {
  const off = offset !== null && offset !== undefined && validOffset(offset) ? offset : 0;
  const d = new Date(atMs + off * 60000);
  const hour = d.getUTCHours();
  const dow = d.getUTCDay();
  const weekend = dow === 0 || dow === 6;
  return {
    hour_local: hour,
    dow_local: dow,
    sin_hour: SIN_HOUR[hour],
    cos_hour: COS_HOUR[hour],
    is_weekend_local: weekend,
    is_night_local: nightContains(night, hour),
    is_business_hours_local: !weekend && hour >= 9 && hour < 17,
  };
}

// ---- segment predicates (core/src/insights/segments.rs) ----

function parsePredicate(v) {
  if (!isObj(v)) return null;
  const keys = Object.keys(v);
  if (keys.length === 0) return null;
  const p = { bools: [], hours: null };
  for (const k of keys) {
    const val = v[k];
    if (DEMOGRAPHIC.includes(k)) return null;
    if (k === 'hour_local') {
      if (!Array.isArray(val) || val.length !== 2) return null;
      const h = (x) => (Number.isInteger(x) && x >= 0 && x < 24 ? x : null);
      const f = h(val[0]);
      const t = h(val[1]);
      if (f === null || t === null) return null;
      p.hours = [f, t];
    } else if (BOOL_FEATURES.includes(k)) {
      if (!isBool(val)) return null;
      p.bools.push([k, val]);
    } else {
      return null;
    }
  }
  return p;
}

const usesLocalTime = (p) => p.hours !== null || p.bools.some(([k]) => k.endsWith('_local'));

function matches(p, visit) {
  if (p.hours) {
    const h = visit.local.hour_local;
    const [f, t] = p.hours;
    const inside = f <= t ? h >= f && h <= t : h >= f || h <= t;
    if (!inside) return false;
  }
  return p.bools.every(([k, want]) => {
    const got = k in visit.ctx ? visit.ctx[k] : visit.local[k];
    return got === want;
  });
}

/** `gate_by_audience`: drops shut variants from `arms` in place; returns the explanation list. */
function gateByAudience(gates, ctx, atMs, night, arms) {
  const visit = {
    ctx: { returning: ctx.returning, mobile: ctx.mobile, from_ad: ctx.from_ad },
    local: localTime(atMs, ctx.tz_offset, night),
  };
  const out = [];
  const shut = [];
  for (const g of gates) {
    if (out.length === 0 || out[out.length - 1].id !== g.audience_id) {
      const p = g.status === 'audience' ? parsePredicate(g.predicate) : null;
      const matched = p !== null && (ctx.tz_offset !== null || !usesLocalTime(p)) && matches(p, visit);
      out.push({ id: g.audience_id, name: g.audience, matched, variants: [] });
    }
    const entry = out[out.length - 1];
    if (entry.matched === false) {
      shut.push(g.variant_id);
      entry.variants.push(g.variant);
    }
  }
  for (let i = arms.length - 1; i >= 0; i--) if (shut.includes(arms[i].id)) arms.splice(i, 1);
  return out;
}

// ---- LinTS (core/src/bandit.rs) and its extension (core/src/stated.rs) ----

function cholesky(a, n) {
  const l = Array.from({ length: n }, () => new Array(n).fill(0.0));
  for (let i = 0; i < n; i++) {
    for (let j = 0; j <= i; j++) {
      let s = a[i][j];
      for (let k = 0; k < j; k++) s -= l[i][k] * l[j][k];
      l[i][j] = i === j ? Math.sqrt(Math.max(s, 1e-12)) : s / l[j][j];
    }
  }
  return l;
}

const prior = (dim) => ({
  mean: new Array(dim).fill(0.0),
  cov: Array.from({ length: dim }, (_, i) => Array.from({ length: dim }, (_, j) => (i === j ? 1.0 : 0.0))),
});

function baseArm(model, id) {
  const a = model.arms && model.arms[String(id)];
  return a ? { mean: a.mean.slice(), cov: a.cov.map((r) => r.slice()) } : prior(DIM);
}

/** `bandit::Arm::sample_score`: `w[i] += l[i][k] * z[k]` one term at a time. */
function sampleBase(arm, x, rng) {
  const l = cholesky(arm.cov, DIM);
  const z = [];
  for (let i = 0; i < DIM; i++) z.push(rng.normal());
  const w = arm.mean.slice();
  for (let i = 0; i < DIM; i++) for (let k = 0; k <= i; k++) w[i] += l[i][k] * z[k];
  return sigmoid(dot(w, x));
}

/** `stated::DynArm::sample_score`: each row's terms summed first, then added. */
function sampleDyn(arm, x, rng) {
  const n = arm.mean.length;
  const l = cholesky(arm.cov, n);
  const z = [];
  for (let i = 0; i < n; i++) z.push(rng.normal());
  const w = arm.mean.slice();
  for (let i = 0; i < n; i++) {
    let s = -0.0;
    for (let k = 0; k <= i; k++) s += l[i][k] * z[k];
    w[i] += s;
  }
  return sigmoid(dot(w, x));
}

const sortedIds = (ids) => [...new Set(ids)].sort((a, b) => a - b);

/** `LinTs::choose`: the first highest score wins. */
export function chooseBase(model, ids, x, seed) {
  const rng = new ChaCha8(seed);
  const scores = sortedIds(ids).map((id) => [id, sampleBase(baseArm(model, id), x, rng)]);
  let best = null;
  for (const [id, s] of scores) if (best === null || !(best[1] >= s)) best = [id, s];
  return [best[0], scores];
}

/** `DynArm::extend_to`, Vec::resize semantics included. */
function extendTo(arm, dim) {
  const old = arm.mean.length;
  arm.mean.length = Math.min(arm.mean.length, dim);
  while (arm.mean.length < dim) arm.mean.push(0.0);
  for (const row of arm.cov) {
    row.length = Math.min(row.length, dim);
    while (row.length < dim) row.push(0.0);
  }
  for (let i = old; i < dim; i++) {
    const row = new Array(dim).fill(0.0);
    row[i] = 1.0;
    arm.cov.push(row);
  }
}

/** `stated::model_for`: the stored extension with any new keys appended, sorted. */
function modelFor(model, keys) {
  const st = model.stated || { keys: [], arms: {} };
  const m = { keys: (st.keys || []).slice(), arms: {} };
  for (const [id, a] of Object.entries(st.arms || {})) m.arms[id] = { mean: a.mean.slice(), cov: a.cov.map((r) => r.slice()) };
  const known = new Set(m.keys);
  const fresh = [...new Set(keys.filter((k) => !known.has(k)))].sort(byKey);
  if (fresh.length > 0) {
    m.keys.push(...fresh);
    const dim = DIM + m.keys.length;
    for (const a of Object.values(m.arms)) extendTo(a, dim);
  }
  return m;
}

function dynArm(m, model, id) {
  const dim = DIM + m.keys.length;
  const a = m.arms[String(id)];
  if (a && a.mean.length === dim && a.cov.length === dim && a.cov.every((r) => r.length === dim)) return a;
  const b = baseArm(model, id);
  const arm = { mean: b.mean, cov: b.cov };
  extendTo(arm, dim);
  return arm;
}

/** `stated::choose`. `ext` maps model key -> value. */
export function chooseStated(model, keys, ids, x, ext, seed) {
  const m = modelFor(model, keys);
  const v = x.concat(m.keys.map((k) => (k in ext ? ext[k] : 0.0)));
  const rng = new ChaCha8(seed);
  const scores = sortedIds(ids).map((id) => [id, sampleDyn(dynArm(m, model, id), v, rng)]);
  let best = scores[0];
  for (const sc of scores.slice(1)) if (sc[1] > best[1]) best = sc;
  return [best[0], scores];
}

// ---- features (decide.rs `named`, stated.rs `with_stated`, features.rs `values`) ----

function featureName(key) {
  if (key.startsWith(FEATURE_PREFIX)) return key;
  if (key.startsWith(TIME_PREFIX)) return key.slice(TIME_PREFIX.length);
  return STATED_PREFIX + key;
}

function named(x) {
  const m = {};
  FEATURE_NAMES.forEach((n, i) => { m[n] = x[i]; });
  return m;
}

function withStated(features, keys, ext) {
  for (const k of keys) features[featureName(k)] = k in ext ? ext[k] : 0.0;
  return features;
}

/** `stated::from_features`: option key -> value, sorted. */
function fromFeatures(features) {
  const out = {};
  for (const k of Object.keys(features || {}).sort(byKey)) {
    if (k.startsWith(STATED_PREFIX) && typeof features[k] === 'number') out[k.slice(STATED_PREFIX.length)] = features[k];
  }
  return out;
}

const sameMap = (a, b) => {
  const ka = Object.keys(a).sort(byKey);
  const kb = Object.keys(b).sort(byKey);
  return ka.length === kb.length && ka.every((k, i) => k === kb[i] && a[k] === b[k]);
};

function timeValues(t) {
  const b = (v) => (v ? 1.0 : 0.0);
  const v = [t.sin_hour, t.cos_hour, b(t.is_night_local), b(t.is_weekend_local), b(t.is_business_hours_local)];
  const out = {};
  TIME_FEATURES.forEach((n, i) => { out[TIME_PREFIX + n] = v[i]; });
  return out;
}

// ---- cold start ----

function coldStart(until, cold, ids, seed) {
  if (until <= 0 || !cold) return null;
  if (cold.exposures >= until) return null;
  const turns = (id) => cold.served[String(id)] || 0;
  let fewest = null;
  for (const id of ids) fewest = fewest === null ? turns(id) : Math.min(fewest, turns(id));
  if (fewest === null) fewest = 0;
  const due = ids.filter((id) => turns(id) === fewest);
  const arm = due[Number(seed % BigInt(due.length))];
  return [arm, { exposures: cold.exposures, until }];
}

// ---- the response (decide.rs `response`) ----

function respond(id, slot, variant, payload, policy, version, features, scoresById, seed, held, arms, audiences, cold, kind, experiment) {
  const scores = {};
  for (const a of arms) if (String(a.id) in scoresById) scores[a.key] = scoresById[String(a.id)];
  return {
    decision_id: id,
    slot,
    variant,
    payload,
    explanation: {
      policy,
      policy_version: version,
      features,
      scores,
      seed: seed.toString(),
      in_holdout: held,
      audiences,
      cold_start: cold,
      allocation: held ? 'holdout' : kind,
      experiment: experiment ? { id: experiment.id, name: experiment.name } : null,
    },
  };
}

/**
 * Decide one slot.
 *
 * - `snap`: the site's edge snapshot (format 2).
 * - `req`: `{ site, slot, subject, ctx }` as the client sent it (subject raw).
 * - `opts.stored`: the subject as Core stores it. Needed only when the request is not
 *   identified (a cookieless pseudonym, which only Core can make); without it the answer is
 *   `unseen-cookieless`.
 * - `opts.seed`: a BigInt u64. `opts.at`: the clock, ms since the epoch.
 * - `opts.sticky`: the subject's live decision on this slot, as `record` returned it, or null.
 * - `opts.picks`: the subject's latest picks (Core reads them per visitor; see the caller).
 * - `opts.id`: the new decision's id.
 *
 * Returns `{ ok: true, response, record }` (`record` null when the sticky decision was reused)
 * or `{ ok: false, reason }`.
 */
export function decide(snap, req, opts) {
  if (!snap || snap.format !== FORMAT) return no('snapshot-format');
  if (!isObj(req)) return no('bad-request');
  for (const k of Object.keys(req)) if (!['site', 'slot', 'subject', 'ctx'].includes(k)) return no('bad-request');
  if (typeof req.site !== 'string' || typeof req.slot !== 'string' || req.site !== snap.site) return no('bad-request');
  if (!validSubject(req.subject)) return no('bad-request');
  const raw = parseCtx(req.ctx);
  if (!raw) return no('bad-request');
  if (raw.tz_offset !== null && !validOffset(raw.tz_offset)) return no('bad-request');
  if (raw.tz !== null && (raw.tz.length === 0 || raw.tz.length > 64 || !ZONES.has(raw.tz))) return no('bad-request');

  const settings = snap.settings;
  if (settings.simulated_clock) return no('simulated-clock');
  const ident = identified(settings.privacy_mode, raw.consent);
  const subject = ident ? req.subject : opts.stored;
  if (!subject) return no('unseen-cookieless');
  const fields = settings.fields_allowed;
  const allows = (f) => fields === null || fields === undefined || fields.includes(f);
  const filtered = {
    returning: raw.returning && ident && allows('returning'),
    mobile: raw.mobile && allows('mobile'),
    from_ad: raw.from_ad && allows('from_ad'),
    stated: allows('stated') ? raw.stated : [],
    tz_offset: allows('tz_offset') ? raw.tz_offset : null,
    tz: allows('tz_offset') ? raw.tz : null,
    consent: raw.consent,
    features: raw.features,
  };
  const at = opts.at;
  const slot = snap.slots[req.slot];
  if (!slot) return no('unknown-slot');
  let offset = filtered.tz_offset;
  if (filtered.tz !== null) {
    offset = zoneOffset(filtered.tz, at);
    if (offset === null) return no('zone');
  }
  const ctx = { ...filtered, tz_offset: offset };

  let arms = slot.arms.map((a) => ({ ...a }));
  const paused = settings.paused;
  if (paused) {
    arms = arms.filter((a) => a.baseline);
    if (arms.length === 0) return no('paused');
  }
  if (arms.length === 0) return no('no-arms');
  const night = { start: settings.night.start, end: settings.night.end };
  const audiences = gateByAudience(slot.gates, ctx, at, night, arms);
  if (arms.length === 0) return no('no-open-arm');

  const siteKeys = slot.picker_keys;
  const bad = ctx.stated.find((k) => !siteKeys.includes(k));
  if (bad !== undefined) return no('bad-request');
  const said = {};
  for (const k of ctx.stated) said[k] = 1.0;
  for (const k of (opts.picks || []).flat()) if (siteKeys.includes(k)) said[k] = 1.0;
  const saidNow = fromFeatures(withStated({}, siteKeys, said));
  const declaredSpec = slot.declared_features || {};
  for (const k of Object.keys(ctx.features)) if (!(k in declaredSpec)) return no('bad-request');
  const declared = {};
  for (const k of Object.keys(declaredSpec).sort(byKey)) {
    if (!declaredSpec[k] || ident) declared[FEATURE_PREFIX + k] = ctx.features[k] === true ? 1.0 : 0.0;
  }
  const keyOf = (id) => arms.find((a) => a.id === id);
  const experiment = paused ? null : slot.experiment;
  if (experiment && experiment.ends_at && Date.parse(experiment.ends_at) <= at) return no('experiment-clock');
  if (!paused && slot.next_experiment_at && Date.parse(slot.next_experiment_at) <= at) return no('experiment-clock');
  const experimentId = experiment ? experiment.id : null;

  const st = opts.sticky;
  if (st && st.created_at_ms > at - slot.decision_ttl_secs * 1000 && st.created_at_ms <= at) {
    const sameStatement = sameMap(fromFeatures(st.features), saidNow);
    const arm = keyOf(st.variant_id);
    if (sameStatement && (st.experiment_id ?? null) === experimentId && arm) {
      const exp = experiment;
      return {
        ok: true,
        record: null,
        response: respond(st.id, req.slot, arm.key, arm.payload, st.policy, st.policy_version, st.features, st.scores, BigInt(st.seed),
          st.in_holdout, arms, st.audiences || [], st.cold_start ?? null, st.allocation, exp),
      };
    }
  }

  const eligible = arms
    .filter((a) => (a.traffic_share === null || a.traffic_share === undefined ? true : withinShare(a.id, subject, Math.fround(a.traffic_share))))
    .map((a) => a.id);
  const baselineArm = arms.find((a) => a.baseline);
  const baseline = baselineArm ? baselineArm.id : null;
  const holdout = baseline !== null && inHoldout(req.site, subject, Math.fround(settings.holdout_fraction));

  if (!slot.model) return no('no-model');
  const version = slot.model.version;
  const model = slot.model.state || {};
  const x = [1.0, ctx.returning ? 1.0 : 0.0, ctx.mobile ? 1.0 : 0.0, ctx.from_ad ? 1.0 : 0.0];
  const seed = opts.seed;
  const ids = eligible.length === 0 ? arms.map((a) => a.id) : eligible;
  const extKeys = siteKeys.slice();
  const ext = { ...said };
  extKeys.push(...Object.keys(declared));
  Object.assign(ext, declared);
  if (ctx.tz_offset !== null) {
    const local = localTime(at, ctx.tz_offset, night);
    extKeys.push(...TIME_FEATURES.map((n) => TIME_PREFIX + n));
    Object.assign(ext, timeValues(local));
  }
  let picked;
  let scores;
  let policy;
  let features;
  if (extKeys.length === 0) {
    [picked, scores] = chooseBase(model, ids, x, seed);
    policy = POLICY;
    features = named(x);
  } else {
    [picked, scores] = chooseStated(model, extKeys, ids, x, ext, seed);
    policy = siteKeys.length > 0 ? STATED_POLICY : Object.keys(declared).length > 0 ? FEATURES_POLICY : TIME_POLICY;
    features = withStated(named(x), extKeys, ext);
  }
  const held = holdout && baseline !== null;
  let assigned = null;
  if (experiment && !held) {
    const open = experiment.arms.filter((a) => ids.includes(a.variant_id)).map((a) => [a.variant_id, a.weight]);
    if (open.length > 0) assigned = assign(experiment.id, subject, open);
  }
  let cold = held || assigned !== null ? null : coldStart(slot.cold_start_exposures, slot.cold_start, ids, seed);
  let chosen;
  if (held) chosen = baseline;
  else if (assigned !== null) chosen = assigned;
  else if (cold) chosen = cold[0];
  else chosen = picked;
  const allocation = assigned !== null ? 'experiment' : 'bandit';
  cold = cold ? cold[1] : null;
  const scoresById = {};
  for (const [id, s] of scores) scoresById[String(id)] = s;
  const arm = keyOf(chosen);
  const id = opts.id;
  const record = {
    id,
    slot: req.slot,
    subject,
    variant_id: chosen,
    pool: ids,
    features,
    scores: scoresById,
    policy,
    policy_version: version,
    seed: seed.toString(),
    in_holdout: holdout,
    created_at: new Date(at).toISOString(),
    created_at_ms: at,
    audiences,
    cold_start: cold,
    experiment_id: experimentId,
    allocation,
  };
  return {
    ok: true,
    record,
    response: respond(id, req.slot, arm.key, arm.payload, policy, version, features, scoresById, seed, holdout, arms, audiences, cold, allocation, experiment),
  };
}

export const _test = { sum, cholesky, parsePredicate, coldStart, modelFor };

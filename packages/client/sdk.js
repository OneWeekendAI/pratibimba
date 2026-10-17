/*! Pratibimba client fallback (MIT). Served by Core at /sdk.js; proxy it to /_prb/sdk.js.
 *
 * Install: two plain script tags in <head>, no defer or async, the manifest first:
 *   script src="/_prb/manifest/SITE.js"
 *   script src="/_prb/sdk.js" data-site="SITE"
 * (This file must never contain a closing script tag, so it stays safe to inline.)
 *
 * What it does, in order:
 *   1. Synchronously hides ONLY the manifest's slot selectors (<style id="prb-hide">). The rest
 *      of the page paints normally.
 *   2. Asks /decide for every slot at once, without waiting for the DOM.
 *   3. When the DOM and a decision are both ready, swaps the slot's HTML, reveals it and reports
 *      an `exposure`.
 *   4. A hard timeout (400 ms, data-timeout) removes the hide rule whatever happened. A decision
 *      that lands after it is NOT swapped (that would be the flicker this exists to avoid) and is
 *      reported as `missed`.
 *
 * The subject lives in sessionStorage (cookieless) or in a first-party `_prb_subject` cookie for
 * a year, by the site's privacy_mode (from the manifest): `full` always uses the cookie,
 * `cookieless` never does, and `consent_aware` (the default) uses it only with consent. Consent
 * is the tag's data-cookie="on" (the page says it has it), or else IAB TCF v2 purpose 1 from
 * __tcfapi, or else the latest Google Consent Mode `analytics_storage` in dataLayer, or else
 * OneTrust's performance group (C0002). It is read once per page; a choice made on the page
 * (or a TCF answer that arrives after the read) applies from the next one. Every decide and
 * event batch says whether the visitor consented, and Core stores a daily pseudonym otherwise.
 *
 * Declared features: a page may set window.__prb_features = { name: true|false } before this
 * tag. The SDK sends, as ctx.features, only the names the manifest declares and only boolean
 * values; a feature the site tagged pii is sent only when the visitor is identified (`full`, or
 * `consent_aware` with consent), so it never leaves the device otherwise.
 *
 * Picker slots (kind "picker"): when the decision is `picker`, the SDK renders the owner's
 * options, 1 to max_picks of them, with Done and Skip. Done sends a `pick` event with the keys;
 * the picks become `ctx.stated` on every later decide (sessionStorage, or localStorage only
 * with data-cookie="on"). Skip sends `skip` and stores nothing.
 *
 * In-session slots (in_session: true) ship each variant's payload and linear weights. After a
 * signal (a pick, or a click on an element with data-prb-intent="<option key>"; at least
 * data-adapt-min of them, default 1) the SDK re-scores them on the device in one animation
 * frame: a dot product per variant. Hard rule: it never swaps a slot that is in the viewport,
 * holds focus, or is under the pointer. Those, and slots not on this page, are cached for the
 * next page instead. Each on-device decision is posted as a `rescore` event. A holdout visitor
 * is never re-scored. The weights include the declared features (`feature:<key>`): the SDK
 * fills them from what it sends as ctx.features, and leaves a PII one out unless identified.
 * Once a slot has learned the time features (`sin_hour` .. `is_business_hours_local`), its
 * weights name them too: the SDK computes them from the browser clock and the manifest's `night`.
 */
(function () {
  'use strict';
  var script = document.currentScript;
  var manifest = window.__prb_manifest;
  if (!script || !manifest || !manifest.slots) return;

  var site = script.getAttribute('data-site') || manifest.site;
  var base = script.getAttribute('data-base') || (script.src || '').replace(/sdk\.js(\?.*)?$/, '');
  var timeoutMs = parseInt(script.getAttribute('data-timeout') || '400', 10);
  var mode = manifest.privacy_mode || 'consent_aware';
  // IAB TCF v2 (purpose 1, "store and/or access information on a device"). __tcfapi is
  // asynchronous: a CMP that already holds the choice answers at once and that answer counts on
  // this page; a later answer is kept in sessionStorage and counts from the next page. A CMP
  // that says GDPR does not apply, or whose banner is still up, is no answer.
  function tcf() {
    var now = null;
    if (typeof window.__tcfapi === 'function') {
      try {
        window.__tcfapi('addEventListener', 2, function (d, ok) {
          if (!ok || !d) return;
          var v = null;
          if (d.gdprApplies === false) v = 'na';
          else if (d.eventStatus === 'tcloaded' || d.eventStatus === 'useractioncomplete')
            v = d.purpose && d.purpose.consents && d.purpose.consents[1] === true ? '1' : '0';
          if (v === null) return;
          now = v;
          try { window.sessionStorage.setItem('prb_tcf', v); } catch (e) { /* private mode */ }
        });
      } catch (e) { /* a broken CMP is no answer */ }
    }
    if (now === null) { try { now = window.sessionStorage.getItem('prb_tcf'); } catch (e) { /* private mode */ } }
    return now === '1' ? true : now === '0' ? false : null;
  }
  function consented() {
    if (script.getAttribute('data-cookie') === 'on') return true;
    var iab = tcf();
    if (iab !== null) return iab;
    var google = null;
    (window.dataLayer || []).forEach(function (e) {
      if (e && e[0] === 'consent' && e[2] && e[2].analytics_storage) google = e[2].analytics_storage;
    });
    if (google) return google === 'granted';
    if (typeof window.OptanonActiveGroups === 'string') return window.OptanonActiveGroups.indexOf(',C0002,') !== -1;
    return false;
  }
  var useCookie = mode === 'full' || (mode === 'consent_aware' && consented());
  var adaptMin = parseInt(script.getAttribute('data-adapt-min') || '1', 10);
  var slots = manifest.slots;

  var store = {
    get: function (k) { try { return window.sessionStorage.getItem(k); } catch (e) { return null; } },
    set: function (k, v) { try { window.sessionStorage.setItem(k, v); } catch (e) { /* private mode */ } },
    del: function (k) { try { window.sessionStorage.removeItem(k); } catch (e) { /* private mode */ } }
  };

  function newId() {
    if (window.crypto && window.crypto.randomUUID) return window.crypto.randomUUID();
    var s = '';
    for (var i = 0; i < 32; i++) s += Math.floor(Math.random() * 16).toString(16);
    return s.slice(0, 8) + '-' + s.slice(8, 12) + '-4' + s.slice(13, 16) + '-a' + s.slice(17, 20) + '-' + s.slice(20, 32);
  }

  function readCookie(name) {
    var m = document.cookie.match(new RegExp('(?:^|; )' + name + '=([^;]*)'));
    return m ? decodeURIComponent(m[1]) : null;
  }

  // --- the subject and the three context features -------------------------------------------
  var subject;
  var hadCookie = false;
  if (useCookie) {
    subject = readCookie('_prb_subject');
    hadCookie = !!subject;
    if (!subject) subject = newId();
    document.cookie = '_prb_subject=' + encodeURIComponent(subject) + '; Max-Age=31536000; Path=/; SameSite=Lax' +
      (location.protocol === 'https:' ? '; Secure' : '');
  } else {
    subject = store.get('prb_subject') || newId();
    store.set('prb_subject', subject);
  }
  // "Returning" is fixed for the whole browser session: true when the cookie predates it.
  var ret = store.get('prb_returning');
  if (ret === null) {
    ret = hadCookie ? '1' : '0';
    store.set('prb_returning', ret);
  }
  var ctx = {
    returning: ret === '1',
    mobile: /Mobi|Android/i.test(navigator.userAgent || ''),
    from_ad: /[?&](utm_source|utm_medium|utm_campaign|gclid|fbclid|msclkid)=/.test(location.search || ''),
    // The local clock is a decision feature (sin_hour, cos_hour, is_night_local) in Core.
    tz_offset: -new Date().getTimezoneOffset(),
    consent: useCookie
  };
  // Declared features: the page sets window.__prb_features = { name: true|false } before this
  // tag. Only names the manifest declares, only booleans, and a PII one only when the visitor
  // is identified (useCookie is Core's "identified": full, or consent_aware with consent).
  var pageFeatures = window.__prb_features;
  var sentFeatures = {};
  var anyFeature = false;
  // feature:<key> -> whether the on-device scorer may use it: declared, and not a PII one on
  // an unidentified visit. The rest are left out of the vector and the rescore event.
  var usableFeature = {};
  (manifest.features || []).forEach(function (f) {
    if (!f.pii || useCookie) usableFeature['feature:' + f.key] = true;
    if (!pageFeatures || typeof pageFeatures[f.key] !== 'boolean') return;
    if (f.pii && !useCookie) return;
    sentFeatures[f.key] = pageFeatures[f.key];
    anyFeature = true;
  });
  if (anyFeature) ctx.features = sentFeatures;

  // --- stated preferences: picks by picker slot; only the owner's option keys -----------------
  var optionKeys = {};
  slots.forEach(function (s) {
    if (s.kind === 'picker' && s.picker) s.picker.options.forEach(function (o) { optionKeys[o.key] = true; });
  });
  // Persistent only when the site has cookies on; otherwise it dies with the tab.
  var prefs = {
    get: function (k) { try { return (useCookie ? window.localStorage : window.sessionStorage).getItem(k); } catch (e) { return null; } },
    set: function (k, v) { try { (useCookie ? window.localStorage : window.sessionStorage).setItem(k, v); } catch (e) { /* private mode */ } }
  };
  function readJson(raw, fallback) {
    try { var v = JSON.parse(raw); return v && typeof v === 'object' ? v : fallback; } catch (e) { return fallback; }
  }
  var statedBySlot = readJson(prefs.get('prb_stated'), {});
  function statedKeys() {
    var out = [];
    Object.keys(statedBySlot).forEach(function (sk) {
      (statedBySlot[sk] || []).forEach(function (k) { if (optionKeys[k] && out.indexOf(k) < 0) out.push(k); });
    });
    return out;
  }
  function refreshCtx() {
    var st = statedKeys();
    if (st.length) ctx.stated = st; else delete ctx.stated;
  }
  refreshCtx();
  // Implicit intent from clicks: session-only, never sent as `stated`.
  var intent = readJson(store.get('prb_intent'), {});
  var signals = parseInt(store.get('prb_signals') || '0', 10) || 0;

  // --- 1. hide only the slots, before first paint -------------------------------------------
  var style = null;
  if (slots.length) {
    style = document.createElement('style');
    style.id = 'prb-hide';
    style.textContent = slots.map(function (s) { return ':is(' + s.selector + '):not([data-prb-done])'; }).join(',') +
      '{visibility:hidden!important}';
    (document.head || document.documentElement).appendChild(style);
  }
  var timedOut = false;
  function revealAll() {
    timedOut = true;
    if (style && style.parentNode) style.parentNode.removeChild(style);
  }
  var timer = setTimeout(revealAll, timeoutMs);
  var pending = slots.length;
  function settled() {
    pending -= 1;
    if (pending === 0) { clearTimeout(timer); revealAll(); }
  }

  function post(path, body) {
    return window.fetch(base + path, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
      keepalive: true,
      credentials: 'omit'
    });
  }

  function sendAll(evs) {
    evs.forEach(function (ev) { ev.id = newId(); });
    post('e', { site: site, subject: subject, events: evs, tz_offset: -new Date().getTimezoneOffset(), consent: useCookie })['catch'](function () { /* best effort */ });
  }

  function send(name, slotKey, decisionId, extra) {
    var ev = { name: name };
    if (slotKey) ev.slot = slotKey;
    if (decisionId) ev.decision_id = decisionId;
    if (extra) Object.keys(extra).forEach(function (k) { ev[k] = extra[k]; });
    sendAll([ev]);
  }

  function onReady(fn) {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', fn);
    else fn();
  }

  function each(slot, fn) {
    var els = document.querySelectorAll(slot.selector);
    for (var i = 0; i < els.length; i++) fn(els[i]);
  }

  function reveal(slot) {
    each(slot, function (el) { el.setAttribute('data-prb-done', ''); });
  }

  // --- 3. apply a decision ------------------------------------------------------------------
  function apply(slot, d) {
    if (timedOut) {
      // Too late to swap without a visible jump: keep the baseline the visitor already sees.
      store.del(cacheKey(slot));
      send('missed', slot.key, d.decision_id);
      return;
    }
    current[slot.key] = d;
    if (d.explanation && d.explanation.in_holdout === true) heldOut = true;
    if (d.explanation && d.explanation.in_holdout === false) notHeld = true;
    var sorted = slot.kind === 'sorted' && Array.isArray(d.order);
    var shown = [];
    each(slot, function (el) {
      if (!('prbOrig' in el)) el.prbOrig = el.innerHTML;
      if (sorted) shown = reorder(el, d.order);
      else if (d.copy && typeof d.copy === 'object') writeCopy(el, d.copy);
      else if (d.payload !== null && d.payload !== undefined) el.innerHTML = d.payload;
      el.setAttribute('data-prb-done', '');
      el.setAttribute('data-prb-variant', d.variant);
      // Reads the slot's current decision: an on-device swap re-points it.
      el.addEventListener('click', function (e) {
        var item = sorted ? itemOf(e.target, el) : null;
        send('click', slot.key, current[slot.key].decision_id, item ? { items: [{ id: item }] } : null);
      }, true);
      if (slot.kind === 'picker' && d.variant === 'picker' && slot.picker) renderPicker(slot, d, el);
    });
    send('exposure', slot.key, d.decision_id, sorted && shown.length ? { items: shown.slice(0, 100).map(function (k) { return { id: k }; }) } : null);
  }

  // --- a sorted slot (F2b) ------------------------------------------------------------------
  // F4: a copy slot. The page marks each field with data-prb-field="<name>"; the SDK writes the
  // decided text with textContent, never innerHTML, so a field cannot become markup. A field the
  // variant leaves out keeps the page's own text.
  function writeCopy(el, copy) {
    Array.prototype.forEach.call(el.querySelectorAll('[data-prb-field]'), function (f) {
      var v = copy[f.getAttribute('data-prb-field')];
      if (typeof v === 'string') f.textContent = v;
    });
  }

  // The page marks each item with data-prb-item="<key>". The decided items fill the items'
  // places, in order; an item the order leaves out (gated for this visitor) follows them in its
  // page order: the page owns its cards. Other children (an ad tile, a divider, the whitespace
  // between cards) keep their places, as in the edge rewriter and the next SDK.
  // Returns the keys in the order they now stand, the ordered ones first.
  function reorder(el, order) {
    var items = [];
    for (var i = 0; i < el.children.length; i++) {
      if (el.children[i].hasAttribute('data-prb-item')) items.push(el.children[i]);
    }
    if (!items.length) return [];
    var byKey = {};
    items.forEach(function (c) { var k = c.getAttribute('data-prb-item'); if (!(k in byKey)) byKey[k] = c; });
    var placed = [];
    var keys = [];
    order.forEach(function (k) {
      if (byKey[k] && placed.indexOf(byKey[k]) < 0) { placed.push(byKey[k]); keys.push(k); }
    });
    items.forEach(function (c) { if (placed.indexOf(c) < 0) placed.push(c); });
    var marks = items.map(function (c) { var m = document.createComment('prb'); el.insertBefore(m, c); return m; });
    placed.forEach(function (c, n) { el.insertBefore(c, marks[n]); });
    marks.forEach(function (m) { el.removeChild(m); });
    return keys;
  }

  // The item a click landed in, if any, inside the slot element `root`.
  function itemOf(node, root) {
    for (var n = node; n && n !== root; n = n.parentNode) {
      if (n.nodeType === 1 && n.hasAttribute('data-prb-item')) return n.getAttribute('data-prb-item');
    }
    return null;
  }

  // --- the taste picker ---------------------------------------------------------------------
  function renderPicker(slot, d, el) {
    var max = slot.picker.max_picks || 3;
    var chosen = [];
    var box = document.createElement('div');
    box.className = 'prb-picker';
    box.setAttribute('role', 'group');
    var done = document.createElement('button');
    done.type = 'button';
    done.textContent = 'Done';
    done.setAttribute('data-prb-submit', '');
    done.disabled = true;
    slot.picker.options.forEach(function (o) {
      var b = document.createElement('button');
      b.type = 'button';
      b.textContent = o.label;
      b.setAttribute('data-prb-option', o.key);
      b.setAttribute('aria-pressed', 'false');
      b.addEventListener('click', function () {
        var i = chosen.indexOf(o.key);
        if (i >= 0) chosen.splice(i, 1);
        else if (chosen.length < max) chosen.push(o.key);
        b.setAttribute('aria-pressed', chosen.indexOf(o.key) >= 0 ? 'true' : 'false');
        done.disabled = chosen.length === 0;
      });
      box.appendChild(b);
    });
    var skip = document.createElement('button');
    skip.type = 'button';
    skip.textContent = 'Skip';
    skip.setAttribute('data-prb-skip', '');
    function close() { if (box.parentNode) box.parentNode.removeChild(box); }
    done.addEventListener('click', function () {
      if (!chosen.length) return;
      send('pick', slot.key, d.decision_id, { picks: chosen.slice() });
      statedBySlot[slot.key] = chosen.slice();
      prefs.set('prb_stated', JSON.stringify(statedBySlot));
      refreshCtx();
      close();
      signal();
    });
    skip.addEventListener('click', function () {
      send('skip', slot.key, d.decision_id);
      close();
    });
    box.appendChild(done);
    box.appendChild(skip);
    el.appendChild(box);
  }

  // --- in-session re-scoring on the device --------------------------------------------------
  var current = {};
  var heldOut = false;
  var notHeld = false;
  var pointerAt = null;
  document.addEventListener('mouseover', function (e) { pointerAt = e.target; }, true);
  document.addEventListener('pointermove', function (e) { pointerAt = e.target; }, true);
  document.addEventListener('click', function (e) {
    var t = e.target && e.target.closest ? e.target.closest('[data-prb-intent]') : null;
    var k = t && t.getAttribute('data-prb-intent');
    if (!k || !optionKeys[k]) return;
    intent[k] = Math.min(1, (intent[k] || 0) + 0.5);
    store.set('prb_intent', JSON.stringify(intent));
    signal();
  }, true);

  function signal() {
    signals += 1;
    store.set('prb_signals', String(signals));
    var raf = window.requestAnimationFrame || function (f) { return setTimeout(f, 16); };
    raf(rescore);
  }

  function features(names) {
    var st = statedKeys();
    var clock = localClock();
    var x = {};
    names.forEach(function (n) {
      if (n === 'bias') x[n] = 1;
      else if (n === 'returning' || n === 'mobile' || n === 'from_ad') x[n] = ctx[n] ? 1 : 0;
      else if (n.indexOf('stated:') === 0) {
        var k = n.slice(7);
        x[n] = st.indexOf(k) >= 0 ? 1 : (intent[k] || 0);
      } else if (n.indexOf('feature:') === 0) {
        if (usableFeature[n]) x[n] = sentFeatures[n.slice(8)] ? 1 : 0;
      } else if (n in clock) x[n] = clock[n];
      else x[n] = 0;
    });
    return x;
  }

  // The time features, from the browser clock as Core computes them from tz_offset: the hour's
  // sine and cosine, night by the site's window ([start, end), wrapping midnight), weekend, and
  // Monday-Friday 09:00-17:00.
  function localClock() {
    var d = new Date();
    var h = d.getHours();
    var dow = d.getDay();
    var nw = manifest.night || { start: 22, end: 5 };
    var a = 2 * Math.PI * h / 24;
    var weekend = dow === 0 || dow === 6;
    var night = nw.start <= nw.end ? (h >= nw.start && h < nw.end) : (h >= nw.start || h < nw.end);
    return {
      sin_hour: Math.sin(a),
      cos_hour: Math.cos(a),
      is_night_local: night ? 1 : 0,
      is_weekend_local: weekend ? 1 : 0,
      is_business_hours_local: !weekend && h >= 9 && h < 17 ? 1 : 0,
    };
  }

  // Never under the visitor's hand or eyes: not in the viewport, not focused, not hovered.
  function safeToSwap(el) {
    if (el.contains(document.activeElement) && document.activeElement !== document.body) return false;
    if (pointerAt && el.contains(pointerAt)) return false;
    try { if (el.matches(':hover')) return false; } catch (e) { /* no :hover support */ }
    var r = el.getBoundingClientRect();
    var vh = window.innerHeight || document.documentElement.clientHeight;
    var vw = window.innerWidth || document.documentElement.clientWidth;
    // A zero rect (unlaid-out, or a test DOM) counts as on screen: when in doubt, do not swap.
    return r.top >= vh || r.bottom < 0 || r.left >= vw || r.right < 0;
  }

  function rescore() {
    // A holdout visitor sees the baseline whatever happens; unknown status means wait.
    if (heldOut || !notHeld || signals < adaptMin) return;
    slots.forEach(function (slot) {
      if (!slot.in_session || slot.kind === 'picker') return;
      var vs = (slot.variants || []).filter(function (v) { return v.weights; });
      if (!vs.length) return;
      var x = features(Object.keys(vs[0].weights));
      var best = null;
      vs.forEach(function (v) {
        var z = 0;
        Object.keys(v.weights).forEach(function (n) { z += v.weights[n] * (x[n] || 0); });
        if (!best || z > best.z) best = { v: v, z: z };
      });
      var els = document.querySelectorAll(slot.selector);
      var now = current[slot.key];
      if (els.length && !now) return; // its server decision has not landed yet
      var shown = now ? now.variant : null;
      if (best.v.key === shown) return;
      var cachedNow = cached(slot);
      if (!els.length && cachedNow && cachedNow.variant === best.v.key) return;
      var d = {
        decision_id: newId(), slot: slot.key, variant: best.v.key,
        payload: best.v.payload === undefined ? null : best.v.payload,
        explanation: { policy: 'on_device', features: x, in_holdout: false }
      };
      var score = 1 / (1 + Math.exp(-best.z));
      var evs = [{ name: 'rescore', slot: slot.key, decision_id: d.decision_id, variant: d.variant, features: x, score: score }];
      var all = els.length > 0;
      for (var i = 0; i < els.length; i++) if (!safeToSwap(els[i])) all = false;
      // Next navigation gets it either way; this page only when every copy is safe to swap.
      store.set(cacheKey(slot), JSON.stringify({ subject: subject, at: Date.now(), d: d }));
      if (all) {
        for (var j = 0; j < els.length; j++) {
          els[j].innerHTML = d.payload === null ? (els[j].prbOrig || '') : d.payload;
          els[j].setAttribute('data-prb-variant', d.variant);
        }
        current[slot.key] = d;
        evs.push({ name: 'exposure', slot: slot.key, decision_id: d.decision_id });
      }
      sendAll(evs);
    });
  }

  // --- 2. decide (or reuse a cached decision) -----------------------------------------------
  function cacheKey(slot) { return 'prb_d_' + site + '_' + slot.key; }

  function cached(slot) {
    var raw = store.get(cacheKey(slot));
    if (!raw) return null;
    try {
      var c = JSON.parse(raw);
      var ttl = (slot.ttl_secs || 0) * 1000;
      if (c.subject === subject && Date.now() - c.at < ttl) return c.d;
    } catch (e) { /* corrupt entry: decide again */ }
    return null;
  }

  slots.forEach(function (slot) {
    var hit = cached(slot);
    var p = hit ? Promise.resolve(hit) : post('decide', { site: site, slot: slot.key, subject: subject, ctx: ctx })
      .then(function (r) {
        if (!r.ok) throw new Error('decide ' + r.status);
        return r.json();
      })
      .then(function (d) {
        store.set(cacheKey(slot), JSON.stringify({ subject: subject, at: Date.now(), d: d }));
        return d;
      });
    p.then(
      function (d) { onReady(function () { apply(slot, d); settled(); }); },
      function () { onReady(function () { reveal(slot); settled(); }); }
    );
  });

  // --- commerce (F1) -------------------------------------------------------------------------
  // Checked here as Core checks them, so a malformed order fails in the developer's console, not
  // silently at Core. Core derives a purchase's event id from order_id: sending one order twice
  // (a reloaded thank-you page) stores it once.
  function bad(msg) { throw new TypeError('[pratibimba] ' + msg); }
  function checkItems(items) {
    if (items === undefined) return undefined;
    if (!Array.isArray(items)) bad('items must be an array of {id, price?, quantity?}');
    items.forEach(function (it) {
      // Core counts bytes (UTF-8), so a 200-character id of accented letters can be too long.
      if (!it || typeof it.id !== 'string' || !it.id || new TextEncoder().encode(it.id).length > 256) bad('each item needs a string id (1-256 bytes)');
      if (it.price !== undefined && !(typeof it.price === 'number' && isFinite(it.price) && it.price >= 0)) bad('item price must be a number >= 0');
      if (it.quantity !== undefined && !(typeof it.quantity === 'number' && it.quantity >= 1 && Math.floor(it.quantity) === it.quantity)) bad('item quantity must be a whole number >= 1');
    });
    return items;
  }

  window.prb = {
    subject: subject,
    // A goal with no decision id: Core credits the subject's latest exposed decision per slot
    // whose metric is `name` (last touch).
    track: function (name) { send(name); },
    // `add_to_cart` with the item: a goal like any other (a slot may measure it); its price is
    // not revenue.
    addToCart: function (item) {
      send('add_to_cart', null, null, { items: checkItems([item]) });
    },
    // `purchase` with the order: Core shares its value among the decisions in each slot's window.
    purchase: function (order) {
      var o = order || {};
      if (typeof o.order_id !== 'string' || !o.order_id || o.order_id.length > 256) bad('purchase needs an order_id (1-256 chars)');
      if (!(typeof o.value === 'number' && isFinite(o.value) && o.value >= 0)) bad('purchase value must be a number >= 0, in major units (49.99)');
      if (typeof o.currency !== 'string' || !/^[A-Za-z]{3}$/.test(o.currency)) bad('purchase currency must be an ISO 4217 code, e.g. USD');
      var extra = { order_id: o.order_id, value: o.value, currency: o.currency.toUpperCase() };
      var items = checkItems(o.items);
      if (items) extra.items = items;
      send('purchase', null, null, extra);
    }
  };
})();

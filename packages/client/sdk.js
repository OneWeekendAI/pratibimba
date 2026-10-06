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
 * The subject lives in sessionStorage (cookieless) unless the tag has data-cookie="on", which
 * sets a first-party `_prb_subject` cookie for a year. Get consent before turning that on.
 */
(function () {
  'use strict';
  var script = document.currentScript;
  var manifest = window.__prb_manifest;
  if (!script || !manifest || !manifest.slots) return;

  var site = script.getAttribute('data-site') || manifest.site;
  var base = script.getAttribute('data-base') || (script.src || '').replace(/sdk\.js(\?.*)?$/, '');
  var timeoutMs = parseInt(script.getAttribute('data-timeout') || '400', 10);
  var useCookie = script.getAttribute('data-cookie') === 'on';
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
    from_ad: /[?&](utm_source|utm_medium|utm_campaign|gclid|fbclid|msclkid)=/.test(location.search || '')
  };

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

  function send(name, slotKey, decisionId) {
    var ev = { id: newId(), name: name };
    if (slotKey) ev.slot = slotKey;
    if (decisionId) ev.decision_id = decisionId;
    post('e', { site: site, subject: subject, events: [ev] })['catch'](function () { /* best effort */ });
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
    each(slot, function (el) {
      if (d.payload !== null && d.payload !== undefined) el.innerHTML = d.payload;
      el.setAttribute('data-prb-done', '');
      el.setAttribute('data-prb-variant', d.variant);
      el.addEventListener('click', function () { send('click', slot.key, d.decision_id); }, true);
    });
    send('exposure', slot.key, d.decision_id);
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

  window.prb = {
    subject: subject,
    // A goal with no decision id: Core credits the subject's latest exposed decision per slot
    // whose metric is `name` (last touch).
    track: function (name) { send(name); }
  };
})();

// The browser half: `/_prb/beacon.js`, served by the edge handler itself (same origin, so a
// strict CSP with 'self' allows it). It reads the decisions the edge wrote into its own tag,
// posts one exposure per decision once the page has loaded, and posts a click for any click
// between a slot's two marker comments. No wrapper element is ever injected around a slot.

export const BEACON_JS = `(function () {
  var me = document.currentScript;
  if (!me) return;
  var cfg = JSON.parse(me.getAttribute('data-prb') || '{}');
  // items: a sorted slot's cards, [{id}] (the rendered order on exposure, the card on a click).
  function send(name, d, items) {
    var ev = { id: (crypto.randomUUID ? crypto.randomUUID() : String(Date.now()) + Math.random()), name: name, slot: d.slot, decision_id: d.id };
    if (items && items.length) ev.items = items.slice(0, 100).map(function (k) { return { id: k }; });
    var body = JSON.stringify({ site: cfg.site, subject: cfg.subject,
      events: [ev],
      tz_offset: -new Date().getTimezoneOffset(), consent: cfg.consent === true });
    try { fetch('/_prb/e', { method: 'POST', headers: { 'content-type': 'application/json' }, body: body, keepalive: true }); } catch (e) {}
  }
  // The edge has no clock of the visitor's: this first-party cookie names the browser's zone,
  // which the edge prefers to its geolocation. It is device storage, so it is kept (a year) only
  // when the site may remember this visitor (cfg.remember: full, or consent_aware with consent);
  // otherwise nothing is written and a cookie left from an earlier choice is expired.
  try {
    var secure = location.protocol === 'https:' ? '; Secure' : '';
    if (cfg.remember === true) {
      var zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
      if (zone) document.cookie = '_prb_tz=' + encodeURIComponent(zone) + '; Path=/; SameSite=Lax; Max-Age=31536000' + secure;
    } else if (/(?:^|; )_prb_tz=/.test(document.cookie)) {
      document.cookie = '_prb_tz=; Path=/; SameSite=Lax; Max-Age=0' + secure;
    }
  } catch (e) {}
  function markers(slot) {
    var w = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_COMMENT);
    var start = null, n;
    while ((n = w.nextNode())) {
      var t = n.nodeValue.trim();
      if (!start && t === 'prb:slot="' + slot + '"') start = n;
      else if (start && t === '/prb:slot') return [start, n];
    }
    return null;
  }
  function inside(target, pair) {
    return !!(pair[0].compareDocumentPosition(target) & Node.DOCUMENT_POSITION_FOLLOWING) &&
           !!(pair[1].compareDocumentPosition(target) & Node.DOCUMENT_POSITION_PRECEDING);
  }
  function start() {
    (cfg.decisions || []).forEach(function (d) {
      var pair = markers(d.slot);
      if (!pair) return;
      var sorted = Array.isArray(d.items);
      send('exposure', d, sorted ? d.items : null);
      document.addEventListener('click', function (ev) {
        if (!inside(ev.target, pair)) return;
        // The card a click landed in, if it sits between the markers (a sorted slot only).
        var card = sorted && ev.target.closest ? ev.target.closest('[data-prb-item]') : null;
        send('click', d, card && inside(card, pair) ? [card.getAttribute('data-prb-item')] : null);
      }, true);
    });
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start); else start();
})();
`;

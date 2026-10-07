// The browser half: `/_prb/beacon.js`, served by the edge handler itself (same origin, so a
// strict CSP with 'self' allows it). It reads the decisions the edge wrote into its own tag,
// posts one exposure per decision once the page has loaded, and posts a click for any click
// between a slot's two marker comments. No wrapper element is ever injected around a slot.

export const BEACON_JS = `(function () {
  var me = document.currentScript;
  if (!me) return;
  var cfg = JSON.parse(me.getAttribute('data-prb') || '{}');
  function send(name, d) {
    var body = JSON.stringify({ site: cfg.site, subject: cfg.subject,
      events: [{ id: (crypto.randomUUID ? crypto.randomUUID() : String(Date.now()) + Math.random()), name: name, slot: d.slot, decision_id: d.id }],
      tz_offset: -new Date().getTimezoneOffset(), consent: cfg.consent === true });
    try { fetch('/_prb/e', { method: 'POST', headers: { 'content-type': 'application/json' }, body: body, keepalive: true }); } catch (e) {}
  }
  // The edge has no clock of the visitor's: this first-party session cookie is its fallback
  // when the platform sends no geolocation time zone.
  try {
    var zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    var pair = '_prb_tz=' + encodeURIComponent(zone);
    if (zone && document.cookie.split('; ').indexOf(pair) < 0)
      document.cookie = pair + '; Path=/; SameSite=Lax' + (location.protocol === 'https:' ? '; Secure' : '');
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
      send('exposure', d);
      document.addEventListener('click', function (ev) { if (inside(ev.target, pair)) send('click', d); }, true);
    });
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start); else start();
})();
`;

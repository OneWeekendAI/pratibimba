// S13: preview mode. A page opened with ?prb_preview=<token> renders what the room picks and
// leaves no trace: no decide, no event, no subject, no cookie, no cached decision.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, ok, sleep } from './harness.mjs';

const ROOM = 'http://dash.test';
const b64 = (o) => Buffer.from(JSON.stringify(o)).toString('base64url');
const TOKEN = `${b64({ site: 'demo', exp: 9999999999, parent: ROOM })}.sig`;
const versions = (ctx) => ({
  site: 'demo', parent: ROOM,
  slots: [{
    slot: 'hero', kind: 'swap', kept_out: ctx.returning ? [] : [{ key: 'welcome', why: 'only for Returning visitors' }],
    versions: [
      ...(ctx.returning ? [{ id: 3, key: 'welcome', status: 'approved', baseline: false, payload: '<h1>Welcome back</h1>', why: 'made for Returning visitors' }] : []),
      { id: 1, key: 'own', status: 'approved', baseline: true, payload: null, why: "the page's own version" },
      { id: 2, key: 'spring', status: 'draft', baseline: false, payload: '<h1>Spring</h1>', why: 'draft, not live yet; open to every visitor' },
    ],
  }],
});

function room(opts = {}) {
  const sent = [];
  const fake = { postMessage: (msg, origin) => sent.push({ msg, origin }) };
  const p = load({
    url: 'https://shop.test/?prb_preview=' + encodeURIComponent(opts.token || TOKEN),
    preview: (body) => (opts.refuse ? Promise.resolve({ ok: false, status: 401, json: () => Promise.resolve({}) }) : ok(versions(body.ctx || {}))),
    before: (w) => Object.defineProperty(w, 'parent', { value: fake, configurable: true }),
  });
  p.sent = sent;
  p.say = (data, origin = ROOM, source = fake) => {
    const ev = new p.w.MessageEvent('message', { data, origin });
    Object.defineProperty(ev, 'source', { value: source });
    p.w.dispatchEvent(ev);
  };
  p.previews = () => p.calls.filter((c) => c.url.endsWith('/preview'));
  p.states = () => sent.filter((s) => s.msg.type === 'prb:state').map((s) => s.msg);
  return p;
}

test('preview_sends_no_decide_no_event_and_keeps_no_subject', async () => {
  const p = room();
  await sleep(30);
  p.say({ type: 'prb:visitor', ctx: { returning: true } });
  await sleep(30);
  p.say({ type: 'prb:pick', slot: 'hero', index: 2 });
  p.w.document.getElementById('hero').dispatchEvent(new p.w.MouseEvent('click', { bubbles: true }));
  await sleep(30);
  assert.equal(p.decides().length, 0, 'preview called /decide');
  assert.equal(p.events().length, 0, 'preview sent events');
  assert.ok(p.calls.every((c) => c.url.endsWith('/preview')), `only /preview: ${p.calls.map((c) => c.url)}`);
  assert.equal(p.w.document.cookie, '', 'preview set a cookie');
  assert.equal(p.w.sessionStorage.getItem('prb_subject'), null);
  assert.ok(Object.keys(p.w.sessionStorage).every((k) => k === 'prb_preview'), `stored ${Object.keys(p.w.sessionStorage)}`);
  assert.ok(Object.keys(p.w.localStorage).length === 0);
});

test('preview_renders_the_first_version_and_the_room_flips_it', async () => {
  const p = room();
  await sleep(30);
  assert.equal(p.hero().textContent, 'Original', 'first visit: the page’s own version first');
  assert.equal(p.hero().getAttribute('data-prb-preview'), 'many');
  assert.equal(p.visibility('hero'), 'visible');
  p.say({ type: 'prb:visitor', ctx: { returning: true } });
  await sleep(30);
  assert.equal(p.hero().textContent, 'Welcome back');
  assert.ok(p.hero().hasAttribute('data-prb-changed'), 'the switch changed the hero');
  const last = p.states().at(-1);
  assert.equal(last.slots[0].changed, true);
  assert.deepEqual(last.slots[0].versions.map((v) => v.key), ['welcome', 'own', 'spring']);
  p.say({ type: 'prb:pick', slot: 'hero', index: 1 });
  assert.equal(p.hero().textContent, 'Original', 'back to the page’s own content, not built on the last version');
  p.say({ type: 'prb:pick', slot: 'hero', index: 2 });
  assert.equal(p.hero().textContent, 'Spring');
  p.say({ type: 'prb:pick', slot: 'hero', index: 3 });
  assert.equal(p.hero().textContent, 'Welcome back', 'Next wraps around');
  assert.equal(p.sent.every((s) => s.origin === ROOM), true, 'posts only to the room');
});

// B4: the room's persona travels to /preview; a message without one sends none.
// Mutation: drop `persona` from the /preview body -> red.
test('preview_forwards_the_rooms_persona', async () => {
  const p = room();
  await sleep(30);
  assert.equal(p.previews()[0].body.persona, undefined, 'no persona before the room names one');
  p.say({ type: 'prb:visitor', ctx: { returning: true }, persona: 'usual_rajma' });
  await sleep(30);
  assert.equal(p.previews().at(-1).body.persona, 'usual_rajma');
  p.say({ type: 'prb:visitor', ctx: {}, persona: 42 });
  await sleep(30);
  assert.equal(p.previews().at(-1).body.persona, undefined, 'a non-string persona is dropped');
});

// B4: the same copy version, refilled for another persona, is written again.
// Mutation: versionKey of a copy version is its key alone -> red.
test('a_persona_switch_rewrites_the_shown_copy_version', async () => {
  const sent = [];
  const fake = { postMessage: (msg) => sent.push(msg) };
  const title = (persona) => (persona === 'usual_rajma' ? 'Your usual rajma-chawal' : 'Your usual thali');
  const p = load({
    url: 'https://shop.test/?prb_preview=' + encodeURIComponent(TOKEN),
    body: '<div id="hero"><h1 data-prb-field="title">Original</h1></div>',
    preview: (body) => ok({ site: 'demo', parent: ROOM, slots: [{ slot: 'hero', kind: 'copy', kept_out: [], versions: [
      // An open version comes before the page's own, so it is the one shown after a switch.
      { id: 2, key: 'usual', status: 'approved', baseline: false, copy: { title: title(body.persona) }, filled: true, why: 'open to every visitor' },
      { id: 1, key: 'own', status: 'approved', baseline: true, copy: null, why: "the page's own version" },
    ] }] }),
    before: (w) => Object.defineProperty(w, 'parent', { value: fake, configurable: true }),
  });
  const say = (data) => {
    const ev = new p.w.MessageEvent('message', { data, origin: ROOM });
    Object.defineProperty(ev, 'source', { value: fake });
    p.w.dispatchEvent(ev);
  };
  const h1 = () => p.w.document.querySelector('#hero h1').textContent;
  await sleep(30);
  assert.equal(h1(), 'Your usual thali');
  say({ type: 'prb:visitor', ctx: {}, persona: 'usual_rajma' });
  await sleep(30);
  assert.equal(h1(), 'Your usual rajma-chawal');
});

test('a_click_on_a_slot_opens_it_in_the_room_and_does_not_follow_the_link', async () => {
  const p = room();
  await sleep(30);
  const ev = new p.w.MouseEvent('click', { bubbles: true, cancelable: true });
  p.w.document.getElementById('cta').dispatchEvent(ev);
  assert.equal(ev.defaultPrevented, true);
  assert.equal(p.states().at(-1).picked, 'hero');
});

test('preview_ignores_messages_from_any_other_window_or_origin', async () => {
  const p = room();
  await sleep(30);
  p.say({ type: 'prb:visitor', ctx: { returning: true } }, 'https://evil.test');
  p.say({ type: 'prb:visitor', ctx: { returning: true } }, ROOM, p.w);
  await sleep(30);
  assert.equal(p.previews().length, 1, 'a stranger made the page fetch again');
  assert.equal(p.hero().textContent, 'Original');
});

test('a_refused_token_shows_the_page_and_tells_the_room', async () => {
  const p = room({ refuse: true });
  await sleep(30);
  assert.equal(p.decides().length, 0, 'never falls back to production');
  assert.equal(p.visibility('hero'), 'visible');
  assert.ok(p.sent.some((s) => s.msg.type === 'prb:expired'));
});

test('without_the_parameter_the_page_never_previews', async () => {
  const p = load({ preview: () => ok(versions({})) });
  await sleep(30);
  assert.equal(p.calls.filter((c) => c.url.endsWith('/preview')).length, 0);
  assert.equal(p.decides().length, 1);
});

// The room's explorer: prb:view zones outlines and labels EVERY slot (one version or many), and
// a click on any of them opens it. prb:view marks:false (the room's thumbnails) draws nothing.
function explorer() {
  const sent = [];
  const fake = { postMessage: (msg, origin) => sent.push({ msg, origin }) };
  const manifest = {
    site: 'demo',
    slots: [
      { key: 'hero', selector: '#hero', metric: 'click', ttl_secs: 60, variants: [] },
      { key: 'promo', selector: '#promo', metric: 'click', ttl_secs: 60, variants: [] },
    ],
  };
  const data = {
    site: 'demo', parent: ROOM,
    slots: [
      versions({}).slots[0],
      { slot: 'promo', kind: 'swap', kept_out: [], versions: [{ id: 9, key: 'own', status: 'approved', baseline: true, payload: null, why: "the page's own version" }] },
    ],
  };
  const p = load({
    url: 'https://shop.test/?prb_preview=' + encodeURIComponent(TOKEN),
    manifest,
    preview: () => ok(data),
    before: (w) => {
      Object.defineProperty(w, 'parent', { value: fake, configurable: true });
      // jsdom has no layout: give every element a box, so slots and their chips have a place.
      w.HTMLElement.prototype.getBoundingClientRect = () => ({ top: 10, left: 10, width: 100, height: 50, bottom: 60, right: 110 });
    },
  });
  p.say = (msg) => {
    const ev = new p.w.MessageEvent('message', { data: msg, origin: ROOM });
    Object.defineProperty(ev, 'source', { value: fake });
    p.w.dispatchEvent(ev);
  };
  p.sent = sent;
  p.mark = (id) => {
    const el = p.w.document.getElementById(id);
    return [el, ...el.querySelectorAll('[data-prb-mark]')].map((x) => x.getAttribute('data-prb-mark') || '').join(' ').trim();
  };
  p.chips = () => [...p.w.document.querySelectorAll('#prb-zones [data-prb-zone]')].map((c) => c.textContent);
  p.last = () => sent.filter((s) => s.msg.type === 'prb:state').at(-1).msg;
  return p;
}

test('the_room_hears_which_messages_this_sdk_understands', async () => {
  const p = explorer();
  await sleep(30);
  assert.deepEqual([...p.sent.find((s) => s.msg.type === 'prb:hello').msg.can], ['view']);
});

test('zones_outline_and_label_every_slot_and_any_one_opens_on_a_click', async () => {
  const p = explorer();
  await sleep(30);
  assert.equal(p.mark('promo'), '', 'before zones, a one-version slot is not marked');
  assert.deepEqual(p.chips(), []);
  p.say({ type: 'prb:view', zones: true });
  assert.deepEqual([...p.w.document.querySelectorAll('#prb-zones [data-prb-zone-box]')].map((b) => b.getAttribute('data-prb-zone-box')), ['hero', 'promo'], 'one box per slot');
  assert.match(p.mark('hero'), /\bzone\b/);
  assert.match(p.mark('promo'), /\bzone\b/, 'a one-version slot is a compartment too');
  assert.deepEqual(p.chips(), ['hero', 'promo']);
  p.w.document.getElementById('promo').dispatchEvent(new p.w.MouseEvent('mouseover', { bubbles: true }));
  assert.match(p.mark('promo'), /\bhot\b/);
  assert.doesNotMatch(p.mark('hero'), /\bhot\b/);
  p.w.document.getElementById('promo').dispatchEvent(new p.w.MouseEvent('click', { bubbles: true }));
  assert.equal(p.last().picked, 'promo', 'a click on a one-version slot opens it in zones');
  assert.match(p.mark('promo'), /\bpicked\b/);
  await sleep(40);
  assert.deepEqual(p.chips(), ['hero', 'promo'], 'the chips survive their own redraw');
  p.say({ type: 'prb:view', zones: false });
  assert.equal(p.w.document.getElementById('prb-zones'), null, 'zones off removes the chips');
  assert.doesNotMatch(p.mark('promo'), /\bzone\b/);
});

test('marks_off_draws_nothing_but_the_pick_still_renders', async () => {
  const p = explorer();
  await sleep(30);
  p.say({ type: 'prb:view', marks: false });
  p.say({ type: 'prb:pick', slot: 'hero', index: 1 });
  assert.equal(p.hero().textContent, 'Spring');
  assert.equal(p.mark('hero'), '', 'a thumbnail frame shows no outline');
  assert.equal(p.w.document.getElementById('prb-preview-marks').textContent, '');
  assert.equal(p.last().picked, 'hero');
});

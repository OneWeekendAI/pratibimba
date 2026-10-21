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

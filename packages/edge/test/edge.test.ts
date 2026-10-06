import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createEdge } from '../src/index.ts';

const PAGE = '<html><body><!-- prb:slot="hero" --><h1>Espresso</h1><!-- /prb:slot --></body></html>';

function html(body = PAGE, extra: Record<string, string> = {}) {
  return async () => new Response(body, { headers: { 'content-type': 'text/html; charset=utf-8', 'content-length': String(body.length), etag: '"x"', 'cache-control': 'public, max-age=600', ...extra } });
}

function core(answer: (slot: string) => Response | Error, calls: any[] = []) {
  return (async (url: string, init: any) => {
    calls.push({ url, init, body: init?.body ? JSON.parse(init.body) : null });
    const r = answer(calls.at(-1).body?.slot);
    if (r instanceof Error) throw r;
    return r;
  }) as unknown as typeof fetch;
}

const gift = () => Response.json({ decision_id: 'd-1', slot: 'hero', variant: 'gift', payload: '<h1>Gifts</h1>', explanation: {} });

function edge(origin = html(), fetch = core(gift), extra = {}) {
  return createEdge({ core: 'http://core:8080', site: 'shop', origin, fetch, ...extra });
}

const get = (path = '/', headers: Record<string, string> = {}) => new Request(`https://shop.test${path}`, { headers });

test('edge_replaces_the_slot_before_the_browser_sees_it', async () => {
  const res = await edge().handleRequest(get());
  const text = await res.text();
  assert.match(text, /<!-- prb:slot="hero" --><h1>Gifts<\/h1><!-- \/prb:slot -->/);
  assert.doesNotMatch(text, /Espresso/);
});

test('edge_sends_subject_and_context_to_core', async () => {
  const calls: any[] = [];
  await (await edge(html(), core(gift, calls)).handleRequest(get('/?gclid=1', { cookie: '_prb_subject=abc', 'user-agent': 'iPhone' }))).text();
  assert.deepEqual(calls[0].body, { site: 'shop', slot: 'hero', subject: 'abc', ctx: { returning: false, mobile: true, from_ad: true } });
});

test('edge_keeps_the_baseline_when_core_fails', async () => {
  const res = await edge(html(), core(() => new Error('ECONNREFUSED'))).handleRequest(get());
  const text = await res.text();
  assert.match(text, /<h1>Espresso<\/h1>/);
  assert.doesNotMatch(text, /beacon\.js/, 'a beacon was added for a slot that was not decided');
});

test('edge_keeps_the_baseline_for_an_unknown_slot', async () => {
  const res = await edge(html(), core(() => Response.json({ error: 'unknown slot' }, { status: 404 }))).handleRequest(get());
  assert.match(await res.text(), /<h1>Espresso<\/h1>/);
});

test('edge_passes_non_html_through', async () => {
  const calls: any[] = [];
  const json = async () => Response.json({ a: 1 }, { headers: { 'cache-control': 'public, max-age=60' } });
  const res = await edge(json, core(gift, calls)).handleRequest(get('/api'));
  assert.deepEqual(await res.json(), { a: 1 });
  assert.equal(res.headers.get('cache-control'), 'public, max-age=60');
  assert.equal(calls.length, 0);
});

test('rewritten_html_is_never_shared_cached', async () => {
  const res = await edge().handleRequest(get());
  assert.equal(res.headers.get('cache-control'), 'private, no-store');
  assert.equal(res.headers.get('content-length'), null);
  assert.equal(res.headers.get('etag'), null);
});

test('new_visitor_gets_a_session_subject_cookie', async () => {
  const res = await edge().handleRequest(get());
  const c = res.headers.getSetCookie().find((c) => c.startsWith('_prb_subject='))!;
  assert.ok(c, 'no subject cookie');
  assert.doesNotMatch(c, /Max-Age/, 'persisted without opt-in');
  assert.match(c, /HttpOnly/);
});

test('persist_opt_in_keeps_the_subject_a_year', async () => {
  const res = await edge(html(), core(gift), { persist: true }).handleRequest(get());
  assert.match(res.headers.getSetCookie().find((c) => c.startsWith('_prb_subject='))!, /Max-Age=31536000/);
});

test('beacon_tag_carries_the_decisions', async () => {
  const text = await (await edge().handleRequest(get('/', { cookie: '_prb_subject=abc' }))).text();
  const m = /<script src="\/_prb\/beacon\.js" data-prb='([^']+)'><\/script><\/body>/.exec(text);
  assert.ok(m, text);
  assert.deepEqual(JSON.parse(m[1]), { site: 'shop', subject: 'abc', decisions: [{ slot: 'hero', id: 'd-1' }] });
});

test('edge_serves_the_beacon_script', async () => {
  const res = await edge().handleRequest(get('/_prb/beacon.js'));
  assert.match(res.headers.get('content-type')!, /javascript/);
  assert.match(await res.text(), /prb:slot=/);
});

test('edge_forwards_the_public_surface', async () => {
  const calls: any[] = [];
  const res = await edge(html(), core(() => Response.json({ site: 'shop' }), calls)).handleRequest(get('/_prb/manifest/shop'));
  assert.equal(res.status, 200);
  assert.equal(calls[0].url, 'http://core:8080/manifest/shop');
});

test('edge_refuses_the_dashboard_api', async () => {
  const calls: any[] = [];
  const res = await edge(html(), core(gift, calls)).handleRequest(get('/_prb/api/sites'));
  assert.equal(res.status, 404);
  assert.equal(calls.length, 0);
});

test('edge_names_itself_for_the_probe', async () => {
  const res = await edge().handleRequest(get());
  assert.equal(res.headers.get('x-prb-sdk'), 'edge/0.1.0');
});

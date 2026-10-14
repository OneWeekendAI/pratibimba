import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRoute } from '../src/route.ts';

function upstream(calls: { url: string; init: any }[]) {
  return (async (url: string, init: any) => {
    calls.push({ url, init });
    return new Response('{"ok":true}', { status: 200, headers: { 'content-type': 'application/json', 'set-cookie': 'leak=1' } });
  }) as unknown as typeof fetch;
}

test('route_forwards_the_manifest', async () => {
  const calls: any[] = [];
  const r = createRoute({ core: 'http://core:8080', fetch: upstream(calls) });
  const res = await r.GET(new Request('https://shop.test/_prb/manifest/shop.js'));
  assert.equal(res.status, 200);
  assert.equal(calls[0].url, 'http://core:8080/manifest/shop.js');
});

test('route_forwards_events_with_their_body', async () => {
  const calls: any[] = [];
  const r = createRoute({ core: 'http://core:8080/', fetch: upstream(calls) });
  await r.POST(new Request('https://shop.test/_prb/e', { method: 'POST', body: '{"site":"shop"}' }));
  assert.equal(calls[0].url, 'http://core:8080/e');
  assert.equal(calls[0].init.body, '{"site":"shop"}');
});

test('route_refuses_the_dashboard_api', async () => {
  const calls: any[] = [];
  const r = createRoute({ core: 'http://core', fetch: upstream(calls) });
  const res = await r.GET(new Request('https://shop.test/_prb/api/sites'));
  assert.equal(res.status, 404);
  assert.equal(calls.length, 0, 'the admin API was proxied');
});

test('route_refuses_a_wrong_method', async () => {
  const calls: any[] = [];
  const r = createRoute({ core: 'http://core', fetch: upstream(calls) });
  assert.equal((await r.GET(new Request('https://shop.test/_prb/e'))).status, 404);
  assert.equal(calls.length, 0);
});

test('route_forwards_no_cookies_either_way', async () => {
  const calls: any[] = [];
  const r = createRoute({ core: 'http://core', fetch: upstream(calls) });
  const res = await r.POST(new Request('https://shop.test/_prb/e', { method: 'POST', body: '{}', headers: { cookie: 'session=secret' } }));
  assert.equal(JSON.stringify(calls[0].init.headers).includes('secret'), false, 'a cookie went to Core');
  assert.equal(res.headers.get('set-cookie'), null, 'Core set a cookie on the site');
});

// F2 live run: the shop's relay to the Worker threw `fetch failed` 14 times (8 ECONNRESET, 6 IPv6
// connect timeouts) and Next answered each with an opaque 500. A reset can come after the Worker
// took the event, so /e is retried once with the SAME body (Core ignores a repeated event id);
// anything still unreachable is a 502 the caller can read, never a 500.
function flaky(calls: { url: string; init: any }[], failures: number) {
  return (async (url: string, init: any) => {
    calls.push({ url, init });
    if (calls.length <= failures) throw new TypeError('fetch failed', { cause: Object.assign(new Error('read ECONNRESET'), { code: 'ECONNRESET' }) });
    return new Response('{"queued":1}', { status: 202, headers: { 'content-type': 'application/json' } });
  }) as unknown as typeof fetch;
}

test('route_retries_a_reset_event_post_once_with_the_same_body', async () => {
  const calls: any[] = [];
  const r = createRoute({ core: 'http://core', fetch: flaky(calls, 1) });
  const body = '{"site":"shop","events":[{"id":"7d1f0f3e-0000-4000-8000-000000000001","name":"click"}]}';
  const res = await r.POST(new Request('https://shop.test/_prb/e', { method: 'POST', body }));
  assert.equal(res.status, 202);
  assert.equal(calls.length, 2);
  assert.equal(calls[1].init.body, body, 'the retry must carry the same event ids, or Core counts it twice');
});

test('route_answers_502_not_500_when_the_upstream_is_unreachable', async () => {
  const calls: any[] = [];
  const r = createRoute({ core: 'http://core', fetch: flaky(calls, 99) });
  const res = await r.POST(new Request('https://shop.test/_prb/e', { method: 'POST', body: '{"site":"shop"}' }));
  assert.equal(res.status, 502);
  assert.equal(calls.length, 2, 'an event post is tried twice, no more');
  assert.match((await res.json()).error, /unreachable/);
});

test('route_never_retries_a_decide', async () => {
  const calls: any[] = [];
  const r = createRoute({ core: 'http://core', fetch: flaky(calls, 1) });
  const res = await r.POST(new Request('https://shop.test/_prb/decide', { method: 'POST', body: '{}' }));
  assert.equal(res.status, 502);
  assert.equal(calls.length, 1, 'a decide is not idempotent; it must not be sent twice');
});

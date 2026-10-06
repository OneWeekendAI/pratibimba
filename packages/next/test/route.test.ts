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

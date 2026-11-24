// Guards the pratibimba-edge sidecar image contract, locally (nothing is pushed anywhere):
// build packages/edge/Dockerfile, run it against a real Core (target/debug/pratibimba, needs
// `cargo build`) and a tiny origin on this machine, and check that
//   - the container reports healthy and runs as a non-root user,
//   - a page through the container comes back with its marked slot decided by Core,
//   - a container with no configuration refuses to start.
// Always removes the container it started.
//
//   DATABASE_URL=postgres://$(whoami)@localhost:5432/postgres node ci/edge-image.mjs
import { execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { createServer as netServer } from 'node:net';
import assert from 'node:assert/strict';
import path from 'node:path';
import { heroSite, startCore } from '../packages/edge/test/e2e/core.ts';

const ROOT = path.resolve(import.meta.dirname, '..');
const IMAGE = 'pratibimba-edge:ci';
const NAME = `prb-edge-ci-${process.pid}`;
const docker = (...a) => execFileSync('docker', a, { stdio: 'pipe' }).toString().trim();
const PAGE = '<html><body><!-- prb:slot="hero" --><h1>Espresso</h1><!-- /prb:slot --></body></html>';

// Ephemeral ports: bind 0, read the port, release it. Nothing here is fixed, so two runs (or a
// busy machine) cannot collide.
const freePort = () => new Promise((resolve, reject) => {
  const s = netServer().once('error', reject).listen(0, '0.0.0.0', () => {
    const { port } = s.address();
    s.close(() => resolve(port));
  });
});
const [CORE_PORT, ORIGIN_PORT, EDGE_PORT] = [await freePort(), await freePort(), await freePort()];

const origin = createServer((_, res) => res.writeHead(200, { 'content-type': 'text/html' }).end(PAGE)).listen(ORIGIN_PORT, '0.0.0.0');
const core = await startCore(`prb_edge_image_ci_${process.pid}`, CORE_PORT, [heroSite('shop')],
  // On Linux, host.docker.internal is the bridge gateway, not loopback: Core must listen wider.
  process.platform === 'linux' ? '0.0.0.0' : '127.0.0.1');
try {
  execFileSync('docker', ['build', '-q', '-t', IMAGE, path.join(ROOT, 'packages/edge')], { stdio: 'inherit' });

  let refused = false;
  try { docker('run', '--rm', IMAGE); } catch { refused = true; }
  assert.ok(refused, 'an unconfigured container must exit non-zero');

  // The container reaches Core and the origin on the host; Core binds 127.0.0.1 only for
  // host processes, so go through host.docker.internal.
  docker('run', '-d', '--name', NAME, '--add-host=host.docker.internal:host-gateway', '-p', `127.0.0.1:${EDGE_PORT}:8787`,
    '-e', `PRB_CORE_URL=http://host.docker.internal:${CORE_PORT}`, '-e', 'PRB_SITE=shop',
    '-e', `PRB_ORIGIN_URL=http://host.docker.internal:${ORIGIN_PORT}`, IMAGE);
  let status = '';
  for (let i = 0; i < 30 && status !== 'healthy'; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    status = docker('inspect', '-f', '{{.State.Health.Status}}', NAME);
  }
  assert.equal(status, 'healthy');
  assert.notEqual(docker('exec', NAME, 'id', '-u'), '0', 'must not run as root');

  const html = await (await fetch(`http://127.0.0.1:${EDGE_PORT}/`)).text();
  assert.match(html, /data-prb='/, `slot was not decided through the container:\n${html}`);
  console.log('edge image: ok');
} finally {
  try { docker('rm', '-f', NAME); } catch {}
  origin.close();
  core.stop();
}
process.exit(0);

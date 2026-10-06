// A fresh database, then Pratibimba Core (`serve`) and the worker; both stop with this process.
import { execFileSync, spawn } from 'node:child_process';
import path from 'node:path';

const ADMIN_DB = process.env.PRB_E2E_ADMIN_DB ?? 'postgres://postgres@localhost:55440/postgres';
const NAME = 'prb_edge_e2e';
const DB = ADMIN_DB.replace(/\/[^/]*$/, `/${NAME}`);
const BIN = path.resolve(import.meta.dirname, '../../../target/debug/pratibimba');
const psql = (sql) => execFileSync('psql', [ADMIN_DB, '-qAt', '-c', sql], { stdio: 'pipe' });
psql(`DROP DATABASE IF EXISTS ${NAME} WITH (FORCE)`);
psql(`CREATE DATABASE ${NAME}`);
const env = { ...process.env, PRB_DB: DB, RUST_LOG: 'warn', PRB_ADMIN_TOKEN: 'e2e-admin-token', PRB_BIND: '127.0.0.1:18082', PRB_WORKER_TICK_MS: '200' };
execFileSync(BIN, ['apply', path.resolve(import.meta.dirname, '../site.json')], { env, stdio: 'pipe' });
const kids = [spawn(BIN, ['serve'], { env, stdio: 'inherit' }), spawn(BIN, ['worker'], { env, stdio: 'inherit' })];
const stop = () => {
  for (const k of kids) k.kill('SIGTERM');
  process.exit(0);
};
process.on('SIGTERM', stop);
process.on('SIGINT', stop);

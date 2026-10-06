// Starts Pratibimba Core (`serve`) and the learning worker on the prepared database, and stops
// both when Playwright stops this process.
import { spawn } from 'node:child_process';
import path from 'node:path';

const ADMIN_DB = process.env.PRB_E2E_ADMIN_DB ?? 'postgres://postgres@localhost:55440/postgres';
const DB = ADMIN_DB.replace(/\/[^/]*$/, '/prb_next_e2e');
const BIN = path.resolve(import.meta.dirname, '../../../target/debug/pratibimba');
const PORT = process.env.PRB_E2E_CORE_PORT ?? '18081';
const env = { ...process.env, PRB_DB: DB, RUST_LOG: 'warn', PRB_ADMIN_TOKEN: 'e2e-admin-token', PRB_BIND: `127.0.0.1:${PORT}`, PRB_WORKER_TICK_MS: '200' };
const kids = [spawn(BIN, ['serve'], { env, stdio: 'inherit' }), spawn(BIN, ['worker'], { env, stdio: 'inherit' })];
const stop = () => {
  for (const k of kids) k.kill('SIGTERM');
  process.exit(0);
};
process.on('SIGTERM', stop);
process.on('SIGINT', stop);
for (const k of kids) k.on('exit', (code) => code && process.exit(code));

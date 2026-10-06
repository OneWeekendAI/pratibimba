// A fresh database for the e2e run: drop, create, migrate, apply the site file.
// Run by the first web server in playwright.config.ts, before Core starts.
import { execFileSync } from 'node:child_process';
import path from 'node:path';

const ADMIN_DB = process.env.PRB_E2E_ADMIN_DB ?? 'postgres://postgres@localhost:55440/postgres';
const DB = ADMIN_DB.replace(/\/[^/]*$/, '/prb_next_e2e');
const BIN = path.resolve(import.meta.dirname, '../../../target/debug/pratibimba');
const psql = (sql) => execFileSync('psql', [ADMIN_DB, '-qAt', '-c', sql], { stdio: 'pipe' });
psql('DROP DATABASE IF EXISTS prb_next_e2e WITH (FORCE)');
psql('CREATE DATABASE prb_next_e2e');
const env = { ...process.env, PRB_DB: DB, RUST_LOG: 'warn' };
execFileSync(BIN, ['migrate'], { env, stdio: 'pipe' });
execFileSync(BIN, ['apply', path.resolve(import.meta.dirname, '../site.json')], { env, stdio: 'pipe' });

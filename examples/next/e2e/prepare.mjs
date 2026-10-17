// A fresh database for the e2e run: drop, create, migrate, apply the site file.
// Run by the first web server in playwright.config.ts, before Core starts.
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const ADMIN_DB = process.env.PRB_E2E_ADMIN_DB ?? 'postgres://postgres@localhost:55440/postgres';
const DB = ADMIN_DB.replace(/\/[^/]*$/, '/prb_next_e2e');
const BIN = path.resolve(import.meta.dirname, '../../../target/debug/pratibimba');
const psql = (sql) => execFileSync('psql', [ADMIN_DB, '-qAt', '-c', sql], { stdio: 'pipe' });
psql('DROP DATABASE IF EXISTS prb_next_e2e WITH (FORCE)');
psql('CREATE DATABASE prb_next_e2e');
const env = { ...process.env, PRB_DB: DB, RUST_LOG: 'warn' };
execFileSync(BIN, ['migrate'], { env, stdio: 'pipe' });
// The install probes fetch the site's origin, so it must name the port this run serves on
// (PRB_E2E_NEXT_PORT, when 3100 is taken), not the file's default.
const port = Number(process.env.PRB_E2E_NEXT_PORT ?? 3100);
const site = JSON.parse(readFileSync(path.resolve(import.meta.dirname, '../site.json'), 'utf8'));
site.origin = `http://localhost:${port}`;
const file = path.join(mkdtempSync(path.join(tmpdir(), 'prb-next-e2e-')), 'site.json');
writeFileSync(file, JSON.stringify(site));
execFileSync(BIN, ['apply', file], { env, stdio: 'pipe' });

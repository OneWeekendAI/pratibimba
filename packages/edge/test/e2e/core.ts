// A real Pratibimba Core for the end-to-end tests: a fresh database, `migrate`, `apply` of the
// given site files with the holdout off, then `serve` and a live `worker`. Build Core first
// (`cargo build`). Postgres comes from PRB_E2E_ADMIN_DB, else DATABASE_URL.

import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const BIN = path.resolve(import.meta.dirname, '../../../../target/debug/pratibimba');

export interface Core {
  url: string;
  sql(query: string): string;
  stop(): void;
}

/** One site with a `hero` slot (baseline + gift) whose attribution window is one second. */
export function heroSite(site: string) {
  return {
    site,
    origin: 'http://localhost',
    slots: [
      {
        key: 'hero',
        selector: '#hero',
        metric: 'click',
        attribution_window_secs: 1,
        variants: [
          { key: 'baseline', baseline: true },
          { key: 'gift', html: '<h1>Gift sets</h1>' },
        ],
      },
    ],
  };
}

export async function startCore(name: string, port: number, sites: object[]): Promise<Core> {
  const admin = process.env.PRB_E2E_ADMIN_DB ?? process.env.DATABASE_URL ?? 'postgres://postgres@localhost:55440/postgres';
  const db = admin.replace(/\/[^/]*$/, `/${name}`);
  const psql = (url: string, q: string) => execFileSync('psql', [url, '-qAt', '-c', q], { stdio: 'pipe' }).toString().trim();
  psql(admin, `DROP DATABASE IF EXISTS ${name} WITH (FORCE)`);
  psql(admin, `CREATE DATABASE ${name}`);
  const env = { ...process.env, PRB_DB: db, RUST_LOG: 'warn', PRB_BIND: `127.0.0.1:${port}`, PRB_WORKER_TICK_MS: '100' };
  execFileSync(BIN, ['migrate'], { env, stdio: 'pipe' });
  const dir = mkdtempSync(path.join(tmpdir(), 'prb-e2e-'));
  for (const s of sites) {
    const file = path.join(dir, `${(s as { site: string }).site}.json`);
    writeFileSync(file, JSON.stringify(s));
    execFileSync(BIN, ['apply', file], { env, stdio: 'pipe' });
  }
  psql(db, 'UPDATE sites SET holdout_fraction = 0');
  const kids: ChildProcess[] = [spawn(BIN, ['serve'], { env, stdio: 'inherit' }), spawn(BIN, ['worker'], { env, stdio: 'inherit' })];
  const url = `http://127.0.0.1:${port}`;
  for (let i = 0; ; i++) {
    try {
      if ((await fetch(`${url}/health`)).ok) break;
    } catch {}
    if (i > 100) throw new Error('Core did not start');
    await new Promise((r) => setTimeout(r, 100));
  }
  return {
    url,
    sql: (q) => psql(db, q),
    stop: () => {
      for (const k of kids) k.kill('SIGTERM');
    },
  };
}

/** Wait until the worker has closed every decision of `site` (its windows are one second). */
export async function settled(core: Core, site: string) {
  for (let i = 0; i < 200; i++) {
    if (core.sql(`SELECT count(*) FROM decisions WHERE site_id = '${site}' AND settled_at IS NULL`) === '0') return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`decisions of ${site} never settled`);
}

/** A seeded RNG (mulberry32), so the visitor stream is the same every run. */
export function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** The visitor's local hour right now in `zone`. */
export function hourIn(zone: string): number {
  return Number(new Intl.DateTimeFormat('en-US', { timeZone: zone, hour: 'numeric', hourCycle: 'h23' }).format(new Date()));
}

/**
 * Fixed-offset IANA zones (`Etc/GMT-5` is UTC+5) whose local hour is, right now, deep inside the
 * night (23:00-02:59, inside the default 22-05 window even if the hour turns during the run)
 * or deep inside the day (10:00-15:59).
 */
export function zonesNow(): { night: string[]; day: string[] } {
  const all: string[] = [];
  for (let h = -14; h <= 12; h++) all.push(h === 0 ? 'Etc/GMT' : `Etc/GMT${h > 0 ? '+' : ''}${h}`);
  return {
    night: all.filter((z) => [23, 0, 1, 2].includes(hourIn(z))),
    day: all.filter((z) => hourIn(z) >= 10 && hourIn(z) <= 15),
  };
}

/** Planted truth: night visitors click the gift hero, day visitors the baseline. */
export function truth(variant: string, night: boolean): number {
  if (variant === 'gift') return night ? 0.3 : 0.04;
  return night ? 0.04 : 0.25;
}

export const VISITORS = 1500;
export const MEASURE = 400;
export const SETTLE_EVERY = 100;

import path from 'node:path';

export const CORE_PORT = Number(process.env.PRB_E2E_CORE_PORT ?? 18081);
export const NEXT_PORT = Number(process.env.PRB_E2E_NEXT_PORT ?? 3100);
export const CORE = `http://127.0.0.1:${CORE_PORT}`;
export const TOKEN = 'e2e-admin-token';
/** A Postgres server where the e2e may create and drop its own database. */
export const ADMIN_DB = process.env.PRB_E2E_ADMIN_DB ?? 'postgres://postgres@localhost:55440/postgres';
export const DB = ADMIN_DB.replace(/\/[^/]*$/, '/prb_next_e2e');
export const BIN = path.resolve(import.meta.dirname, '../../../target/debug/pratibimba');

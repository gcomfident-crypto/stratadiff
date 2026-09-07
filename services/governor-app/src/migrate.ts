import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import pg from "pg";

import { runMigrations } from "./database.js";

const databaseUrl = process.env["DATABASE_URL"];
if (databaseUrl === undefined || databaseUrl.length === 0) {
  throw new Error("DATABASE_URL is required");
}

const pool = new pg.Pool({ connectionString: databaseUrl });
const migrationsDirectory = join(dirname(fileURLToPath(import.meta.url)), "..", "migrations");

await runMigrations(pool, migrationsDirectory);
await pool.end();

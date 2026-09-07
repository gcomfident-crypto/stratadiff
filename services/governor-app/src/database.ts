import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";

import type { Pool } from "pg";

import type { DatabasePool } from "./store.js";

export async function runMigrations(pool: DatabasePool | Pool, directory: string): Promise<void> {
  const database = pool as DatabasePool;
  const client = await database.connect();
  try {
    await client.query(
      `CREATE TABLE IF NOT EXISTS governor_schema_migration (
         name text PRIMARY KEY,
         applied_at timestamptz NOT NULL
       )`,
    );
    const files = (await readdir(directory))
      .filter((name) => /^[0-9]+_[a-z0-9_]+\.sql$/.test(name))
      .sort();
    for (const name of files) {
      const sql = await readFile(join(directory, name), "utf8");
      await client.query("BEGIN");
      try {
        const claimed = await client.query(
          `INSERT INTO governor_schema_migration (name, applied_at)
           VALUES ($1, $2)
           ON CONFLICT (name) DO NOTHING
           RETURNING name`,
          [name, new Date()],
        );
        if (claimed.rowCount === 0) {
          await client.query("COMMIT");
          continue;
        }
        await client.query(sql);
        await client.query("COMMIT");
      } catch (error) {
        await client.query("ROLLBACK");
        throw error;
      }
    }
  } finally {
    client.release();
  }
}

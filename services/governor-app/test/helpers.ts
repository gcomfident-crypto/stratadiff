import { createHmac } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { DataType, newDb } from "pg-mem";

import { runMigrations } from "../src/database.js";
import { PgGovernorStore, type DatabasePool } from "../src/store.js";
import { WebhookService } from "../src/webhook.js";

export const SECRET = "test-webhook-secret";
export const BASE_A = "a".repeat(40);
export const BASE_B = "b".repeat(40);
export const HEAD_A = "c".repeat(40);
export const HEAD_B = "d".repeat(40);

export async function createStore(databaseNow: () => Date = () => new Date()): Promise<{
  store: PgGovernorStore;
  pool: DatabasePool;
}> {
  const database = newDb();
  database.public.registerFunction({
    name: "char_length",
    args: [DataType.text],
    returns: DataType.integer,
    implementation: (value: string) => value.length,
  });
  database.public.registerFunction({
    name: "pg_advisory_xact_lock",
    args: [DataType.bigint],
    returns: DataType.bool,
    implementation: () => true,
  });
  database.public.registerFunction({
    name: "statement_timestamp",
    returns: DataType.timestamptz,
    impure: true,
    implementation: databaseNow,
  });
  const adapter = database.adapters.createPg();
  const pool = new adapter.Pool() as unknown as DatabasePool;
  const migrationDirectory = join(dirname(fileURLToPath(import.meta.url)), "..", "migrations");
  await runMigrations(pool, migrationDirectory);
  return { store: new PgGovernorStore(pool), pool };
}

export function pullPayload(options: {
  number: number;
  baseSha: string;
  headSha: string;
  updatedAt: string;
  state?: "open" | "closed";
  draft?: boolean;
}): Record<string, unknown> {
  return {
    action: options.state === "closed" ? "closed" : "synchronize",
    installation: { id: 71 },
    repository: {
      id: 99,
      name: "repo",
      full_name: "acme/repo",
      owner: { login: "acme" },
    },
    number: options.number,
    pull_request: {
      number: options.number,
      state: options.state ?? "open",
      draft: options.draft ?? false,
      updated_at: options.updatedAt,
      base: { sha: options.baseSha },
      head: { sha: options.headSha },
    },
  };
}

export function issueCommentPayload(options: {
  number: number;
  action: "created" | "edited" | "deleted";
  id?: number;
  createdAt?: string;
  updatedAt?: string;
}): Record<string, unknown> {
  const createdAt = options.createdAt ?? "2026-09-07T12:01:00.000Z";
  const updatedAt = options.updatedAt ?? createdAt;
  return {
    action: options.action,
    installation: { id: 71 },
    repository: {
      id: 99,
      name: "repo",
      full_name: "acme/repo",
      owner: { login: "acme" },
    },
    issue: { number: options.number, pull_request: { url: "https://api.github.test/pulls/1" } },
    comment: {
      id: options.id ?? 501,
      body:
        "<!-- CodeRabbit review command invocation: 01234567-89ab-cdef-0123-456789abcdef -->\nFull review finished.",
      created_at: createdAt,
      updated_at: updatedAt,
      user: {
        id: 136622811,
        login: "coderabbitai[bot]",
        type: "Bot",
        html_url: "https://github.com/apps/coderabbitai",
      },
    },
  };
}

export function mergeGroupPayload(
  action: "checks_requested" | "destroyed",
  headSha: string,
): Record<string, unknown> {
  return {
    action,
    installation: { id: 71 },
    repository: {
      id: 99,
      name: "repo",
      full_name: "acme/repo",
      owner: { login: "acme" },
    },
    merge_group: {
      head_sha: headSha,
      base_sha: BASE_A,
      head_ref: "refs/heads/gh-readonly-queue/main/pr-1-deadbeef",
    },
  };
}

export async function deliver(
  service: WebhookService,
  deliveryId: string,
  eventName: string,
  payload: Record<string, unknown>,
) {
  const body = Buffer.from(JSON.stringify(payload));
  const signature = `sha256=${createHmac("sha256", SECRET).update(body).digest("hex")}`;
  return service.receive({ signature, deliveryId, eventName }, body);
}

export function testService(store: PgGovernorStore, now: Date): WebhookService {
  return new WebhookService(SECRET, store, () => now);
}

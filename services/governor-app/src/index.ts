import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import pg from "pg";

import { loadConfig } from "./config.js";
import { runMigrations } from "./database.js";
import { GithubAppClient } from "./github.js";
import { createServer } from "./server.js";
import { PgGovernorStore } from "./store.js";
import { WebhookService } from "./webhook.js";
import { GovernorWorker } from "./worker.js";

const config = loadConfig(process.env);
const pool = new pg.Pool({ connectionString: config.databaseUrl });
const migrationsDirectory = join(dirname(fileURLToPath(import.meta.url)), "..", "migrations");

await runMigrations(pool, migrationsDirectory);

const store = new PgGovernorStore(pool);
const github = new GithubAppClient({
  appId: config.githubAppId,
  privateKey: config.githubPrivateKey,
  apiUrl: config.githubApiUrl,
  requestTimeoutMs: config.githubRequestTimeoutMs,
  fetch: globalThis.fetch,
  now: () => new Date(),
});
const webhookService = new WebhookService(config.webhookSecret, store);
const server = createServer(webhookService);
const abort = new AbortController();
const worker = new GovernorWorker(store, github, {
  workerId: config.workerId,
  leaseSeconds: config.leaseSeconds,
  pollMs: config.workerPollMs,
  retrySeconds: config.outboxRetrySeconds,
  evidenceTimeoutSeconds: config.evidenceTimeoutSeconds,
  now: () => new Date(),
});

await new Promise<void>((resolve, reject) => {
  server.once("error", reject);
  server.listen(config.port, resolve);
});

const shutdownRequested = new Promise<void>((resolve) => {
  const requestShutdown = () => resolve();
  process.once("SIGINT", requestShutdown);
  process.once("SIGTERM", requestShutdown);
});

const outcome = await Promise.race([
  shutdownRequested.then(() => null),
  worker.run(abort.signal).then(
    () => null,
    (error: unknown) => error,
  ),
]);
abort.abort();
await new Promise<void>((resolve, reject) => {
  server.close((error) => {
    if (error === undefined) {
      resolve();
    } else {
      reject(error);
    }
  });
});
await pool.end();
if (outcome !== null) {
  throw outcome;
}

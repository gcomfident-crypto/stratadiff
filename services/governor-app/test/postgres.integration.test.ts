import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import pg, { type QueryResult, type QueryResultRow } from "pg";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";

import { runMigrations } from "../src/database.js";
import { PgGovernorStore, type DatabasePool } from "../src/store.js";
import type { GateSubject } from "../src/types.js";

const databaseUrl = process.env["DATABASE_URL"];
if (databaseUrl === undefined || databaseUrl.length === 0) {
  throw new Error("DATABASE_URL is required for the real PostgreSQL integration suite");
}

const START = new Date("2030-01-01T00:00:00.000Z");
const pool = new pg.Pool({
  connectionString: databaseUrl,
  max: 8,
  statement_timeout: 2_000,
});
const store = new PgGovernorStore(pool);
const migrationsDirectory = join(dirname(fileURLToPath(import.meta.url)), "..", "migrations");

interface Deferred {
  promise: Promise<void>;
  resolve(): void;
}

function deferred(): Deferred {
  let resolvePromise!: () => void;
  const promise = new Promise<void>((resolve) => {
    resolvePromise = resolve;
  });
  return { promise, resolve: resolvePromise };
}

class ClaimBarrierPool implements DatabasePool {
  readonly candidateLocked = deferred();
  readonly releaseCandidate = deferred();
  readonly #pool: pg.Pool;
  #holdingFirstCandidate = false;

  constructor(realPool: pg.Pool) {
    this.#pool = realPool;
  }

  async connect() {
    const client = await this.#pool.connect();
    return {
      query: async <R extends QueryResultRow = QueryResultRow>(
        text: string,
        values?: unknown[],
      ): Promise<QueryResult<R>> => {
        const result = await client.query<R>(text, values);
        if (!this.#holdingFirstCandidate && text.includes("FOR UPDATE SKIP LOCKED")) {
          this.#holdingFirstCandidate = true;
          this.candidateLocked.resolve();
          await this.releaseCandidate.promise;
        }
        return result;
      },
      release: () => client.release(),
    };
  }

  async end(): Promise<void> {
    await this.#pool.end();
  }
}

class GlobalIngestSignalPool implements DatabasePool {
  readonly globalIngestLocked = deferred();
  readonly #pool: pg.Pool;
  #signalled = false;

  constructor(realPool: pg.Pool) {
    this.#pool = realPool;
  }

  async connect() {
    const client = await this.#pool.connect();
    return {
      query: async <R extends QueryResultRow = QueryResultRow>(
        text: string,
        values?: unknown[],
      ): Promise<QueryResult<R>> => {
        const result = await client.query<R>(text, values);
        if (!this.#signalled && text.includes("SELECT pg_advisory_xact_lock")) {
          this.#signalled = true;
          this.globalIngestLocked.resolve();
        }
        return result;
      },
      release: () => client.release(),
    };
  }

  async end(): Promise<void> {
    await this.#pool.end();
  }
}

async function ingestPull(
  target: PgGovernorStore,
  options: {
    deliveryId: string;
    pullNumber: number;
    sourceUpdatedAt: Date;
    receivedAt: Date;
    draft?: boolean;
  },
): Promise<GateSubject> {
  const result = await target.ingest({
    deliveryId: options.deliveryId,
    eventName: "pull_request",
    payloadSha256: createHash("sha256").update(options.deliveryId).digest("hex"),
    receivedAt: options.receivedAt,
    impact: {
      kind: "pull_request",
      eventName: "pull_request",
      action: "synchronize",
      repository: {
        installationId: 71,
        repositoryId: 99,
        fullName: "acme/repo",
        owner: "acme",
        name: "repo",
      },
      pair: {
        number: options.pullNumber,
        baseSha: "a".repeat(40),
        headSha: "b".repeat(40),
        state: "open",
        draft: options.draft ?? false,
        sourceUpdatedAt: options.sourceUpdatedAt,
      },
    },
  });
  if (result.disposition !== "applied") {
    throw new Error(`test pull request was not applied: ${result.disposition}`);
  }
  const gate = await target.findActiveGate(99, "pull_request", `pr:${options.pullNumber}`);
  if (gate === null) {
    throw new Error("test pull request did not create an active gate");
  }
  return gate;
}

beforeAll(async () => {
  const version = await pool.query<{ version: string }>("SELECT version()");
  expect(version.rows[0]?.version).toContain("PostgreSQL");
  await runMigrations(pool, migrationsDirectory);
});

beforeEach(async () => {
  await pool.query(
    "TRUNCATE TABLE webhook_delivery, pr_pair, dispatch, evidence, gate_subject, outbox, repository_reconcile_generation RESTART IDENTITY CASCADE",
  );
});

afterAll(async () => {
  await pool.end();
});

describe("real PostgreSQL worker fencing", () => {
  it("uses SKIP LOCKED so a second worker cannot claim the row held by the first", async () => {
    const barrierPool = new ClaimBarrierPool(pool);
    const concurrentStore = new PgGovernorStore(barrierPool);
    await ingestPull(concurrentStore, {
      deliveryId: "skip-locked-pair",
      pullNumber: 1,
      sourceUpdatedAt: new Date(START.valueOf() - 60_000),
      receivedAt: START,
      draft: true,
    });

    const firstClaim = concurrentStore.claimOutbox("worker-a", START, 30);
    await barrierPool.candidateLocked.promise;

    let secondClaim;
    try {
      secondClaim = await concurrentStore.claimOutbox("worker-b", START, 30);
    } finally {
      barrierPool.releaseCandidate.resolve();
    }

    const firstLease = await firstClaim;
    expect(firstLease).not.toBeNull();
    expect(secondClaim).toBeNull();
    expect(await concurrentStore.completeOutbox(firstLease!, new Date(START.valueOf() + 1))).toBe(
      true,
    );
  });

  it("rejects completion and failure from an expired or re-fenced outbox lease", async () => {
    await ingestPull(store, {
      deliveryId: "outbox-fence-pair",
      pullNumber: 2,
      sourceUpdatedAt: new Date(START.valueOf() - 60_000),
      receivedAt: START,
      draft: true,
    });

    const oldLease = await store.claimOutbox("old-worker", START, 5);
    expect(oldLease).not.toBeNull();
    const afterExpiry = new Date(START.valueOf() + 6_000);
    expect(await store.completeOutbox(oldLease!, afterExpiry)).toBe(false);
    expect(await store.failOutbox(oldLease!, "late failure", afterExpiry, 10)).toBe(false);

    const newLease = await store.claimOutbox("new-worker", afterExpiry, 5);
    expect(newLease).toMatchObject({ id: oldLease!.id, workerId: "new-worker" });
    expect(newLease!.fence).toBeGreaterThan(oldLease!.fence);
    expect(await store.completeOutbox(oldLease!, new Date(afterExpiry.valueOf() + 1))).toBe(false);
    expect(
      await store.failOutbox(
        oldLease!,
        "stale fenced failure",
        new Date(afterExpiry.valueOf() + 1),
        10,
      ),
    ).toBe(false);
    expect(await store.completeOutbox(newLease!, new Date(afterExpiry.valueOf() + 1))).toBe(true);
  });

  it("rejects a pair success commit after its lease expires", async () => {
    const gate = await ingestPull(store, {
      deliveryId: "expired-pair-lease",
      pullNumber: 3,
      sourceUpdatedAt: new Date(START.valueOf() - 60_000),
      receivedAt: START,
    });
    const lease = await store.acquirePairLease(gate.pairId!, gate.epoch, "old-worker", START, 5);
    expect(lease).not.toBeNull();

    expect(
      await store.commitPairGate(
        lease!,
        "success",
        "stale success",
        new Date(START.valueOf() + 6_000),
      ),
    ).toBe(false);
    expect(await store.getGateSubject(gate.id)).toMatchObject({ desiredState: "revoked" });
  });

  it("rejects a pair success commit after a newer event advances its fence", async () => {
    const gate = await ingestPull(store, {
      deliveryId: "initial-pair-event",
      pullNumber: 4,
      sourceUpdatedAt: new Date(START.valueOf() - 60_000),
      receivedAt: START,
    });
    const staleLease = await store.acquirePairLease(
      gate.pairId!,
      gate.epoch,
      "old-worker",
      START,
      30,
    );
    expect(staleLease).not.toBeNull();

    const eventAt = new Date(START.valueOf() + 1_000);
    const updatedGate = await ingestPull(store, {
      deliveryId: "newer-pair-event",
      pullNumber: 4,
      sourceUpdatedAt: eventAt,
      receivedAt: eventAt,
    });
    expect(updatedGate.id).toBe(gate.id);
    expect(
      await store.commitPairGate(
        staleLease!,
        "success",
        "stale success",
        new Date(START.valueOf() + 2_000),
      ),
    ).toBe(false);
    expect(await store.getGateSubject(gate.id)).toMatchObject({
      desiredState: "revoked",
      desiredSummary: "Pull request state changed; re-evaluating final-head evidence.",
    });
  });

  it("rejects a repository snapshot invalidated by a concurrent pull-request event", async () => {
    await ingestPull(store, {
      deliveryId: "snapshot-existing-pull",
      pullNumber: 5,
      sourceUpdatedAt: new Date(START.valueOf() - 60_000),
      receivedAt: START,
    });
    const repository = {
      installationId: 71,
      repositoryId: 99,
      fullName: "acme/repo",
      owner: "acme",
      name: "repo",
    };
    const staleToken = await store.beginRepositorySnapshot(repository, START);

    const generationLock = await pool.connect();
    await generationLock.query("BEGIN");
    let generationLockReleased = false;
    try {
      await generationLock.query(
        `SELECT generation FROM repository_reconcile_generation
          WHERE repository_id = $1
          FOR UPDATE`,
        [repository.repositoryId],
      );
      const signalledPool = new GlobalIngestSignalPool(pool);
      const concurrentStore = new PgGovernorStore(signalledPool);
      const newPull = ingestPull(concurrentStore, {
        deliveryId: "snapshot-concurrent-pull",
        pullNumber: 6,
        sourceUpdatedAt: new Date(START.valueOf() + 1_000),
        receivedAt: new Date(START.valueOf() + 1_000),
      });
      await signalledPool.globalIngestLocked.promise;

      const staleReconcile = store.reconcileOpenPullRequests(
        staleToken,
        [],
        "snapshot-existing-pull",
        new Date(START.valueOf() + 2_000),
      );
      await generationLock.query("COMMIT");
      generationLockReleased = true;

      await newPull;
      expect(await staleReconcile).toBe("stale");
    } finally {
      if (!generationLockReleased) {
        await generationLock.query("ROLLBACK");
      }
      generationLock.release();
    }

    expect(await store.findActiveGate(99, "pull_request", "pr:5")).not.toBeNull();
    expect(await store.findActiveGate(99, "pull_request", "pr:6")).not.toBeNull();
    const activePairs = await pool.query<{ pull_number: number }>(
      `SELECT pull_number FROM pr_pair
        WHERE repository_id = $1 AND active
        ORDER BY pull_number`,
      [repository.repositoryId],
    );
    expect(activePairs.rows.map((row) => row.pull_number)).toEqual([5, 6]);
    const generation = await pool.query<{ generation: string }>(
      `SELECT generation FROM repository_reconcile_generation
        WHERE repository_id = $1`,
      [repository.repositoryId],
    );
    const generationRow = generation.rows[0];
    if (generationRow === undefined) {
      throw new Error("repository generation disappeared during the concurrency test");
    }
    expect(Number(generationRow.generation)).toBe(staleToken.generation + 1);
  });
});

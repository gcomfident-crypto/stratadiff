import { afterEach, describe, expect, it } from "vitest";

import { evaluatePair } from "../src/evaluate.js";
import type { DatabasePool, PgGovernorStore } from "../src/store.js";
import {
  BASE_A,
  BASE_B,
  HEAD_A,
  HEAD_B,
  createStore,
  deliver,
  deliverRaw,
  issueCommentPayload,
  mergeGroupPayload,
  pullPayload,
  testService,
} from "./helpers.js";

describe("transactional webhook projection", () => {
  const pools: DatabasePool[] = [];

  afterEach(async () => {
    await Promise.all(pools.splice(0).map((pool) => pool.end()));
  });

  async function harness(
    now = new Date("2026-09-07T13:00:00.000Z"),
    databaseNow: () => Date = () => new Date(),
  ) {
    const result = await createStore(databaseNow);
    pools.push(result.pool);
    return { ...result, service: testService(result.store, now) };
  }

  async function commitCoveredGate(
    store: PgGovernorStore,
    pairId: string,
    epoch: number,
    workerId: string,
  ): Promise<void> {
    const now = new Date();
    const dispatchLease = await store.acquirePairLease(pairId, epoch, workerId, now, 30);
    expect(dispatchLease).not.toBeNull();
    expect(await store.planDispatch(dispatchLease!, now)).toBe(true);
    expect(await store.beginDispatchAttempt(dispatchLease!, now)).toBe(true);
    expect(
      await store.adoptDispatch(dispatchLease!, 10_000 + epoch, now, 1_800, now),
    ).toBe(true);
    const gateLease = await store.acquirePairLease(pairId, epoch, workerId, now, 30);
    expect(gateLease).not.toBeNull();
    expect(await store.commitPairGate(gateLease!, "success", "covered", now)).toBe(true);
  }

  it("deduplicates deliveries and refuses an out-of-order pair regression", async () => {
    const { store, service } = await harness();
    const first = pullPayload({
      number: 1,
      baseSha: BASE_A,
      headSha: HEAD_A,
      updatedAt: "2026-09-07T12:00:00.000Z",
    });
    expect(await deliver(service, "delivery-1", "pull_request", first)).toMatchObject({
      duplicate: false,
      disposition: "applied",
    });
    expect(await deliver(service, "delivery-1", "pull_request", first)).toMatchObject({
      duplicate: true,
    });

    await deliver(
      service,
      "delivery-2",
      "pull_request",
      pullPayload({
        number: 1,
        baseSha: BASE_B,
        headSha: HEAD_B,
        updatedAt: "2026-09-07T12:02:00.000Z",
      }),
    );
    const stale = await deliver(
      service,
      "delivery-3",
      "pull_request",
      pullPayload({
        number: 1,
        baseSha: BASE_A,
        headSha: HEAD_A,
        updatedAt: "2026-09-07T12:01:00.000Z",
      }),
    );
    expect(stale.disposition).toBe("stale");
    const gate = await store.findActiveGate(99, "pull_request", "pr:1");
    expect(gate).toMatchObject({ headSha: HEAD_B, baseSha: BASE_B, epoch: 2 });

    await deliver(
      service,
      "delivery-close",
      "pull_request",
      pullPayload({
        number: 1,
        baseSha: BASE_B,
        headSha: HEAD_B,
        updatedAt: "2026-09-07T12:03:00.000Z",
        state: "closed",
      }),
    );
    expect(await store.getGateSubject(gate!.id)).toMatchObject({
      active: false,
      desiredState: "failure",
    });
    expect(
      await deliver(
        service,
        "delivery-delayed-reopen",
        "pull_request",
        pullPayload({
          number: 1,
          baseSha: BASE_B,
          headSha: HEAD_B,
          updatedAt: "2026-09-07T12:02:30.000Z",
        }),
      ),
    ).toMatchObject({ disposition: "stale" });
    expect(await store.findActiveGate(99, "pull_request", "pr:1")).toBeNull();
  });

  it("keeps same-SHA pull requests isolated and revokes only the affected gate", async () => {
    const { store, service } = await harness();
    await deliver(
      service,
      "pr-one",
      "pull_request",
      pullPayload({
        number: 1,
        baseSha: BASE_A,
        headSha: HEAD_A,
        updatedAt: "2026-09-07T12:00:00.000Z",
      }),
    );
    await deliver(
      service,
      "pr-two",
      "pull_request",
      pullPayload({
        number: 2,
        baseSha: BASE_B,
        headSha: HEAD_A,
        updatedAt: "2026-09-07T12:00:00.000Z",
      }),
    );
    const firstGate = await store.findActiveGate(99, "pull_request", "pr:1");
    const secondGate = await store.findActiveGate(99, "pull_request", "pr:2");
    expect(firstGate?.pairId).not.toBeNull();
    expect(secondGate?.pairId).not.toBeNull();
    await commitCoveredGate(store, firstGate!.pairId!, 1, "worker-a");
    await commitCoveredGate(store, secondGate!.pairId!, 1, "worker-b");

    await deliver(
      service,
      "comment-pr-one",
      "issue_comment",
      issueCommentPayload({ number: 1, action: "edited" }),
    );
    expect(await store.findActiveGate(99, "pull_request", "pr:1")).toMatchObject({
      desiredState: "revoked",
    });
    expect(await store.findActiveGate(99, "pull_request", "pr:2")).toMatchObject({
      desiredState: "success",
    });
  });

  it("uses lease fencing so an expired worker cannot commit", async () => {
    const { store, service } = await harness();
    await deliver(
      service,
      "pair-for-lease",
      "pull_request",
      pullPayload({
        number: 3,
        baseSha: BASE_A,
        headSha: HEAD_A,
        updatedAt: "2026-09-07T12:00:00.000Z",
      }),
    );
    const gate = await store.findActiveGate(99, "pull_request", "pr:3");
    const pairId = gate!.pairId!;
    const start = new Date("2026-09-07T13:00:00.000Z");
    const oldLease = await store.acquirePairLease(pairId, 1, "old-worker", start, 10);
    expect(oldLease).not.toBeNull();
    expect(
      await store.acquirePairLease(
        pairId,
        1,
        "new-worker",
        new Date("2026-09-07T13:00:05.000Z"),
        10,
      ),
    ).toBeNull();
    const newLease = await store.acquirePairLease(
      pairId,
      1,
      "new-worker",
      new Date("2026-09-07T13:00:11.000Z"),
      10,
    );
    expect(newLease!.fence).toBeGreaterThan(oldLease!.fence);
    expect(
      await store.commitPairGate(
        oldLease!,
        "success",
        "stale success",
        new Date("2026-09-07T13:00:12.000Z"),
      ),
    ).toBe(false);
    expect(
      await store.commitPairGate(
        newLease!,
        "failure",
        "new worker decision",
        new Date("2026-09-07T13:00:12.000Z"),
      ),
    ).toBe(true);
    expect(await store.findActiveGate(99, "pull_request", "pr:3")).toMatchObject({
      desiredState: "failure",
      desiredSummary: "new worker decision",
    });
  });

  it("keeps a deletion tombstone when an older comment delivery arrives later", async () => {
    const { store, service } = await harness();
    await deliver(
      service,
      "pair-comment-order",
      "pull_request",
      pullPayload({
        number: 4,
        baseSha: BASE_A,
        headSha: HEAD_A,
        updatedAt: "2026-09-07T12:00:00.000Z",
      }),
    );
    await deliver(
      service,
      "comment-deleted",
      "issue_comment",
      issueCommentPayload({
        number: 4,
        action: "deleted",
        id: 900,
        createdAt: "2026-09-07T12:01:00.000Z",
        updatedAt: "2026-09-07T12:05:00.000Z",
      }),
    );
    await deliver(
      service,
      "comment-created-late",
      "issue_comment",
      issueCommentPayload({
        number: 4,
        action: "created",
        id: 900,
        createdAt: "2026-09-07T12:01:00.000Z",
        updatedAt: "2026-09-07T12:01:00.000Z",
      }),
    );
    const gate = await store.findActiveGate(99, "pull_request", "pr:4");
    const snapshot = await store.loadPairSnapshot(gate!.pairId!, gate!.epoch);
    expect(snapshot!.evidence).toHaveLength(1);
    expect(snapshot!.evidence[0]).toMatchObject({ sourceId: "900", valid: false });
  });

  it("creates and cancels a distinct gate on the merge-group head SHA", async () => {
    const { store, service } = await harness();
    await deliver(
      service,
      "merge-created",
      "merge_group",
      mergeGroupPayload("checks_requested", HEAD_B),
    );
    const created = await store.findActiveGate(
      99,
      "merge_group",
      "merge-group:refs/heads/gh-readonly-queue/main/pr-1-deadbeef",
    );
    expect(created).toMatchObject({
      subjectType: "merge_group",
      headSha: HEAD_B,
      pairId: null,
      desiredState: "revoked",
    });
    await deliver(
      service,
      "merge-destroyed",
      "merge_group",
      mergeGroupPayload("destroyed", HEAD_B),
    );
    expect(
      await store.findActiveGate(
        99,
        "merge_group",
        "merge-group:refs/heads/gh-readonly-queue/main/pr-1-deadbeef",
      ),
    ).toBeNull();
    expect(await store.getGateSubject(created!.id)).toMatchObject({
      active: false,
      desiredState: "cancelled",
      headSha: HEAD_B,
    });
  });

  it("quarantines malformed signed events so reconciliation cannot reuse old evidence", async () => {
    const { store, service } = await harness();
    const payload = pullPayload({
      number: 6,
      baseSha: BASE_A,
      headSha: HEAD_A,
      updatedAt: "2026-09-07T12:00:00.000Z",
    });
    await deliver(service, "malformed-pair", "pull_request", payload);
    const gate = await store.findActiveGate(99, "pull_request", "pr:6");
    await commitCoveredGate(store, gate!.pairId!, gate!.epoch, "worker");

    const result = await deliver(
      service,
      "malformed-review",
      "pull_request_review",
      { ...payload, action: "submitted" },
    );
    expect(result).toMatchObject({ disposition: "applied" });
    expect(await store.findActiveGate(99, "pull_request", "pr:6")).toMatchObject({
      desiredState: "failure",
    });
    expect(await store.loadPairSnapshot(gate!.pairId!, gate!.epoch)).toMatchObject({
      dispatch: { state: "abandoned", evidenceDeadlineAt: null },
    });

    await store.reconcileOpenPullRequests(
      {
        installationId: 71,
        repositoryId: 99,
        fullName: "acme/repo",
        owner: "acme",
        name: "repo",
      },
      [
        {
          number: 6,
          baseSha: BASE_A,
          headSha: HEAD_A,
          state: "open",
          merged: false,
          draft: false,
          sourceUpdatedAt: new Date("2026-09-07T12:00:00.000Z"),
        },
      ],
      "malformed-review",
      new Date("2026-09-07T13:00:01.000Z"),
    );
    const reconciled = await store.loadPairSnapshot(gate!.pairId!, gate!.epoch);
    const decision = evaluatePair(reconciled!, new Date("2026-09-07T13:00:02.000Z"));
    expect(decision).toMatchObject({ state: "failure", needsDispatch: false });
    const evaluationLease = await store.acquirePairLease(
      gate!.pairId!,
      gate!.epoch,
      "evaluation-worker",
      new Date("2026-09-07T13:00:02.000Z"),
      30,
    );
    expect(evaluationLease).toBeNull();
    expect(await store.findActiveGate(99, "pull_request", "pr:6")).toMatchObject({
      desiredState: "failure",
      quarantined: true,
    });
  });

  it("persists an unscoped signed delivery and quarantines every active gate", async () => {
    const { store, service, pool } = await harness();
    await deliver(
      service,
      "global-pr-one",
      "pull_request",
      pullPayload({
        number: 8,
        baseSha: BASE_A,
        headSha: HEAD_A,
        updatedAt: "2026-09-07T12:00:00.000Z",
      }),
    );
    const secondPayload = pullPayload({
      number: 9,
      baseSha: BASE_B,
      headSha: HEAD_B,
      updatedAt: "2026-09-07T12:00:00.000Z",
    });
    secondPayload["repository"] = {
      id: 100,
      name: "other",
      full_name: "elsewhere/other",
      owner: { login: "elsewhere" },
    };
    await deliver(service, "global-pr-two", "pull_request", secondPayload);
    const mergePayload = mergeGroupPayload("checks_requested", HEAD_B);
    mergePayload["repository"] = {
      id: 100,
      name: "other",
      full_name: "elsewhere/other",
      owner: { login: "elsewhere" },
    };
    await deliver(service, "global-merge", "merge_group", mergePayload);

    const firstGate = await store.findActiveGate(99, "pull_request", "pr:8");
    const secondGate = await store.findActiveGate(100, "pull_request", "pr:9");
    const mergeGate = await store.findActiveGate(
      100,
      "merge_group",
      "merge-group:refs/heads/gh-readonly-queue/main/pr-1-deadbeef",
    );
    await commitCoveredGate(store, firstGate!.pairId!, firstGate!.epoch, "covered-worker");
    const coveredGate = await store.getGateSubject(firstGate!.id);
    const stalePublicationLease = await store.acquireGateLease(
      coveredGate!.id,
      coveredGate!.epoch,
      coveredGate!.revision,
      "pre-quarantine-publisher",
      new Date("2026-09-07T13:00:00.000Z"),
      30,
    );
    expect(stalePublicationLease).not.toBeNull();
    const staleLease = await store.acquirePairLease(
      secondGate!.pairId!,
      secondGate!.epoch,
      "pre-quarantine-worker",
      new Date("2026-09-07T13:00:00.000Z"),
      30,
    );
    expect(staleLease).not.toBeNull();

    const invalidBody = Buffer.from("{not-json");
    const result = await deliverRaw(service, "global-invalid-json", "pull_request", invalidBody);
    expect(result).toMatchObject({ duplicate: false, disposition: "applied" });
    expect(new Set(result.touchedSubjects)).toEqual(
      new Set([firstGate!.id, secondGate!.id, mergeGate!.id]),
    );

    expect(await store.findActiveGate(99, "pull_request", "pr:8")).toMatchObject({
      desiredState: "failure",
      quarantined: true,
      quarantineDeliveryId: "global-invalid-json",
    });
    expect(await store.findActiveGate(100, "pull_request", "pr:9")).toMatchObject({
      desiredState: "failure",
      quarantined: true,
    });
    expect(
      await store.findActiveGate(
        100,
        "merge_group",
        "merge-group:refs/heads/gh-readonly-queue/main/pr-1-deadbeef",
      ),
    ).toMatchObject({ desiredState: "failure", quarantined: true });
    expect(await store.loadPairSnapshot(firstGate!.pairId!, firstGate!.epoch)).toMatchObject({
      quarantined: true,
      quarantineDeliveryId: "global-invalid-json",
      dispatch: { state: "abandoned", evidenceDeadlineAt: null },
    });
    expect(await store.loadPairSnapshot(secondGate!.pairId!, secondGate!.epoch)).toMatchObject({
      quarantined: true,
      dispatch: null,
    });
    expect(
      await store.commitPairGate(
        staleLease!,
        "success",
        "stale success",
        new Date("2026-09-07T13:00:01.000Z"),
      ),
    ).toBe(false);
    expect(
      await store.acquirePairLease(
        secondGate!.pairId!,
        secondGate!.epoch,
        "future-worker",
        new Date("2026-09-07T13:00:01.000Z"),
        30,
      ),
    ).toBeNull();
    expect(
      await store.authorizeSuccessPublication(
        stalePublicationLease!,
        null,
        new Date("2026-09-07T13:00:01.000Z"),
      ),
    ).toBe("stale");
    expect(
      await store.commitGatePublication(
        stalePublicationLease!,
        12_345,
        "success",
        new Date("2026-09-07T13:00:01.000Z"),
      ),
    ).toBe(false);

    const client = await pool.connect();
    try {
      const delivery = await client.query(
        `SELECT scope, error_code, installation_id, repository_id
           FROM webhook_delivery WHERE delivery_id = $1`,
        ["global-invalid-json"],
      );
      expect(delivery.rows[0]).toMatchObject({
        scope: "global",
        error_code: "invalid_json",
        installation_id: null,
        repository_id: null,
      });
    } finally {
      client.release();
    }

    const revision = (await store.findActiveGate(99, "pull_request", "pr:8"))!.revision;
    expect(await deliverRaw(service, "global-invalid-json", "pull_request", invalidBody)).toMatchObject({
      duplicate: true,
      disposition: "applied",
    });
    expect((await store.findActiveGate(99, "pull_request", "pr:8"))!.revision).toBe(revision);

    const laterComment = issueCommentPayload({ number: 9, action: "created" });
    laterComment["repository"] = secondPayload["repository"]!;
    expect(await deliver(service, "post-quarantine-evidence", "issue_comment", laterComment)).toMatchObject({
      disposition: "stale",
    });
    await store.reconcileOpenPullRequests(
      {
        installationId: 71,
        repositoryId: 100,
        fullName: "elsewhere/other",
        owner: "elsewhere",
        name: "other",
      },
      [
        {
          number: 9,
          baseSha: BASE_B,
          headSha: HEAD_B,
          state: "open",
          merged: false,
          draft: false,
          sourceUpdatedAt: new Date("2026-09-07T12:05:00.000Z"),
        },
      ],
      "global-invalid-json",
      new Date("2026-09-07T13:00:02.000Z"),
    );
    expect(await store.findActiveGate(100, "pull_request", "pr:9")).toMatchObject({
      desiredState: "failure",
      quarantined: true,
    });

    expect(
      await deliver(
        service,
        "post-quarantine-new-head",
        "pull_request",
        {
          ...pullPayload({
            number: 9,
            baseSha: BASE_B,
            headSha: HEAD_A,
            updatedAt: "2026-09-07T12:06:00.000Z",
          }),
          repository: secondPayload["repository"],
        },
      ),
    ).toMatchObject({ disposition: "applied" });
    expect(await store.findActiveGate(100, "pull_request", "pr:9")).toMatchObject({
      headSha: HEAD_A,
      epoch: 2,
      desiredState: "revoked",
      quarantined: false,
      quarantineDeliveryId: null,
    });
    expect(await store.getGateSubject(secondGate!.id)).toMatchObject({
      active: false,
      desiredState: "failure",
      quarantined: true,
    });
  });

  it("globally quarantines malformed repository envelopes and delivery collisions", async () => {
    const { store, service, pool } = await harness();
    const original = pullPayload({
      number: 10,
      baseSha: BASE_A,
      headSha: HEAD_A,
      updatedAt: "2026-09-07T12:00:00.000Z",
    });
    await deliver(service, "collision-delivery", "pull_request", original);
    const gate = await store.findActiveGate(99, "pull_request", "pr:10");

    const malformedRepository = Buffer.from(
      JSON.stringify({ action: "synchronize", installation: { id: 71 }, repository: { id: "bad" } }),
    );
    expect(
      await deliverRaw(service, "bad-repository", "pull_request", malformedRepository),
    ).toMatchObject({ duplicate: false, disposition: "applied" });
    expect(await store.findActiveGate(99, "pull_request", "pr:10")).toMatchObject({
      desiredState: "failure",
      quarantined: true,
    });

    const collisionBody = Buffer.from("{different-signed-body");
    expect(
      await deliverRaw(service, "collision-delivery", "pull_request", collisionBody),
    ).toMatchObject({ duplicate: false, disposition: "applied" });
    const afterCollision = await store.getGateSubject(gate!.id);
    expect(afterCollision).toMatchObject({ desiredState: "failure", quarantined: true });
    const collisionRevision = afterCollision!.revision;
    expect(
      await deliverRaw(service, "collision-delivery", "pull_request", collisionBody),
    ).toMatchObject({ duplicate: true, disposition: "applied" });
    expect((await store.getGateSubject(gate!.id))!.revision).toBe(collisionRevision);

    const client = await pool.connect();
    try {
      const badRepositoryDelivery = await client.query(
        `SELECT scope, error_code FROM webhook_delivery WHERE delivery_id = $1`,
        ["bad-repository"],
      );
      expect(badRepositoryDelivery.rows[0]).toMatchObject({
        scope: "global",
        error_code: "invalid_repository_envelope",
      });
      const collision = await client.query(
        `SELECT event_name, payload_sha256
           FROM webhook_delivery_collision WHERE delivery_id = $1`,
        ["collision-delivery"],
      );
      expect(collision.rowCount).toBe(1);
      expect(collision.rows[0]?.event_name).toBe("pull_request");
    } finally {
      client.release();
    }
  });

  it("persists a dispatch plan and adopts its comment after a webhook advances the fence", async () => {
    const { store, service } = await harness();
    await deliver(
      service,
      "planned-pair",
      "pull_request",
      pullPayload({
        number: 7,
        baseSha: BASE_A,
        headSha: HEAD_A,
        updatedAt: "2026-09-07T12:00:00.000Z",
      }),
    );
    const gate = await store.findActiveGate(99, "pull_request", "pr:7");
    const plannedAt = new Date("2026-09-07T13:00:00.000Z");
    const lease = await store.acquirePairLease(
      gate!.pairId!,
      gate!.epoch,
      "dispatch-worker",
      plannedAt,
      30,
    );
    expect(await store.planDispatch(lease!, new Date(plannedAt.valueOf() + 1))).toBe(true);
    expect(await store.loadPairSnapshot(gate!.pairId!, gate!.epoch)).toMatchObject({
      dispatch: { state: "planned", commandCommentId: null },
    });
    expect(
      await store.beginDispatchAttempt(lease!, new Date(plannedAt.valueOf() + 1)),
    ).toBe(true);
    expect(await store.loadPairSnapshot(gate!.pairId!, gate!.epoch)).toMatchObject({
      dispatch: { state: "attempting", commandCommentId: null },
    });
    expect(
      await store.beginDispatchAttempt(lease!, new Date(plannedAt.valueOf() + 1)),
    ).toBe(false);

    await deliver(
      service,
      "planned-command-webhook",
      "issue_comment",
      issueCommentPayload({ number: 7, action: "created", id: 1_007 }),
    );
    expect(
      await store.adoptDispatch(
        lease!,
        1_007,
        new Date("2026-09-07T13:00:02.000Z"),
        1_800,
        new Date("2026-09-07T13:00:03.000Z"),
      ),
    ).toBe(true);
    expect(await store.loadPairSnapshot(gate!.pairId!, gate!.epoch)).toMatchObject({
      dispatch: {
        state: "sent",
        commandCommentId: 1_007,
        dispatchedAt: new Date("2026-09-07T13:00:02.000Z"),
        evidenceDeadlineAt: new Date("2026-09-07T13:30:02.000Z"),
      },
    });
  });

  it("atomically refuses a success commit after the persisted evidence deadline", async () => {
    const { store, service } = await harness();
    await deliver(
      service,
      "deadline-pair",
      "pull_request",
      pullPayload({
        number: 8,
        baseSha: BASE_A,
        headSha: HEAD_A,
        updatedAt: "2026-09-07T12:00:00.000Z",
      }),
    );
    const gate = await store.findActiveGate(99, "pull_request", "pr:8");
    const now = new Date();
    const dispatchLease = await store.acquirePairLease(
      gate!.pairId!,
      gate!.epoch,
      "deadline-worker",
      now,
      30,
    );
    expect(await store.planDispatch(dispatchLease!, now)).toBe(true);
    expect(await store.beginDispatchAttempt(dispatchLease!, now)).toBe(true);
    expect(
      await store.adoptDispatch(
        dispatchLease!,
        1_008,
        new Date(now.valueOf() - 120_000),
        60,
        now,
      ),
    ).toBe(true);
    const gateLease = await store.acquirePairLease(
      gate!.pairId!,
      gate!.epoch,
      "deadline-worker",
      now,
      30,
    );

    expect(await store.commitPairGate(gateLease!, "success", "covered", now)).toBe(true);
    expect(await store.findActiveGate(99, "pull_request", "pr:8")).toMatchObject({
      desiredState: "failure",
      desiredSummary: "The evidence deadline passed before success could commit.",
    });
  });

  it("expires queued success before publication when its deadline has passed", async () => {
    let databaseNow = new Date("2026-09-07T13:00:30.000Z");
    const appNow = new Date("2026-09-07T13:00:00.000Z");
    const { store, service } = await harness(appNow, () => databaseNow);
    await deliver(
      service,
      "publish-deadline-pair",
      "pull_request",
      pullPayload({
        number: 9,
        baseSha: BASE_A,
        headSha: HEAD_A,
        updatedAt: "2026-09-07T12:00:00.000Z",
      }),
    );
    const initialGate = await store.findActiveGate(99, "pull_request", "pr:9");
    const dispatchLease = await store.acquirePairLease(
      initialGate!.pairId!,
      initialGate!.epoch,
      "publish-worker",
      appNow,
      120,
    );
    expect(await store.planDispatch(dispatchLease!, appNow)).toBe(true);
    expect(await store.beginDispatchAttempt(dispatchLease!, appNow)).toBe(true);
    expect(
      await store.adoptDispatch(dispatchLease!, 1_009, appNow, 60, appNow),
    ).toBe(true);
    const pairLease = await store.acquirePairLease(
      initialGate!.pairId!,
      initialGate!.epoch,
      "publish-worker",
      appNow,
      120,
    );
    expect(await store.commitPairGate(pairLease!, "success", "covered", appNow)).toBe(true);
    const committedGate = await store.getGateSubject(initialGate!.id);
    expect(committedGate?.desiredState).toBe("success");
    const gateLease = await store.acquireGateLease(
      committedGate!.id,
      committedGate!.epoch,
      committedGate!.revision,
      "publish-worker",
      appNow,
      120,
    );
    databaseNow = new Date("2026-09-07T13:01:01.000Z");

    expect(
      await store.authorizeSuccessPublication(gateLease!, 1_500, appNow),
    ).toBe("expired");
    expect(await store.getGateSubject(committedGate!.id)).toMatchObject({
      desiredState: "failure",
      desiredSummary: "The evidence deadline passed before success publication.",
      checkRunId: 1_500,
    });
  });

});

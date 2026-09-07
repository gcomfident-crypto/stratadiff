import { describe, expect, it, vi } from "vitest";

import type { GovernorGithubClient } from "../src/github.js";
import type {
  GateLease,
  GateSubject,
  LivePullRequest,
  OutboxLease,
  PairLease,
  PairSnapshot,
} from "../src/types.js";
import { GovernorWorker, type WorkerStore } from "../src/worker.js";
import { BASE_A, HEAD_A } from "./helpers.js";

const NOW = new Date("2026-09-07T13:00:00.000Z");
const PAIR_ID = "2a69c552-ea21-46eb-8383-d8c5bb65dc26";

function outbox(topic: OutboxLease["topic"]): OutboxLease {
  return {
    id: 1,
    topic,
    aggregateId: PAIR_ID,
    aggregateEpoch: 1,
    payload: { pairId: PAIR_ID, epoch: 1, gateRevision: 1 },
    fence: 1,
    workerId: "worker-a",
  };
}

function pair(dispatch: PairSnapshot["dispatch"] = null): PairSnapshot {
  return {
    id: PAIR_ID,
    installationId: 71,
    repositoryId: 99,
    repositoryFullName: "acme/repo",
    pullNumber: 1,
    baseSha: BASE_A,
    headSha: HEAD_A,
    epoch: 1,
    active: true,
    quarantined: false,
    quarantineDeliveryId: null,
    draft: false,
    state: "open",
    dispatch,
    evidence: [],
  };
}

function lease(): PairLease {
  return {
    pairId: PAIR_ID,
    epoch: 1,
    fence: 4,
    workerId: "worker-a",
    expiresAt: new Date("2026-09-07T13:00:30.000Z"),
  };
}

function live(): LivePullRequest {
  return {
    number: 1,
    baseSha: BASE_A,
    headSha: HEAD_A,
    state: "open",
    merged: false,
    draft: false,
    sourceUpdatedAt: NOW,
  };
}

function fakeStore(overrides: Partial<WorkerStore>): WorkerStore {
  return {
    claimOutbox: vi.fn(async () => null),
    completeOutbox: vi.fn(async () => true),
    failOutbox: vi.fn(async () => true),
    acquireGateLease: vi.fn(async () => null),
    authorizeSuccessPublication: vi.fn(async () => "authorized"),
    getGateSubject: vi.fn(async () => null),
    rejectGatePublication: vi.fn(async () => true),
    supersedeStaleGatePublication: vi.fn(async () => true),
    commitGatePublication: vi.fn(async () => true),
    loadPairSnapshot: vi.fn(async () => null),
    acquirePairLease: vi.fn(async () => null),
    renewPairLease: vi.fn(async (current) => current),
    commitPairGate: vi.fn(async () => true),
    planDispatch: vi.fn(async () => true),
    beginDispatchAttempt: vi.fn(async () => true),
    adoptDispatch: vi.fn(async () => true),
    reconcileOpenPullRequests: vi.fn(async () => undefined),
    ...overrides,
  };
}

function fakeGithub(overrides: Partial<GovernorGithubClient> = {}): GovernorGithubClient {
  return {
    getPullRequest: vi.fn(async () => live()),
    createIssueComment: vi.fn(async () => ({ id: 800, createdAt: NOW })),
    findIssueComment: vi.fn(async () => null),
    publishGate: vi.fn(async () => 900),
    listOpenPullRequests: vi.fn(async () => []),
    ...overrides,
  };
}

function worker(store: WorkerStore, github = fakeGithub()): GovernorWorker {
  return new GovernorWorker(store, github, {
    workerId: "worker-a",
    leaseSeconds: 30,
    pollMs: 10,
    retrySeconds: 5,
    evidenceTimeoutSeconds: 1_800,
    now: () => NOW,
  });
}

describe("outbox worker fencing", () => {
  it("does not publish a queued success after database authorization expires", async () => {
    const subjectId = "38a32ea7-30cd-48e4-9e29-ebf0eaec627b";
    const work: OutboxLease = {
      ...outbox("publish_gate"),
      aggregateId: subjectId,
      payload: { subjectId, epoch: 1, revision: 2 },
    };
    const gateLease: GateLease = {
      subjectId,
      epoch: 1,
      revision: 2,
      fence: 4,
      workerId: "worker-a",
      expiresAt: new Date("2026-09-07T13:00:30.000Z"),
    };
    const subject: GateSubject = {
      id: subjectId,
      installationId: 71,
      repositoryId: 99,
      repositoryFullName: "acme/repo",
      subjectType: "pull_request",
      subjectKey: "pr:1",
      pairId: PAIR_ID,
      epoch: 1,
      revision: 2,
      headSha: HEAD_A,
      baseSha: BASE_A,
      active: true,
      quarantined: false,
      quarantineDeliveryId: null,
      desiredState: "success",
      desiredSummary: "covered",
      checkRunId: null,
      publishedRevision: null,
      publishedState: null,
    };
    const authorizeSuccessPublication = vi.fn(async () => "expired" as const);
    const completeOutbox = vi.fn(async () => true);
    const store = fakeStore({
      claimOutbox: vi.fn(async () => work),
      acquireGateLease: vi.fn(async () => gateLease),
      getGateSubject: vi.fn(async () => subject),
      loadPairSnapshot: vi.fn(async () => pair()),
      authorizeSuccessPublication,
      completeOutbox,
    });
    const publishGate = vi.fn(async () => 900);

    await worker(store, fakeGithub({ publishGate })).processOne();

    expect(authorizeSuccessPublication).toHaveBeenCalledWith(gateLease, null, NOW);
    expect(publishGate).not.toHaveBeenCalled();
    expect(completeOutbox).toHaveBeenCalledOnce();
  });

  it("retries rather than completing work when another worker holds the pair lease", async () => {
    const work = outbox("evaluate_pair");
    const completeOutbox = vi.fn(async () => true);
    const failOutbox = vi.fn(async () => true);
    const store = fakeStore({
      claimOutbox: vi.fn(async () => work),
      acquirePairLease: vi.fn(async () => null),
      loadPairSnapshot: vi.fn(async () => pair()),
      completeOutbox,
      failOutbox,
    });
    await expect(worker(store).processOne()).resolves.toBe(true);
    expect(failOutbox).toHaveBeenCalledOnce();
    expect(completeOutbox).not.toHaveBeenCalled();
  });

  it("completes an outbox item whose pair epoch no longer exists", async () => {
    const completeOutbox = vi.fn(async () => true);
    const store = fakeStore({
      claimOutbox: vi.fn(async () => outbox("evaluate_pair")),
      acquirePairLease: vi.fn(async () => null),
      loadPairSnapshot: vi.fn(async () => null),
      completeOutbox,
    });
    await worker(store).processOne();
    expect(completeOutbox).toHaveBeenCalledOnce();
  });

  it("completes stale dispatch work for a quarantined epoch without calling GitHub", async () => {
    const quarantined = pair();
    quarantined.quarantined = true;
    quarantined.quarantineDeliveryId = "global-invalid-json";
    const completeOutbox = vi.fn(async () => true);
    const store = fakeStore({
      claimOutbox: vi.fn(async () => outbox("dispatch_review")),
      loadPairSnapshot: vi.fn(async () => quarantined),
      completeOutbox,
    });
    const github = fakeGithub();

    await worker(store, github).processOne();

    expect(github.getPullRequest).not.toHaveBeenCalled();
    expect(github.findIssueComment).not.toHaveBeenCalled();
    expect(github.createIssueComment).not.toHaveBeenCalled();
    expect(completeOutbox).toHaveBeenCalledOnce();
  });

  it("recovers an uncertain App comment after a lost POST response without posting again", async () => {
    const attempting = pair({
      id: "f9f0849c-cb00-4052-b117-a76fd771d921",
      provider: "coderabbit",
      state: "attempting",
      command: "@coderabbitai full review",
      commandCommentId: null,
      dispatchedAt: null,
      evidenceDeadlineAt: null,
      plannedAt: new Date("2026-09-07T12:59:00.000Z"),
    });
    const adoptDispatch = vi.fn(async () => true);
    const store = fakeStore({
      claimOutbox: vi.fn(async () => outbox("dispatch_review")),
      loadPairSnapshot: vi.fn(async () => attempting),
      acquirePairLease: vi.fn(async () => lease()),
      adoptDispatch,
    });
    const createIssueComment = vi.fn(async () => ({ id: 801, createdAt: NOW }));
    const findIssueComment = vi.fn(async () => ({
      id: 800,
      createdAt: new Date("2026-09-07T12:59:30.000Z"),
    }));
    const github = fakeGithub({ createIssueComment, findIssueComment });

    await worker(store, github).processOne();

    expect(findIssueComment).toHaveBeenCalledOnce();
    expect(createIssueComment).not.toHaveBeenCalled();
    expect(adoptDispatch).toHaveBeenCalledWith(
      expect.objectContaining({ pairId: PAIR_ID, epoch: 1, fence: 4 }),
      800,
      new Date("2026-09-07T12:59:30.000Z"),
      1_800,
      NOW,
    );
  });

  it("never repeats an uncertain command POST while recovery is temporarily empty", async () => {
    let dispatchState: "planned" | "attempting" = "planned";
    let round = 0;
    const snapshot = () =>
      pair({
        id: "f9f0849c-cb00-4052-b117-a76fd771d921",
        provider: "coderabbit",
        state: dispatchState,
        command: "@coderabbitai full review",
        commandCommentId: null,
        dispatchedAt: null,
        evidenceDeadlineAt: null,
        plannedAt: new Date("2026-09-07T12:59:00.000Z"),
      });
    const completeOutbox = vi.fn(async () => true);
    const failOutbox = vi.fn(async () => true);
    const adoptDispatch = vi.fn(async () => true);
    const beginDispatchAttempt = vi.fn(async () => {
      dispatchState = "attempting";
      return true;
    });
    const store = fakeStore({
      claimOutbox: vi.fn(async () => {
        round += 1;
        return outbox("dispatch_review");
      }),
      loadPairSnapshot: vi.fn(async () => snapshot()),
      acquirePairLease: vi.fn(async () => lease()),
      beginDispatchAttempt,
      adoptDispatch,
      completeOutbox,
      failOutbox,
    });
    const createIssueComment = vi.fn(async () => {
      throw new Error("response lost after GitHub accepted the comment");
    });
    const findIssueComment = vi.fn(async () =>
      round < 3
        ? null
        : { id: 802, createdAt: new Date("2026-09-07T13:00:01.000Z") },
    );
    const github = fakeGithub({ createIssueComment, findIssueComment });
    const subject = worker(store, github);

    await subject.processOne();
    await subject.processOne();
    await subject.processOne();

    expect(beginDispatchAttempt).toHaveBeenCalledOnce();
    expect(createIssueComment).toHaveBeenCalledOnce();
    expect(findIssueComment).toHaveBeenCalledTimes(3);
    expect(failOutbox).toHaveBeenCalledTimes(2);
    expect(completeOutbox).toHaveBeenCalledOnce();
    expect(adoptDispatch).toHaveBeenCalledWith(
      expect.objectContaining({ pairId: PAIR_ID, epoch: 1 }),
      802,
      new Date("2026-09-07T13:00:01.000Z"),
      1_800,
      NOW,
    );
  });
});

import { evaluatePair } from "./evaluate.js";
import type { GovernorGithubClient } from "./github.js";
import type { PgGovernorStore } from "./store.js";
import type {
  JsonObject,
  JsonValue,
  OutboxLease,
  PairLease,
  PairSnapshot,
  RepositoryRef,
} from "./types.js";

export type WorkerStore = Pick<
  PgGovernorStore,
  | "claimOutbox"
  | "completeOutbox"
  | "failOutbox"
  | "acquireGateLease"
  | "authorizeSuccessPublication"
  | "getGateSubject"
  | "rejectGatePublication"
  | "supersedeStaleGatePublication"
  | "commitGatePublication"
  | "loadPairSnapshot"
  | "acquirePairLease"
  | "renewPairLease"
  | "commitPairGate"
  | "planDispatch"
  | "beginDispatchAttempt"
  | "adoptDispatch"
  | "beginRepositorySnapshot"
  | "reconcileOpenPullRequests"
>;

export interface WorkerOptions {
  workerId: string;
  leaseSeconds: number;
  pollMs: number;
  retrySeconds: number;
  evidenceTimeoutSeconds: number;
  now: () => Date;
}

function value(payload: JsonObject, key: string): JsonValue {
  const item = payload[key];
  if (item === undefined) {
    throw new Error(`outbox payload is missing ${key}`);
  }
  return item;
}

function string(payload: JsonObject, key: string): string {
  const item = value(payload, key);
  if (typeof item !== "string") {
    throw new Error(`outbox payload ${key} must be a string`);
  }
  return item;
}

function integer(payload: JsonObject, key: string): number {
  const item = value(payload, key);
  if (typeof item !== "number" || !Number.isSafeInteger(item) || item < 1) {
    throw new Error(`outbox payload ${key} must be a positive safe integer`);
  }
  return item;
}

function sleep(milliseconds: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const onAbort = () => {
      clearTimeout(timeout);
      signal.removeEventListener("abort", onAbort);
      resolve();
    };
    const timeout = setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, milliseconds);
    signal.addEventListener("abort", onAbort, { once: true });
  });
}

export class GovernorWorker {
  readonly #store: WorkerStore;
  readonly #github: GovernorGithubClient;
  readonly #options: WorkerOptions;

  constructor(store: WorkerStore, github: GovernorGithubClient, options: WorkerOptions) {
    this.#store = store;
    this.#github = github;
    this.#options = options;
  }

  async processOne(): Promise<boolean> {
    const lease = await this.#store.claimOutbox(
      this.#options.workerId,
      this.#options.now(),
      this.#options.leaseSeconds,
    );
    if (lease === null) {
      return false;
    }
    try {
      await this.#process(lease);
      await this.#store.completeOutbox(lease, this.#options.now());
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      await this.#store.failOutbox(
        lease,
        message,
        this.#options.now(),
        this.#options.retrySeconds,
      );
    }
    return true;
  }

  async run(signal: AbortSignal): Promise<void> {
    while (!signal.aborted) {
      const worked = await this.processOne();
      if (!worked) {
        await sleep(this.#options.pollMs, signal);
      }
    }
  }

  async #process(outbox: OutboxLease): Promise<void> {
    switch (outbox.topic) {
      case "publish_gate":
        await this.#publishGate(outbox);
        return;
      case "evaluate_pair":
        await this.#evaluatePair(outbox);
        return;
      case "dispatch_review":
        await this.#dispatchReview(outbox);
        return;
      case "reconcile_repository":
        await this.#reconcileRepository(outbox);
        return;
    }
  }

  async #publishGate(outbox: OutboxLease): Promise<void> {
    const subjectId = string(outbox.payload, "subjectId");
    const epoch = integer(outbox.payload, "epoch");
    const revision = integer(outbox.payload, "revision");
    const gateLease = await this.#store.acquireGateLease(
      subjectId,
      epoch,
      revision,
      this.#options.workerId,
      this.#options.now(),
      this.#options.leaseSeconds,
    );
    if (gateLease === null) {
      const current = await this.#store.getGateSubject(subjectId);
      if (current !== null && current.epoch === epoch && current.revision === revision) {
        throw new Error("gate subject lease is currently held by another worker");
      }
      return;
    }
    const subject = await this.#store.getGateSubject(subjectId);
    if (
      subject === null ||
      subject.epoch !== gateLease.epoch ||
      subject.revision !== gateLease.revision
    ) {
      return;
    }
    if (subject.subjectType === "pull_request" && subject.desiredState === "success") {
      if (subject.pairId === null) {
        throw new Error("pull-request gate is missing its pair binding");
      }
      const pair = await this.#store.loadPairSnapshot(subject.pairId, subject.epoch);
      if (pair === null) {
        await this.#store.rejectGatePublication(
          gateLease,
          "The pair disappeared before the success check could be published.",
          null,
          this.#options.now(),
        );
        return;
      }
      const live = await this.#github.getPullRequest(
        pair.installationId,
        pair.repositoryFullName,
        pair.pullNumber,
      );
      if (
        !pair.active ||
        live.state !== "open" ||
        live.draft ||
        live.baseSha !== pair.baseSha ||
        live.headSha !== pair.headSha
      ) {
        await this.#store.rejectGatePublication(
          gateLease,
          "Live pull-request base/head changed before success publication.",
          null,
          this.#options.now(),
        );
        return;
      }
      const authorization = await this.#store.authorizeSuccessPublication(
        gateLease,
        null,
        this.#options.now(),
      );
      if (authorization !== "authorized") {
        return;
      }
    }
    const checkRunId = await this.#github.publishGate(subject);
    if (subject.subjectType === "pull_request" && subject.desiredState === "success") {
      if (subject.pairId === null) {
        throw new Error("pull-request gate is missing its pair binding");
      }
      const authorization = await this.#store.authorizeSuccessPublication(
        gateLease,
        checkRunId,
        this.#options.now(),
      );
      if (authorization === "expired") {
        return;
      }
      if (authorization === "stale") {
        await this.#store.supersedeStaleGatePublication(
          subject.id,
          subject.epoch,
          subject.revision,
          checkRunId,
          this.#options.now(),
        );
        return;
      }
      const pair = await this.#store.loadPairSnapshot(subject.pairId, subject.epoch);
      if (pair === null) {
        await this.#store.rejectGatePublication(
          gateLease,
          "The pair disappeared while the success check was being published.",
          checkRunId,
          this.#options.now(),
        );
        return;
      }
      const live = await this.#github.getPullRequest(
        pair.installationId,
        pair.repositoryFullName,
        pair.pullNumber,
      );
      if (
        !pair.active ||
        live.state !== "open" ||
        live.draft ||
        live.baseSha !== pair.baseSha ||
        live.headSha !== pair.headSha
      ) {
        await this.#store.rejectGatePublication(
          gateLease,
          "Live pull-request base/head changed during success publication.",
          checkRunId,
          this.#options.now(),
        );
        return;
      }
    }
    const committed = await this.#store.commitGatePublication(
      gateLease,
      checkRunId,
      subject.desiredState,
      this.#options.now(),
    );
    if (!committed) {
      await this.#store.supersedeStaleGatePublication(
        subject.id,
        subject.epoch,
        subject.revision,
        checkRunId,
        this.#options.now(),
      );
    }
  }

  async #evaluatePair(outbox: OutboxLease): Promise<void> {
    const pairId = string(outbox.payload, "pairId");
    const epoch = integer(outbox.payload, "epoch");
    const lease = await this.#store.acquirePairLease(
      pairId,
      epoch,
      this.#options.workerId,
      this.#options.now(),
      this.#options.leaseSeconds,
    );
    if (lease === null) {
      const current = await this.#store.loadPairSnapshot(pairId, epoch);
      if (current !== null && current.active && !current.quarantined) {
        throw new Error("pair lease is currently held by another worker");
      }
      return;
    }
    const snapshot = await this.#store.loadPairSnapshot(pairId, epoch);
    if (snapshot === null) {
      return;
    }
    const decision = evaluatePair(snapshot, this.#options.now());
    if (decision.state === "success") {
      const live = await this.#github.getPullRequest(
        snapshot.installationId,
        snapshot.repositoryFullName,
        snapshot.pullNumber,
      );
      if (
        live.state !== "open" ||
        live.draft ||
        live.baseSha !== snapshot.baseSha ||
        live.headSha !== snapshot.headSha
      ) {
        const committed = await this.#store.commitPairGate(
          lease,
          "failure",
          "Live pull-request base/head changed before the success decision committed.",
          this.#options.now(),
        );
        if (!committed) {
          throw new Error("pair moved before the failure decision could commit");
        }
        return;
      }
    }
    const committed = await this.#store.commitPairGate(
      lease,
      decision.state,
      decision.summary,
      this.#options.now(),
    );
    if (!committed) {
      throw new Error("pair moved before the evidence decision could commit");
    }
  }

  async #dispatchReview(outbox: OutboxLease): Promise<void> {
    const pairId = string(outbox.payload, "pairId");
    const epoch = integer(outbox.payload, "epoch");
    const beforeLease = await this.#store.loadPairSnapshot(pairId, epoch);
    if (
      beforeLease === null ||
      beforeLease.quarantined ||
      beforeLease.dispatch?.state === "sent"
    ) {
      return;
    }
    let lease = await this.#store.acquirePairLease(
      pairId,
      epoch,
      this.#options.workerId,
      this.#options.now(),
      this.#options.leaseSeconds,
    );
    if (lease === null) {
      const current = await this.#store.loadPairSnapshot(pairId, epoch);
      if (
        current !== null &&
        current.active &&
        !current.quarantined &&
        current.dispatch?.state !== "sent"
      ) {
        throw new Error("pair lease is currently held by another worker");
      }
      return;
    }
    let snapshot = await this.#store.loadPairSnapshot(pairId, epoch);
    if (
      snapshot === null ||
      snapshot.quarantined ||
      snapshot.draft ||
      snapshot.state !== "open" ||
      !snapshot.active
    ) {
      return;
    }
    const liveBefore = await this.#github.getPullRequest(
      snapshot.installationId,
      snapshot.repositoryFullName,
      snapshot.pullNumber,
    );
    if (
      liveBefore.baseSha !== snapshot.baseSha ||
      liveBefore.headSha !== snapshot.headSha ||
      liveBefore.state !== "open" ||
      liveBefore.draft
    ) {
      const committed = await this.#store.commitPairGate(
        lease,
        "failure",
        "Live pull-request identity no longer matches this fenced pair.",
        this.#options.now(),
      );
      if (!committed) {
        throw new Error("pair moved before the dispatch rejection could commit");
      }
      return;
    }
    if (snapshot.dispatch === null) {
      const planned = await this.#store.planDispatch(lease, this.#options.now());
      if (!planned) {
        return;
      }
      const plannedSnapshot = await this.#store.loadPairSnapshot(pairId, epoch);
      if (plannedSnapshot === null || plannedSnapshot.dispatch?.state !== "planned") {
        throw new Error("planned dispatch could not be reloaded");
      }
      snapshot = plannedSnapshot;
    }
    if (
      snapshot.dispatch?.state !== "planned" &&
      snapshot.dispatch?.state !== "attempting"
    ) {
      return;
    }
    let mayPost = false;
    if (snapshot.dispatch.state === "planned") {
      lease = await this.#beginCommandAttempt(lease);
      mayPost = true;
    }
    const command = snapshot.dispatch.command;
    const recoveryWatermark = new Date(
      Math.floor(snapshot.dispatch.plannedAt.valueOf() / 1_000) * 1_000,
    );
    const recovered = await this.#github.findIssueComment(
      snapshot.installationId,
      snapshot.repositoryFullName,
      snapshot.pullNumber,
      command,
      recoveryWatermark,
    );
    let comment: { id: number; createdAt: Date };
    if (recovered === null) {
      if (!mayPost) {
        throw new Error("command POST outcome is uncertain; waiting for comment recovery");
      }
      comment = await this.#github.createIssueComment(
        snapshot.installationId,
        snapshot.repositoryFullName,
        snapshot.pullNumber,
        command,
      );
    } else {
      comment = recovered;
    }
    const liveAfter = await this.#github.getPullRequest(
      snapshot.installationId,
      snapshot.repositoryFullName,
      snapshot.pullNumber,
    );
    if (
      liveAfter.baseSha !== snapshot.baseSha ||
      liveAfter.headSha !== snapshot.headSha ||
      liveAfter.state !== "open" ||
      liveAfter.draft
    ) {
      const failedClosed = await this.#store.commitPairGate(
        lease,
        "failure",
        "Pull request moved while the provider command was being dispatched.",
        this.#options.now(),
      );
      if (!failedClosed) {
        throw new Error("pair moved while dispatch was in flight");
      }
      return;
    }
    const adopted = await this.#store.adoptDispatch(
      lease,
      comment.id,
      comment.createdAt,
      this.#options.evidenceTimeoutSeconds,
      this.#options.now(),
    );
    if (!adopted) {
      throw new Error("command comment could not be adopted into this dispatch epoch");
    }
  }

  async #beginCommandAttempt(lease: PairLease): Promise<PairLease> {
    const renewed = await this.#store.renewPairLease(
      lease,
      this.#options.now(),
      this.#options.leaseSeconds,
    );
    if (renewed === null) {
      throw new Error("dispatch lease expired before the command POST");
    }
    const began = await this.#store.beginDispatchAttempt(renewed, this.#options.now());
    if (!began) {
      throw new Error("dispatch attempt was already started by another worker");
    }
    return renewed;
  }

  async #reconcileRepository(outbox: OutboxLease): Promise<void> {
    const repository: RepositoryRef = {
      installationId: integer(outbox.payload, "installationId"),
      repositoryId: integer(outbox.payload, "repositoryId"),
      fullName: string(outbox.payload, "repositoryFullName"),
      owner: string(outbox.payload, "owner"),
      name: string(outbox.payload, "name"),
    };
    const token = await this.#store.beginRepositorySnapshot(
      repository,
      this.#options.now(),
    );
    const pulls = await this.#github.listOpenPullRequests(repository);
    const result = await this.#store.reconcileOpenPullRequests(
      token,
      pulls,
      string(outbox.payload, "deliveryId"),
      this.#options.now(),
    );
    if (result === "stale") {
      throw new Error("repository snapshot was invalidated before reconciliation");
    }
  }
}

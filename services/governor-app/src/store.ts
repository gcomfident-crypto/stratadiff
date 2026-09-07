import { createHash, randomUUID } from "node:crypto";

import type { Pool, PoolClient, QueryResult, QueryResultRow } from "pg";

import type {
  EvidenceInput,
  EvidenceRecord,
  GateLease,
  GateState,
  GateSubject,
  GithubImpact,
  IngestDisposition,
  IngestRequest,
  IngestResult,
  JsonObject,
  LivePullRequest,
  OutboxLease,
  OutboxTopic,
  PairLease,
  PairSnapshot,
  PullPairRef,
  RepositoryReconcileResult,
  RepositoryRef,
  RepositorySnapshotToken,
  UnscopedQuarantineRequest,
} from "./types.js";

interface DatabaseClient {
  query<R extends QueryResultRow = QueryResultRow>(
    text: string,
    values?: unknown[],
  ): Promise<QueryResult<R>>;
  release(): void;
}

export interface DatabasePool {
  connect(): Promise<DatabaseClient>;
  end(): Promise<void>;
}

interface PairRow extends QueryResultRow {
  id: string;
  installation_id: string;
  repository_id: string;
  repository_full_name: string;
  pull_number: number;
  base_sha: string;
  head_sha: string;
  epoch: string;
  fence: string;
  active: boolean;
  quarantined: boolean;
  quarantine_delivery_id: string | null;
  draft: boolean;
  pull_state: "open" | "closed";
  source_updated_at: Date;
}

interface GateRow extends QueryResultRow {
  id: string;
  installation_id: string;
  repository_id: string;
  repository_full_name: string;
  subject_type: "pull_request" | "merge_group";
  subject_key: string;
  pair_id: string | null;
  epoch: string;
  revision: string;
  fence: string;
  head_sha: string;
  base_sha: string;
  active: boolean;
  quarantined: boolean;
  quarantine_delivery_id: string | null;
  desired_state: GateState;
  desired_summary: string;
  check_run_id: string | null;
  published_revision: string | null;
  published_state: GateState | null;
}

interface DeadlineRow extends QueryResultRow {
  deadline_valid: boolean;
}

interface CountRow extends QueryResultRow {
  next_epoch: string;
}

interface GenerationRow extends QueryResultRow {
  generation: string;
}

interface DeliveryRow extends QueryResultRow {
  event_name: string;
  payload_sha256: string;
  disposition: "processing" | IngestDisposition;
}

interface LeaseRow extends QueryResultRow {
  id: string;
  epoch: string;
  fence: string;
  revision?: string;
  lease_expires_at: Date;
}

interface OutboxRow extends QueryResultRow {
  id: string;
  topic: OutboxTopic;
  aggregate_id: string | null;
  aggregate_epoch: string | null;
  payload: JsonObject;
  lease_fence: string;
}

interface DispatchRow extends QueryResultRow {
  id: string;
  provider: string;
  state: "planned" | "attempting" | "sent" | "abandoned";
  command: string;
  command_comment_id: string | null;
  dispatched_at: Date | null;
  evidence_deadline_at: Date | null;
  created_at: Date;
}

interface EvidenceRow extends QueryResultRow {
  pair_id: string;
  pair_epoch: string;
  provider: "coderabbit";
  kind: "issue_comment" | "review" | "status";
  source_id: string;
  source_updated_at: Date;
  valid: boolean;
  facts: JsonObject;
}

interface DeliveryEnvelope {
  deliveryId: string;
  eventName: string;
  payloadSha256: string;
  receivedAt: Date;
  repository: RepositoryRef | null;
  errorCode: UnscopedQuarantineRequest["errorCode"] | null;
}

type DeliveryClaim =
  | { kind: "new" }
  | { kind: "duplicate"; disposition: IngestDisposition }
  | { kind: "collision" }
  | { kind: "duplicate_collision" };

export interface GovernorStore {
  ingest(request: IngestRequest): Promise<IngestResult>;
  quarantineUnscopedSignedDelivery(
    request: UnscopedQuarantineRequest,
  ): Promise<IngestResult>;
}

function int(value: string | number): number {
  const parsed = typeof value === "number" ? value : Number(value);
  if (!Number.isSafeInteger(parsed)) {
    throw new Error(`database integer ${String(value)} is outside the safe range`);
  }
  return parsed;
}

function advisoryKey(repositoryId: number, subjectType: string, subjectKey: string): string {
  const digest = createHash("sha256")
    .update(`${repositoryId}:${subjectType}:${subjectKey}`)
    .digest();
  return digest.readBigInt64BE().toString();
}

const GLOBAL_INGEST_LOCK = advisoryKey(0, "global", "signed-webhook-ingest");

function gateFromRow(row: GateRow): GateSubject {
  return {
    id: row.id,
    installationId: int(row.installation_id),
    repositoryId: int(row.repository_id),
    repositoryFullName: row.repository_full_name,
    subjectType: row.subject_type,
    subjectKey: row.subject_key,
    pairId: row.pair_id,
    epoch: int(row.epoch),
    revision: int(row.revision),
    headSha: row.head_sha,
    baseSha: row.base_sha,
    active: row.active,
    quarantined: row.quarantined,
    quarantineDeliveryId: row.quarantine_delivery_id,
    desiredState: row.desired_state,
    desiredSummary: row.desired_summary,
    checkRunId: row.check_run_id === null ? null : int(row.check_run_id),
    publishedRevision:
      row.published_revision === null ? null : int(row.published_revision),
    publishedState: row.published_state,
  };
}

export class PgGovernorStore implements GovernorStore {
  readonly #pool: DatabasePool;

  constructor(pool: DatabasePool | Pool) {
    this.#pool = pool as DatabasePool;
  }

  async #transaction<T>(operation: (client: DatabaseClient) => Promise<T>): Promise<T> {
    const client = await this.#pool.connect();
    try {
      await client.query("BEGIN");
      const value = await operation(client);
      await client.query("COMMIT");
      return value;
    } catch (error) {
      await client.query("ROLLBACK");
      throw error;
    } finally {
      client.release();
    }
  }

  async ingest(request: IngestRequest): Promise<IngestResult> {
    return this.#transaction(async (client) => {
      await this.#lockGlobalIngest(client);
      const claim = await this.#claimDelivery(client, {
        deliveryId: request.deliveryId,
        eventName: request.eventName,
        payloadSha256: request.payloadSha256,
        receivedAt: request.receivedAt,
        repository: request.impact.repository,
        errorCode: null,
      });
      if (claim.kind === "duplicate") {
        return { duplicate: true, disposition: claim.disposition, touchedSubjects: [] };
      }
      if (claim.kind === "duplicate_collision") {
        return { duplicate: true, disposition: "applied", touchedSubjects: [] };
      }
      if (claim.kind === "collision") {
        const outcome = await this.#quarantineAll(client, request);
        return { duplicate: false, ...outcome };
      }

      const outcome = await this.#applyImpact(client, request);
      await client.query(
        `UPDATE webhook_delivery
            SET processed_at = $2, disposition = $3
          WHERE delivery_id = $1`,
        [request.deliveryId, request.receivedAt, outcome.disposition],
      );
      return { duplicate: false, ...outcome };
    });
  }

  async quarantineUnscopedSignedDelivery(
    request: UnscopedQuarantineRequest,
  ): Promise<IngestResult> {
    return this.#transaction(async (client) => {
      await this.#lockGlobalIngest(client);
      const claim = await this.#claimDelivery(client, {
        ...request,
        repository: null,
      });
      if (claim.kind === "duplicate") {
        return { duplicate: true, disposition: claim.disposition, touchedSubjects: [] };
      }
      if (claim.kind === "duplicate_collision") {
        return { duplicate: true, disposition: "applied", touchedSubjects: [] };
      }
      const outcome = await this.#quarantineAll(client, request);
      if (claim.kind === "new") {
        await client.query(
          `UPDATE webhook_delivery
              SET processed_at = $2, disposition = 'applied'
            WHERE delivery_id = $1`,
          [request.deliveryId, request.receivedAt],
        );
      }
      return { duplicate: false, ...outcome };
    });
  }

  async #lockGlobalIngest(client: DatabaseClient): Promise<void> {
    await client.query("SELECT pg_advisory_xact_lock($1::bigint)", [GLOBAL_INGEST_LOCK]);
  }

  async #advanceRepositoryGeneration(
    client: DatabaseClient,
    repositoryId: number,
    now: Date,
  ): Promise<number> {
    const result = await client.query<GenerationRow>(
      `INSERT INTO repository_reconcile_generation (repository_id, generation, updated_at)
       VALUES ($1, 1, $2)
       ON CONFLICT (repository_id) DO UPDATE
         SET generation = repository_reconcile_generation.generation + 1,
             updated_at = EXCLUDED.updated_at
       RETURNING generation`,
      [repositoryId, now],
    );
    const row = result.rows[0];
    if (row === undefined) {
      throw new Error(`failed to advance repository generation for ${repositoryId}`);
    }
    return int(row.generation);
  }

  async #advanceAllRepositoryGenerations(
    client: DatabaseClient,
    now: Date,
  ): Promise<void> {
    await client.query(
      `UPDATE repository_reconcile_generation
          SET generation = generation + 1, updated_at = $1`,
      [now],
    );
  }

  async #claimDelivery(
    client: DatabaseClient,
    request: DeliveryEnvelope,
  ): Promise<DeliveryClaim> {
    const existing = await client.query<DeliveryRow>(
      `SELECT event_name, payload_sha256, disposition
         FROM webhook_delivery
        WHERE delivery_id = $1
        FOR UPDATE`,
      [request.deliveryId],
    );
    const row = existing.rows[0];
    if (row !== undefined) {
      if (row.event_name === request.eventName && row.payload_sha256 === request.payloadSha256) {
        if (row.disposition === "processing") {
          throw new Error("persisted webhook delivery is still processing");
        }
        return { kind: "duplicate", disposition: row.disposition };
      }
      const existingCollision = await client.query(
        `SELECT delivery_id FROM webhook_delivery_collision
          WHERE delivery_id = $1 AND event_name = $2 AND payload_sha256 = $3`,
        [request.deliveryId, request.eventName, request.payloadSha256],
      );
      if (existingCollision.rowCount !== 0) {
        return { kind: "duplicate_collision" };
      }
      await client.query(
        `INSERT INTO webhook_delivery_collision (
           delivery_id, event_name, payload_sha256, received_at
         ) VALUES ($1, $2, $3, $4)`,
        [request.deliveryId, request.eventName, request.payloadSha256, request.receivedAt],
      );
      return { kind: "collision" };
    }
    await client.query(
      `INSERT INTO webhook_delivery (
         delivery_id, event_name, installation_id, repository_id,
         payload_sha256, received_at, scope, error_code
       ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)`,
      [
        request.deliveryId,
        request.eventName,
        request.repository?.installationId ?? null,
        request.repository?.repositoryId ?? null,
        request.payloadSha256,
        request.receivedAt,
        request.repository === null ? "global" : "repository",
        request.errorCode,
      ],
    );
    return { kind: "new" };
  }

  async #applyImpact(
    client: DatabaseClient,
    request: IngestRequest,
  ): Promise<Omit<IngestResult, "duplicate">> {
    const impact = request.impact;
    switch (impact.kind) {
      case "pull_request":
        return this.#applyPullRequest(client, impact.repository, impact.pair, request);
      case "pull_evidence":
        return this.#applyPullEvidence(client, impact, request);
      case "sha_evidence":
        return this.#applyShaEvidence(client, impact, request);
      case "repository_reconcile":
        return this.#applyRepositoryReconcile(client, impact.repository, request);
      case "merge_group":
        return this.#applyMergeGroup(client, impact, request);
      case "ignored":
        return { disposition: "ignored", touchedSubjects: [] };
    }
  }

  async #quarantineAll(
    client: DatabaseClient,
    request: Pick<IngestRequest, "deliveryId" | "receivedAt">,
  ): Promise<Omit<IngestResult, "duplicate">> {
    await this.#advanceAllRepositoryGenerations(client, request.receivedAt);
    const pairs = await client.query<PairRow>(
      `SELECT * FROM pr_pair
        WHERE active
        ORDER BY repository_id, pull_number
        FOR UPDATE`,
    );
    for (const pair of pairs.rows) {
      await client.query(
        `UPDATE pr_pair
            SET quarantined = true, quarantine_delivery_id = $2,
                fence = fence + 1, lease_owner = NULL, lease_expires_at = NULL,
                last_delivery_id = $2, updated_at = $3
          WHERE id = $1`,
        [pair.id, request.deliveryId, request.receivedAt],
      );
      await client.query(
        `UPDATE dispatch
            SET state = 'abandoned', evidence_deadline_at = NULL, updated_at = $3
          WHERE pair_id = $1 AND pair_epoch = $2 AND state <> 'abandoned'`,
        [pair.id, int(pair.epoch), request.receivedAt],
      );
    }

    const gates = await client.query<GateRow>(
      `SELECT * FROM gate_subject
        WHERE active
        ORDER BY repository_id, subject_type, subject_key
        FOR UPDATE`,
    );
    const touchedSubjects: string[] = [];
    for (const current of gates.rows) {
      const result = await client.query<GateRow>(
        `UPDATE gate_subject
            SET quarantined = true, quarantine_delivery_id = $2,
                desired_state = 'failure',
                desired_summary = 'An authenticated webhook could not be scoped; this gate is quarantined.',
                revision = revision + 1, fence = fence + 1,
                lease_owner = NULL, lease_expires_at = NULL,
                last_delivery_id = $2, updated_at = $3
          WHERE id = $1
          RETURNING *`,
        [current.id, request.deliveryId, request.receivedAt],
      );
      const row = result.rows[0];
      if (row === undefined) {
        throw new Error(`active gate subject disappeared during global quarantine: ${current.id}`);
      }
      const gate = gateFromRow(row);
      await this.#enqueueGatePublication(client, gate, request.receivedAt);
      touchedSubjects.push(gate.id);
    }
    return { disposition: "applied", touchedSubjects };
  }

  async #applyPullRequest(
    client: DatabaseClient,
    repository: RepositoryRef,
    pair: PullPairRef,
    request: IngestRequest,
    advanceRepositoryGeneration = true,
  ): Promise<Omit<IngestResult, "duplicate">> {
    await client.query("SELECT pg_advisory_xact_lock($1::bigint)", [
      advisoryKey(repository.repositoryId, "pull_request", String(pair.number)),
    ]);
    const currentResult = await client.query<PairRow>(
      `SELECT * FROM pr_pair
        WHERE repository_id = $1 AND pull_number = $2 AND active
        FOR UPDATE`,
      [repository.repositoryId, pair.number],
    );
    const current = currentResult.rows[0];
    if (current === undefined) {
      const latestResult = await client.query<PairRow>(
        `SELECT * FROM pr_pair
          WHERE repository_id = $1 AND pull_number = $2
          ORDER BY source_updated_at DESC, epoch DESC
          LIMIT 1
          FOR UPDATE`,
        [repository.repositoryId, pair.number],
      );
      const latest = latestResult.rows[0];
      if (
        latest !== undefined &&
        pair.sourceUpdatedAt.valueOf() <= new Date(latest.source_updated_at).valueOf()
      ) {
        return { disposition: "stale", touchedSubjects: [] };
      }
    }
    if (
      current !== undefined &&
      pair.sourceUpdatedAt < new Date(current.source_updated_at)
    ) {
      return { disposition: "stale", touchedSubjects: [] };
    }

    if (advanceRepositoryGeneration) {
      await this.#advanceRepositoryGeneration(
        client,
        repository.repositoryId,
        request.receivedAt,
      );
    }

    if (
      current !== undefined &&
      current.base_sha === pair.baseSha &&
      current.head_sha === pair.headSha
    ) {
      if (pair.sourceUpdatedAt.valueOf() === new Date(current.source_updated_at).valueOf()) {
        return { disposition: "stale", touchedSubjects: [] };
      }
      await client.query(
        `UPDATE pr_pair
            SET draft = $2, pull_state = $3, source_updated_at = $4,
                last_delivery_id = $5, updated_at = $6,
                fence = fence + 1, lease_owner = NULL, lease_expires_at = NULL,
                active = $7
          WHERE id = $1`,
        [
          current.id,
          pair.draft,
          pair.state,
          pair.sourceUpdatedAt,
          request.deliveryId,
          request.receivedAt,
          pair.state === "open",
        ],
      );
      if (current.quarantined) {
        const gate = await this.#revokeCurrentPairGate(
          client,
          current.id,
          request,
          "This review epoch remains quarantined after an unscoped authenticated webhook.",
          pair.state === "open",
          "failure",
        );
        return { disposition: "applied", touchedSubjects: [gate.id] };
      }
      const gate = await this.#revokeCurrentPairGate(
        client,
        current.id,
        request,
        pair.state === "open" ? "Pull request state changed; re-evaluating final-head evidence." : "Pull request closed.",
        pair.state === "open",
        pair.state === "open" ? "revoked" : "failure",
      );
      if (pair.state === "open" && !pair.draft) {
        await this.#enqueuePairWork(client, current.id, int(current.epoch), gate.revision, request);
      }
      return { disposition: "applied", touchedSubjects: [gate.id] };
    }

    if (current !== undefined) {
      await client.query(
        `UPDATE pr_pair
            SET active = false, fence = fence + 1,
                lease_owner = NULL, lease_expires_at = NULL, updated_at = $2
          WHERE id = $1`,
        [current.id, request.receivedAt],
      );
      await this.#revokeCurrentPairGate(
        client,
        current.id,
        request,
        "Superseded by a newer pull-request base/head pair.",
        false,
        "failure",
      );
    }

    const epochResult = await client.query<CountRow>(
      `SELECT COALESCE(MAX(epoch), 0) + 1 AS next_epoch
         FROM pr_pair
        WHERE repository_id = $1 AND pull_number = $2`,
      [repository.repositoryId, pair.number],
    );
    const epochRow = epochResult.rows[0];
    if (epochRow === undefined) {
      throw new Error("failed to allocate pair epoch");
    }
    const epoch = int(epochRow.next_epoch);
    const existingResult = await client.query<PairRow>(
      `SELECT * FROM pr_pair
        WHERE repository_id = $1 AND pull_number = $2
          AND base_sha = $3 AND head_sha = $4
        FOR UPDATE`,
      [repository.repositoryId, pair.number, pair.baseSha, pair.headSha],
    );
    const existing = existingResult.rows[0];
    const pairId = existing?.id ?? randomUUID();
    if (existing === undefined) {
      await client.query(
        `INSERT INTO pr_pair (
           id, installation_id, repository_id, repository_full_name, pull_number,
           base_sha, head_sha, epoch, active, draft, pull_state,
           source_updated_at, last_delivery_id, created_at, updated_at
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14)`,
        [
          pairId,
          repository.installationId,
          repository.repositoryId,
          repository.fullName,
          pair.number,
          pair.baseSha,
          pair.headSha,
          epoch,
          pair.state === "open",
          pair.draft,
          pair.state,
          pair.sourceUpdatedAt,
          request.deliveryId,
          request.receivedAt,
        ],
      );
    } else {
      await client.query(
        `UPDATE pr_pair
            SET installation_id = $2, repository_full_name = $3, epoch = $4,
                fence = fence + 1, lease_owner = NULL, lease_expires_at = NULL,
                active = $5, draft = $6, pull_state = $7,
                source_updated_at = $8, last_delivery_id = $9, updated_at = $10,
                quarantined = false, quarantine_delivery_id = NULL
          WHERE id = $1`,
        [
          pairId,
          repository.installationId,
          repository.fullName,
          epoch,
          pair.state === "open",
          pair.draft,
          pair.state,
          pair.sourceUpdatedAt,
          request.deliveryId,
          request.receivedAt,
        ],
      );
    }

    const gate = await this.#createPairGate(client, repository, pair, pairId, epoch, request);
    if (pair.state === "open" && !pair.draft) {
      await this.#enqueuePairWork(client, pairId, epoch, gate.revision, request);
    }
    return { disposition: "applied", touchedSubjects: [gate.id] };
  }

  async #createPairGate(
    client: DatabaseClient,
    repository: RepositoryRef,
    pair: PullPairRef,
    pairId: string,
    epoch: number,
    request: IngestRequest,
  ): Promise<GateSubject> {
    await client.query(
      `UPDATE gate_subject
          SET active = false, desired_state = 'failure',
              desired_summary = 'Superseded by a newer final-head epoch.',
              revision = revision + 1, fence = fence + 1,
              lease_owner = NULL, lease_expires_at = NULL,
              last_delivery_id = $4, updated_at = $5
        WHERE repository_id = $1 AND subject_type = 'pull_request'
          AND subject_key = $2 AND active AND epoch <> $3`,
      [repository.repositoryId, `pr:${pair.number}`, epoch, request.deliveryId, request.receivedAt],
    );
    const id = randomUUID();
    const active = pair.state === "open";
    const result = await client.query<GateRow>(
      `INSERT INTO gate_subject (
         id, installation_id, repository_id, repository_full_name,
         subject_type, subject_key, pair_id, epoch, head_sha, base_sha,
         active, desired_state, desired_summary, last_delivery_id,
         created_at, updated_at
       ) VALUES ($1, $2, $3, $4, 'pull_request', $5, $6, $7, $8, $9,
                 $10, $11, $12, $13, $14, $14)
       RETURNING *`,
      [
        id,
        repository.installationId,
        repository.repositoryId,
        repository.fullName,
        `pr:${pair.number}`,
        pairId,
        epoch,
        pair.headSha,
        pair.baseSha,
        active,
        pair.state === "open" ? "revoked" : "failure",
        pair.draft
          ? "Draft pull request; final-head review is deferred."
          : pair.state === "open"
            ? "New final-head input observed; evidence has not been evaluated."
            : "Pull request closed without current final-head evidence.",
        request.deliveryId,
        request.receivedAt,
      ],
    );
    const row = result.rows[0];
    if (row === undefined) {
      throw new Error("failed to create pull-request gate subject");
    }
    const gate = gateFromRow(row);
    await this.#enqueueGatePublication(client, gate, request.receivedAt);
    return gate;
  }

  async #revokeCurrentPairGate(
    client: DatabaseClient,
    pairId: string,
    request: IngestRequest,
    summary: string,
    active: boolean,
    desiredState: GateState = "revoked",
  ): Promise<GateSubject> {
    const result = await client.query<GateRow>(
      `UPDATE gate_subject
          SET desired_state = $2, desired_summary = $3, active = $4,
              revision = revision + 1, fence = fence + 1,
              lease_owner = NULL, lease_expires_at = NULL,
              last_delivery_id = $5, updated_at = $6
        WHERE pair_id = $1 AND active
        RETURNING *`,
      [pairId, desiredState, summary, active, request.deliveryId, request.receivedAt],
    );
    const row = result.rows[0];
    if (row === undefined) {
      throw new Error(`active gate subject is missing for pair ${pairId}`);
    }
    const gate = gateFromRow(row);
    await this.#enqueueGatePublication(client, gate, request.receivedAt);
    return gate;
  }

  async #applyPullEvidence(
    client: DatabaseClient,
    impact: Extract<GithubImpact, { kind: "pull_evidence" }>,
    request: IngestRequest,
  ): Promise<Omit<IngestResult, "duplicate">> {
    const pairResult = await client.query<PairRow>(
      `SELECT * FROM pr_pair
        WHERE repository_id = $1 AND pull_number = $2 AND active
        FOR UPDATE`,
      [impact.repository.repositoryId, impact.pullNumber],
    );
    const pair = pairResult.rows[0];
    if (
      pair === undefined ||
      pair.quarantined ||
      (impact.expectedHeadSha !== null && impact.expectedHeadSha !== pair.head_sha)
    ) {
      return { disposition: "stale", touchedSubjects: [] };
    }
    const gate = await this.#invalidatePairForEvidence(client, pair, request);
    await this.#upsertEvidence(client, pair, impact.evidence, request);
    await this.#enqueueEvaluation(client, pair.id, int(pair.epoch), gate.revision, request.receivedAt);
    return { disposition: "applied", touchedSubjects: [gate.id] };
  }

  async #applyShaEvidence(
    client: DatabaseClient,
    impact: Extract<GithubImpact, { kind: "sha_evidence" }>,
    request: IngestRequest,
  ): Promise<Omit<IngestResult, "duplicate">> {
    const pairs = await client.query<PairRow>(
      `SELECT * FROM pr_pair
        WHERE repository_id = $1 AND head_sha = $2 AND active AND NOT quarantined
        ORDER BY pull_number
        FOR UPDATE`,
      [impact.repository.repositoryId, impact.headSha],
    );
    const touchedSubjects: string[] = [];
    for (const pair of pairs.rows) {
      const gate = await this.#invalidatePairForEvidence(client, pair, request);
      await this.#upsertEvidence(client, pair, impact.evidence, request);
      await this.#enqueueEvaluation(client, pair.id, int(pair.epoch), gate.revision, request.receivedAt);
      touchedSubjects.push(gate.id);
    }
    return {
      disposition: pairs.rows.length === 0 ? "stale" : "applied",
      touchedSubjects,
    };
  }

  async #invalidatePairForEvidence(
    client: DatabaseClient,
    pair: PairRow,
    request: IngestRequest,
  ): Promise<GateSubject> {
    await client.query(
      `UPDATE pr_pair
          SET fence = fence + 1, lease_owner = NULL, lease_expires_at = NULL,
              last_delivery_id = $2, updated_at = $3
        WHERE id = $1`,
      [pair.id, request.deliveryId, request.receivedAt],
    );
    return this.#revokeCurrentPairGate(
      client,
      pair.id,
      request,
      "Provider evidence changed; the final-head decision is being re-evaluated.",
      true,
    );
  }

  async #upsertEvidence(
    client: DatabaseClient,
    pair: PairRow,
    evidence: EvidenceInput,
    request: IngestRequest,
  ): Promise<void> {
    await client.query(
      `INSERT INTO evidence (
         id, pair_id, pair_epoch, provider, kind, source_id,
         source_updated_at, valid, facts, delivery_id, created_at, updated_at
       ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::jsonb, $10, $11, $11)
       ON CONFLICT (pair_id, pair_epoch, kind, source_id) DO UPDATE
         SET source_updated_at = EXCLUDED.source_updated_at,
             valid = EXCLUDED.valid,
             facts = EXCLUDED.facts,
             delivery_id = EXCLUDED.delivery_id,
             updated_at = EXCLUDED.updated_at
       WHERE EXCLUDED.source_updated_at > evidence.source_updated_at
          OR (
            EXCLUDED.source_updated_at = evidence.source_updated_at
            AND evidence.valid AND NOT EXCLUDED.valid
          )`,
      [
        randomUUID(),
        pair.id,
        int(pair.epoch),
        evidence.provider,
        evidence.kind,
        evidence.sourceId,
        evidence.sourceUpdatedAt,
        evidence.valid,
        JSON.stringify(evidence.facts),
        request.deliveryId,
        request.receivedAt,
      ],
    );
  }

  async #applyRepositoryReconcile(
    client: DatabaseClient,
    repository: RepositoryRef,
    request: IngestRequest,
  ): Promise<Omit<IngestResult, "duplicate">> {
    const malformed =
      request.impact.kind === "repository_reconcile" &&
      request.impact.reason === "malformed_signed_event";
    await this.#advanceRepositoryGeneration(
      client,
      repository.repositoryId,
      request.receivedAt,
    );
    const pairs = await client.query<PairRow>(
      `SELECT * FROM pr_pair
        WHERE repository_id = $1 AND active
        ORDER BY pull_number
        FOR UPDATE`,
      [repository.repositoryId],
    );
    const touchedSubjects: string[] = [];
    for (const pair of pairs.rows) {
      let gate: GateSubject;
      if (malformed) {
        await client.query(
          `UPDATE pr_pair
              SET quarantined = true, quarantine_delivery_id = $2,
                  fence = fence + 1, lease_owner = NULL, lease_expires_at = NULL,
                  last_delivery_id = $2, updated_at = $3
            WHERE id = $1`,
          [pair.id, request.deliveryId, request.receivedAt],
        );
        await client.query(
          `UPDATE dispatch
              SET state = 'abandoned', evidence_deadline_at = NULL, updated_at = $3
            WHERE pair_id = $1 AND pair_epoch = $2 AND state <> 'abandoned'`,
          [pair.id, int(pair.epoch), request.receivedAt],
        );
        await client.query(
          `UPDATE gate_subject
              SET quarantined = true, quarantine_delivery_id = $2
            WHERE pair_id = $1 AND active`,
          [pair.id, request.deliveryId],
        );
        gate = await this.#revokeCurrentPairGate(
          client,
          pair.id,
          request,
          "Malformed signed webhook quarantined this review epoch; new evidence cannot reopen it.",
          true,
          "failure",
        );
      } else if (pair.quarantined) {
        continue;
      } else {
        gate = await this.#invalidatePairForEvidence(client, pair, request);
      }
      touchedSubjects.push(gate.id);
    }
    const mergeGroups = await client.query<GateRow>(
      `SELECT * FROM gate_subject
        WHERE repository_id = $1 AND subject_type = 'merge_group' AND active
        FOR UPDATE`,
      [repository.repositoryId],
    );
    for (const mergeGroup of mergeGroups.rows) {
      if (!malformed && mergeGroup.quarantined) {
        continue;
      }
      const result = await client.query<GateRow>(
        `UPDATE gate_subject
            SET quarantined = CASE WHEN $2::boolean THEN true ELSE quarantined END,
                quarantine_delivery_id = CASE WHEN $2::boolean THEN $5 ELSE quarantine_delivery_id END,
                desired_state = $3,
                desired_summary = $4,
                revision = revision + 1, fence = fence + 1,
                lease_owner = NULL, lease_expires_at = NULL,
                last_delivery_id = $5, updated_at = $6
          WHERE id = $1
          RETURNING *`,
        [
          mergeGroup.id,
          malformed,
          malformed ? "failure" : "revoked",
          malformed
            ? "Malformed signed webhook quarantined this merge-group gate."
            : "Repository push observed; merge-group input must be revalidated.",
          request.deliveryId,
          request.receivedAt,
        ],
      );
      const row = result.rows[0];
      if (row !== undefined) {
        const gate = gateFromRow(row);
        await this.#enqueueGatePublication(client, gate, request.receivedAt);
        touchedSubjects.push(gate.id);
      }
    }
    await this.#enqueue(
      client,
      "reconcile_repository",
      null,
      null,
      {
        installationId: repository.installationId,
        repositoryId: repository.repositoryId,
        repositoryFullName: repository.fullName,
        owner: repository.owner,
        name: repository.name,
        deliveryId: request.deliveryId,
      },
      `reconcile:${repository.repositoryId}:${request.deliveryId}`,
      request.receivedAt,
    );
    return { disposition: "applied", touchedSubjects };
  }

  async #applyMergeGroup(
    client: DatabaseClient,
    impact: Extract<GithubImpact, { kind: "merge_group" }>,
    request: IngestRequest,
  ): Promise<Omit<IngestResult, "duplicate">> {
    const subjectKey = `merge-group:${impact.headRef}`;
    await client.query("SELECT pg_advisory_xact_lock($1::bigint)", [
      advisoryKey(impact.repository.repositoryId, "merge_group", subjectKey),
    ]);
    const currentResult = await client.query<GateRow>(
      `SELECT * FROM gate_subject
        WHERE repository_id = $1 AND subject_type = 'merge_group'
          AND subject_key = $2 AND active
        FOR UPDATE`,
      [impact.repository.repositoryId, subjectKey],
    );
    const current = currentResult.rows[0];
    if (
      impact.action === "checks_requested" &&
      current !== undefined &&
      current.head_sha === impact.headSha &&
      current.quarantined
    ) {
      return { disposition: "applied", touchedSubjects: [] };
    }
    if (impact.action === "destroyed") {
      if (current === undefined || current.head_sha !== impact.headSha) {
        const historical = await client.query<GateRow>(
          `SELECT * FROM gate_subject
            WHERE repository_id = $1 AND subject_type = 'merge_group'
              AND subject_key = $2 AND head_sha = $3
            ORDER BY epoch DESC
            LIMIT 1
            FOR UPDATE`,
          [impact.repository.repositoryId, subjectKey, impact.headSha],
        );
        if (historical.rows[0] !== undefined) {
          return { disposition: "stale", touchedSubjects: [] };
        }
        const epochResult = await client.query<CountRow>(
          `SELECT COALESCE(MAX(epoch), 0) + 1 AS next_epoch
             FROM gate_subject
            WHERE repository_id = $1 AND subject_type = 'merge_group' AND subject_key = $2`,
          [impact.repository.repositoryId, subjectKey],
        );
        const epochRow = epochResult.rows[0];
        if (epochRow === undefined) {
          throw new Error("failed to allocate destroyed merge-group epoch");
        }
        const tombstoneResult = await client.query<GateRow>(
          `INSERT INTO gate_subject (
             id, installation_id, repository_id, repository_full_name,
             subject_type, subject_key, pair_id, epoch, head_sha, base_sha,
             active, desired_state, desired_summary, last_delivery_id,
             created_at, updated_at
           ) VALUES ($1, $2, $3, $4, 'merge_group', $5, NULL, $6, $7, $8,
                     false, 'cancelled', 'GitHub destroyed this merge group.', $9, $10, $10)
           RETURNING *`,
          [
            randomUUID(),
            impact.repository.installationId,
            impact.repository.repositoryId,
            impact.repository.fullName,
            subjectKey,
            int(epochRow.next_epoch),
            impact.headSha,
            impact.baseSha,
            request.deliveryId,
            request.receivedAt,
          ],
        );
        const tombstone = tombstoneResult.rows[0];
        if (tombstone === undefined) {
          throw new Error("failed to create destroyed merge-group tombstone");
        }
        const gate = gateFromRow(tombstone);
        await this.#enqueueGatePublication(client, gate, request.receivedAt);
        return { disposition: "applied", touchedSubjects: [gate.id] };
      }
      const result = await client.query<GateRow>(
        `UPDATE gate_subject
            SET active = false, desired_state = 'cancelled',
                desired_summary = 'GitHub destroyed this merge group.',
                revision = revision + 1, fence = fence + 1,
                lease_owner = NULL, lease_expires_at = NULL,
                last_delivery_id = $2, updated_at = $3
          WHERE id = $1
          RETURNING *`,
        [current.id, request.deliveryId, request.receivedAt],
      );
      const row = result.rows[0];
      if (row === undefined) {
        throw new Error("failed to cancel merge-group gate");
      }
      const gate = gateFromRow(row);
      await this.#enqueueGatePublication(client, gate, request.receivedAt);
      return { disposition: "applied", touchedSubjects: [gate.id] };
    }

    const sameHeadHistory = await client.query<GateRow>(
      `SELECT * FROM gate_subject
        WHERE repository_id = $1 AND subject_type = 'merge_group'
          AND subject_key = $2 AND head_sha = $3
        ORDER BY epoch DESC
        LIMIT 1
        FOR UPDATE`,
      [impact.repository.repositoryId, subjectKey, impact.headSha],
    );
    const previousSameHead = sameHeadHistory.rows[0];
    if (previousSameHead !== undefined && !previousSameHead.active) {
      return { disposition: "stale", touchedSubjects: [] };
    }

    if (current !== undefined && current.head_sha === impact.headSha) {
      const result = await client.query<GateRow>(
        `UPDATE gate_subject
            SET desired_state = 'revoked',
                desired_summary = 'Merge-group head observed; native final-head evidence is pending.',
                revision = revision + 1, fence = fence + 1,
                lease_owner = NULL, lease_expires_at = NULL,
                last_delivery_id = $2, updated_at = $3
          WHERE id = $1
          RETURNING *`,
        [current.id, request.deliveryId, request.receivedAt],
      );
      const row = result.rows[0];
      if (row === undefined) {
        throw new Error("failed to revoke merge-group gate");
      }
      const gate = gateFromRow(row);
      await this.#enqueueGatePublication(client, gate, request.receivedAt);
      return { disposition: "applied", touchedSubjects: [gate.id] };
    }

    if (current !== undefined) {
      const oldResult = await client.query<GateRow>(
        `UPDATE gate_subject
            SET active = false, desired_state = 'cancelled',
                desired_summary = 'Superseded by a newer merge-group head.',
                revision = revision + 1, fence = fence + 1,
                lease_owner = NULL, lease_expires_at = NULL,
                last_delivery_id = $2, updated_at = $3
          WHERE id = $1
          RETURNING *`,
        [current.id, request.deliveryId, request.receivedAt],
      );
      const oldRow = oldResult.rows[0];
      if (oldRow !== undefined) {
        await this.#enqueueGatePublication(client, gateFromRow(oldRow), request.receivedAt);
      }
    }
    const epochResult = await client.query<CountRow>(
      `SELECT COALESCE(MAX(epoch), 0) + 1 AS next_epoch
         FROM gate_subject
        WHERE repository_id = $1 AND subject_type = 'merge_group' AND subject_key = $2`,
      [impact.repository.repositoryId, subjectKey],
    );
    const epochRow = epochResult.rows[0];
    if (epochRow === undefined) {
      throw new Error("failed to allocate merge-group epoch");
    }
    const epoch = int(epochRow.next_epoch);
    const result = await client.query<GateRow>(
      `INSERT INTO gate_subject (
         id, installation_id, repository_id, repository_full_name,
         subject_type, subject_key, pair_id, epoch, head_sha, base_sha,
         active, desired_state, desired_summary, last_delivery_id,
         created_at, updated_at
       ) VALUES ($1, $2, $3, $4, 'merge_group', $5, NULL, $6, $7, $8,
                 true, 'revoked', $9, $10, $11, $11)
       RETURNING *`,
      [
        randomUUID(),
        impact.repository.installationId,
        impact.repository.repositoryId,
        impact.repository.fullName,
        subjectKey,
        epoch,
        impact.headSha,
        impact.baseSha,
        "Merge-group head is the gate subject; pull-request checks cannot satisfy it.",
        request.deliveryId,
        request.receivedAt,
      ],
    );
    const row = result.rows[0];
    if (row === undefined) {
      throw new Error("failed to create merge-group gate");
    }
    const gate = gateFromRow(row);
    await this.#enqueueGatePublication(client, gate, request.receivedAt);
    return { disposition: "applied", touchedSubjects: [gate.id] };
  }

  async #enqueuePairWork(
    client: DatabaseClient,
    pairId: string,
    epoch: number,
    revision: number,
    request: IngestRequest,
  ): Promise<void> {
    await this.#enqueue(
      client,
      "dispatch_review",
      pairId,
      epoch,
      { pairId, epoch },
      `dispatch:${pairId}:${epoch}`,
      request.receivedAt,
    );
    await this.#enqueueEvaluation(client, pairId, epoch, revision, request.receivedAt);
  }

  async #enqueueEvaluation(
    client: DatabaseClient,
    pairId: string,
    epoch: number,
    revision: number,
    now: Date,
  ): Promise<void> {
    await this.#enqueue(
      client,
      "evaluate_pair",
      pairId,
      epoch,
      { pairId, epoch, gateRevision: revision },
      `evaluate:${pairId}:${epoch}:${revision}`,
      now,
    );
  }

  async #enqueueGatePublication(
    client: DatabaseClient,
    gate: GateSubject,
    now: Date,
  ): Promise<void> {
    await this.#enqueue(
      client,
      "publish_gate",
      gate.id,
      gate.epoch,
      { subjectId: gate.id, epoch: gate.epoch, revision: gate.revision },
      `publish:${gate.id}:${gate.revision}`,
      now,
    );
  }

  async #enqueue(
    client: DatabaseClient,
    topic: OutboxTopic,
    aggregateId: string | null,
    aggregateEpoch: number | null,
    payload: JsonObject,
    dedupeKey: string,
    now: Date,
  ): Promise<void> {
    await client.query(
      `INSERT INTO outbox (
         topic, aggregate_id, aggregate_epoch, payload, dedupe_key,
         available_at, created_at
       ) VALUES ($1, $2, $3, $4::jsonb, $5, $6, $6)
       ON CONFLICT (dedupe_key) DO NOTHING`,
      [topic, aggregateId, aggregateEpoch, JSON.stringify(payload), dedupeKey, now],
    );
  }

  async acquirePairLease(
    pairId: string,
    expectedEpoch: number,
    workerId: string,
    now: Date,
    leaseSeconds: number,
  ): Promise<PairLease | null> {
    const expiresAt = new Date(now.valueOf() + leaseSeconds * 1_000);
    const result = await this.#transaction((client) =>
      client.query<LeaseRow>(
        `UPDATE pr_pair
            SET fence = fence + 1, lease_owner = $3, lease_expires_at = $4,
                updated_at = $5
          WHERE id = $1 AND epoch = $2 AND active AND NOT quarantined
            AND (lease_owner IS NULL OR lease_expires_at <= $5 OR lease_owner = $3)
          RETURNING id, epoch, fence, lease_expires_at`,
        [pairId, expectedEpoch, workerId, expiresAt, now],
      ),
    );
    const row = result.rows[0];
    return row === undefined
      ? null
      : {
          pairId: row.id,
          epoch: int(row.epoch),
          fence: int(row.fence),
          workerId,
          expiresAt: new Date(row.lease_expires_at),
        };
  }

  async renewPairLease(
    lease: PairLease,
    now: Date,
    leaseSeconds: number,
  ): Promise<PairLease | null> {
    const expiresAt = new Date(now.valueOf() + leaseSeconds * 1_000);
    const result = await this.#transaction((client) =>
      client.query<LeaseRow>(
        `UPDATE pr_pair
            SET lease_expires_at = $5, updated_at = $6
          WHERE id = $1 AND epoch = $2 AND fence = $3 AND lease_owner = $4
            AND lease_expires_at > $6 AND active AND NOT quarantined
          RETURNING id, epoch, fence, lease_expires_at`,
        [lease.pairId, lease.epoch, lease.fence, lease.workerId, expiresAt, now],
      ),
    );
    const row = result.rows[0];
    return row === undefined
      ? null
      : {
          pairId: row.id,
          epoch: int(row.epoch),
          fence: int(row.fence),
          workerId: lease.workerId,
          expiresAt: new Date(row.lease_expires_at),
        };
  }

  async commitPairGate(
    lease: PairLease,
    desiredState: GateState,
    summary: string,
    now: Date,
  ): Promise<boolean> {
    return this.#transaction(async (client) => {
      const pair = await client.query<PairRow>(
        `UPDATE pr_pair
            SET lease_owner = NULL, lease_expires_at = NULL, updated_at = $5
          WHERE id = $1 AND epoch = $2 AND fence = $3 AND lease_owner = $4
            AND lease_expires_at > $5 AND active AND NOT quarantined
          RETURNING *`,
        [lease.pairId, lease.epoch, lease.fence, lease.workerId, now],
      );
      if (pair.rowCount === 0) {
        return false;
      }
      const gateResult = await client.query<GateRow>(
        `UPDATE gate_subject
            SET desired_state = CASE
                  WHEN $3::text = 'success' AND NOT EXISTS (
                    SELECT 1
                      FROM dispatch
                     WHERE pair_id = $1 AND pair_epoch = $2
                       AND provider = 'coderabbit' AND state = 'sent'
                       AND evidence_deadline_at > statement_timestamp()
                  ) THEN 'failure'
                  ELSE $3
                END,
                desired_summary = CASE
                  WHEN $3::text = 'success' AND NOT EXISTS (
                    SELECT 1
                      FROM dispatch
                     WHERE pair_id = $1 AND pair_epoch = $2
                       AND provider = 'coderabbit' AND state = 'sent'
                       AND evidence_deadline_at > statement_timestamp()
                  ) THEN 'The evidence deadline passed before success could commit.'
                  ELSE $4
                END,
                revision = revision + 1, fence = fence + 1,
                lease_owner = NULL, lease_expires_at = NULL, updated_at = $5
          WHERE pair_id = $1 AND epoch = $2 AND active
          RETURNING *`,
        [lease.pairId, lease.epoch, desiredState, summary, now],
      );
      const gateRow = gateResult.rows[0];
      if (gateRow === undefined) {
        throw new Error("active gate subject is missing during fenced pair commit");
      }
      await this.#enqueueGatePublication(client, gateFromRow(gateRow), now);
      return true;
    });
  }

  async beginDispatchAttempt(lease: PairLease, now: Date): Promise<boolean> {
    return this.#transaction(async (client) => {
      const pair = await client.query<PairRow>(
        `SELECT * FROM pr_pair
          WHERE id = $1 AND epoch = $2 AND fence = $3 AND lease_owner = $4
            AND lease_expires_at > $5 AND active AND NOT quarantined
          FOR UPDATE`,
        [lease.pairId, lease.epoch, lease.fence, lease.workerId, now],
      );
      if (pair.rowCount === 0) {
        return false;
      }
      const dispatch = await client.query(
        `UPDATE dispatch
            SET state = 'attempting', updated_at = $3
          WHERE pair_id = $1 AND pair_epoch = $2
            AND provider = 'coderabbit' AND state = 'planned'
          RETURNING id`,
        [lease.pairId, lease.epoch, now],
      );
      return dispatch.rowCount === 1;
    });
  }

  async planDispatch(lease: PairLease, now: Date): Promise<boolean> {
    return this.#transaction(async (client) => {
      const pair = await client.query<PairRow>(
        `UPDATE pr_pair
            SET updated_at = $5
          WHERE id = $1 AND epoch = $2 AND fence = $3 AND lease_owner = $4
            AND lease_expires_at > $5 AND active AND NOT quarantined
          RETURNING *`,
        [lease.pairId, lease.epoch, lease.fence, lease.workerId, now],
      );
      if (pair.rowCount === 0) {
        return false;
      }
      await client.query(
        `INSERT INTO dispatch (
           id, pair_id, pair_epoch, provider, command, state, created_at, updated_at
         ) VALUES ($1, $2, $3, 'coderabbit', '@coderabbitai full review', 'planned', $4, $4)
         ON CONFLICT (pair_id, pair_epoch, provider) DO NOTHING`,
        [randomUUID(), lease.pairId, lease.epoch, now],
      );
      return true;
    });
  }

  async adoptDispatch(
    lease: PairLease,
    commentId: number,
    dispatchedAt: Date,
    evidenceTimeoutSeconds: number,
    now: Date,
  ): Promise<boolean> {
    return this.#transaction(async (client) => {
      const pair = await client.query<PairRow>(
        `SELECT * FROM pr_pair
          WHERE id = $1 AND epoch = $2 AND active AND NOT quarantined
          FOR UPDATE`,
        [lease.pairId, lease.epoch],
      );
      if (pair.rowCount === 0) {
        return false;
      }
      const deadline = new Date(dispatchedAt.valueOf() + evidenceTimeoutSeconds * 1_000);
      const dispatch = await client.query(
        `UPDATE dispatch
            SET command_comment_id = $3, state = 'sent', dispatched_at = $4,
                evidence_deadline_at = $5, updated_at = $6
          WHERE pair_id = $1 AND pair_epoch = $2
            AND provider = 'coderabbit' AND state = 'attempting'
         RETURNING id`,
        [lease.pairId, lease.epoch, commentId, dispatchedAt, deadline, now],
      );
      if (dispatch.rowCount !== 1) {
        return false;
      }
      await client.query(
        `UPDATE pr_pair
            SET lease_owner = NULL, lease_expires_at = NULL, updated_at = $5
          WHERE id = $1 AND epoch = $2 AND fence = $3 AND lease_owner = $4`,
        [lease.pairId, lease.epoch, lease.fence, lease.workerId, now],
      );
      const gateResult = await client.query<GateRow>(
        `SELECT * FROM gate_subject
          WHERE pair_id = $1 AND epoch = $2 AND active`,
        [lease.pairId, lease.epoch],
      );
      const gate = gateResult.rows[0];
      if (gate === undefined) {
        throw new Error("active gate subject is missing while adopting a dispatch");
      }
      await client.query(
        `INSERT INTO outbox (
           topic, aggregate_id, aggregate_epoch, payload, dedupe_key,
           available_at, created_at
         ) VALUES ('evaluate_pair', $1, $2, $3::jsonb, $4, $5, $6)
         ON CONFLICT (dedupe_key) DO NOTHING`,
        [
          lease.pairId,
          lease.epoch,
          JSON.stringify({
            pairId: lease.pairId,
            epoch: lease.epoch,
            gateRevision: int(gate.revision),
          }),
          `evaluate-timeout:${lease.pairId}:${lease.epoch}`,
          deadline,
          now,
        ],
      );
      return true;
    });
  }

  async loadPairSnapshot(pairId: string, expectedEpoch: number): Promise<PairSnapshot | null> {
    const client = await this.#pool.connect();
    try {
      const pairResult = await client.query<PairRow>(
        `SELECT * FROM pr_pair WHERE id = $1 AND epoch = $2`,
        [pairId, expectedEpoch],
      );
      const pair = pairResult.rows[0];
      if (pair === undefined) {
        return null;
      }
      const dispatchResult = await client.query<DispatchRow>(
        `SELECT id, provider, state, command, command_comment_id, dispatched_at,
                evidence_deadline_at, created_at
           FROM dispatch
          WHERE pair_id = $1 AND pair_epoch = $2 AND provider = 'coderabbit'`,
        [pairId, expectedEpoch],
      );
      const evidenceResult = await client.query<EvidenceRow>(
        `SELECT pair_id, pair_epoch, provider, kind, source_id,
                source_updated_at, valid, facts
           FROM evidence
          WHERE pair_id = $1 AND pair_epoch = $2
          ORDER BY source_updated_at, source_id`,
        [pairId, expectedEpoch],
      );
      const dispatch = dispatchResult.rows[0];
      return {
        id: pair.id,
        installationId: int(pair.installation_id),
        repositoryId: int(pair.repository_id),
        repositoryFullName: pair.repository_full_name,
        pullNumber: pair.pull_number,
        baseSha: pair.base_sha,
        headSha: pair.head_sha,
        epoch: int(pair.epoch),
        active: pair.active,
        quarantined: pair.quarantined,
        quarantineDeliveryId: pair.quarantine_delivery_id,
        draft: pair.draft,
        state: pair.pull_state,
        dispatch:
          dispatch === undefined
            ? null
            : {
                id: dispatch.id,
                provider: dispatch.provider,
                state: dispatch.state,
                command: dispatch.command,
                commandCommentId:
                  dispatch.command_comment_id === null ? null : int(dispatch.command_comment_id),
                dispatchedAt:
                  dispatch.dispatched_at === null ? null : new Date(dispatch.dispatched_at),
                evidenceDeadlineAt:
                  dispatch.evidence_deadline_at === null
                    ? null
                    : new Date(dispatch.evidence_deadline_at),
                plannedAt: new Date(dispatch.created_at),
              },
        evidence: evidenceResult.rows.map(
          (row): EvidenceRecord => ({
            pairId: row.pair_id,
            pairEpoch: int(row.pair_epoch),
            provider: row.provider,
            kind: row.kind,
            sourceId: row.source_id,
            sourceUpdatedAt: new Date(row.source_updated_at),
            valid: row.valid,
            facts: row.facts,
          }),
        ),
      };
    } finally {
      client.release();
    }
  }

  async getGateSubject(subjectId: string): Promise<GateSubject | null> {
    const client = await this.#pool.connect();
    try {
      const result = await client.query<GateRow>("SELECT * FROM gate_subject WHERE id = $1", [
        subjectId,
      ]);
      const row = result.rows[0];
      return row === undefined ? null : gateFromRow(row);
    } finally {
      client.release();
    }
  }

  async findActiveGate(
    repositoryId: number,
    subjectType: "pull_request" | "merge_group",
    subjectKey: string,
  ): Promise<GateSubject | null> {
    const client = await this.#pool.connect();
    try {
      const result = await client.query<GateRow>(
        `SELECT * FROM gate_subject
          WHERE repository_id = $1 AND subject_type = $2 AND subject_key = $3 AND active`,
        [repositoryId, subjectType, subjectKey],
      );
      const row = result.rows[0];
      return row === undefined ? null : gateFromRow(row);
    } finally {
      client.release();
    }
  }

  async acquireGateLease(
    subjectId: string,
    expectedEpoch: number,
    expectedRevision: number,
    workerId: string,
    now: Date,
    leaseSeconds: number,
  ): Promise<GateLease | null> {
    const expiresAt = new Date(now.valueOf() + leaseSeconds * 1_000);
    const result = await this.#transaction((client) =>
      client.query<LeaseRow>(
        `UPDATE gate_subject
            SET fence = fence + 1, lease_owner = $4, lease_expires_at = $5,
                updated_at = $6
          WHERE id = $1 AND epoch = $2 AND revision = $3
            AND (lease_owner IS NULL OR lease_expires_at <= $6 OR lease_owner = $4)
          RETURNING id, epoch, revision, fence, lease_expires_at`,
        [subjectId, expectedEpoch, expectedRevision, workerId, expiresAt, now],
      ),
    );
    const row = result.rows[0];
    return row === undefined
      ? null
      : {
          subjectId: row.id,
          epoch: int(row.epoch),
          revision: int(row.revision ?? 0),
          fence: int(row.fence),
          workerId,
          expiresAt: new Date(row.lease_expires_at),
      };
  }

  async authorizeSuccessPublication(
    lease: GateLease,
    observedCheckRunId: number | null,
    now: Date,
  ): Promise<"authorized" | "expired" | "stale"> {
    return this.#transaction(async (client) => {
      const result = await client.query<GateRow>(
        `SELECT * FROM gate_subject
          WHERE id = $1 AND epoch = $2 AND revision = $3 AND fence = $4
            AND lease_owner = $5 AND lease_expires_at > $6
          FOR UPDATE`,
        [
          lease.subjectId,
          lease.epoch,
          lease.revision,
          lease.fence,
          lease.workerId,
          now,
        ],
      );
      const row = result.rows[0];
      if (row === undefined || row.desired_state !== "success" || row.quarantined) {
        return "stale";
      }
      if (row.pair_id === null) {
        throw new Error("pull-request success gate is missing its pair binding");
      }
      const pair = await client.query(
        `SELECT id FROM pr_pair
          WHERE id = $1 AND epoch = $2 AND active AND NOT quarantined`,
        [row.pair_id, lease.epoch],
      );
      if (pair.rowCount !== 1) {
        return "stale";
      }
      const deadline = await client.query<DeadlineRow>(
        `SELECT evidence_deadline_at > statement_timestamp() AS deadline_valid
           FROM dispatch
          WHERE pair_id = $1 AND pair_epoch = $2
            AND provider = 'coderabbit' AND state = 'sent'`,
        [row.pair_id, lease.epoch],
      );
      if (deadline.rows[0]?.deadline_valid === true) {
        return "authorized";
      }
      const expired = await client.query<GateRow>(
        `UPDATE gate_subject
            SET desired_state = 'failure',
                desired_summary = 'The evidence deadline passed before success publication.',
                revision = revision + 1, fence = fence + 1,
                check_run_id = COALESCE($6, check_run_id),
                lease_owner = NULL, lease_expires_at = NULL, updated_at = $7
          WHERE id = $1 AND epoch = $2 AND revision = $3 AND fence = $4
            AND lease_owner = $5
          RETURNING *`,
        [
          lease.subjectId,
          lease.epoch,
          lease.revision,
          lease.fence,
          lease.workerId,
          observedCheckRunId,
          now,
        ],
      );
      const expiredRow = expired.rows[0];
      if (expiredRow === undefined) {
        return "stale";
      }
      await this.#enqueueGatePublication(client, gateFromRow(expiredRow), now);
      return "expired";
    });
  }

  async commitGatePublication(
    lease: GateLease,
    checkRunId: number,
    publishedState: GateState,
    now: Date,
  ): Promise<boolean> {
    const result = await this.#transaction((client) =>
      client.query(
        `UPDATE gate_subject
            SET check_run_id = $6, published_revision = revision,
                published_state = $7, lease_owner = NULL,
                lease_expires_at = NULL, updated_at = $8
          WHERE id = $1 AND epoch = $2 AND revision = $3 AND fence = $4
            AND lease_owner = $5 AND lease_expires_at > $8`,
        [
          lease.subjectId,
          lease.epoch,
          lease.revision,
          lease.fence,
          lease.workerId,
          checkRunId,
          publishedState,
          now,
        ],
      ),
    );
    return result.rowCount === 1;
  }

  async rejectGatePublication(
    lease: GateLease,
    summary: string,
    observedCheckRunId: number | null,
    now: Date,
  ): Promise<boolean> {
    return this.#transaction(async (client) => {
      const result = await client.query<GateRow>(
        `UPDATE gate_subject
            SET desired_state = 'failure', desired_summary = $6,
                revision = revision + 1, fence = fence + 1,
                check_run_id = COALESCE($7, check_run_id),
                lease_owner = NULL, lease_expires_at = NULL, updated_at = $8
          WHERE id = $1 AND epoch = $2 AND revision = $3 AND fence = $4
            AND lease_owner = $5 AND lease_expires_at > $8
          RETURNING *`,
        [
          lease.subjectId,
          lease.epoch,
          lease.revision,
          lease.fence,
          lease.workerId,
          summary,
          observedCheckRunId,
          now,
        ],
      );
      const row = result.rows[0];
      if (row === undefined) {
        return false;
      }
      await this.#enqueueGatePublication(client, gateFromRow(row), now);
      return true;
    });
  }

  async supersedeStaleGatePublication(
    subjectId: string,
    expectedEpoch: number,
    staleRevision: number,
    observedCheckRunId: number,
    now: Date,
  ): Promise<boolean> {
    return this.#transaction(async (client) => {
      const result = await client.query<GateRow>(
        `UPDATE gate_subject
            SET revision = revision + 1, fence = fence + 1,
                check_run_id = COALESCE(check_run_id, $4),
                lease_owner = NULL, lease_expires_at = NULL,
                updated_at = $5
          WHERE id = $1 AND epoch = $2 AND revision >= $3
          RETURNING *`,
        [
          subjectId,
          expectedEpoch,
          staleRevision,
          observedCheckRunId,
          now,
        ],
      );
      const row = result.rows[0];
      if (row === undefined) {
        return false;
      }
      await this.#enqueueGatePublication(client, gateFromRow(row), now);
      return true;
    });
  }

  async claimOutbox(
    workerId: string,
    now: Date,
    leaseSeconds: number,
  ): Promise<OutboxLease | null> {
    const expiresAt = new Date(now.valueOf() + leaseSeconds * 1_000);
    return this.#transaction(async (client) => {
      const candidate = await client.query<{ id: string } & QueryResultRow>(
        `SELECT id FROM outbox
          WHERE completed_at IS NULL AND available_at <= $1
            AND (lease_owner IS NULL OR lease_expires_at <= $1)
          ORDER BY available_at, id
          LIMIT 1
          FOR UPDATE SKIP LOCKED`,
        [now],
      );
      const candidateId = candidate.rows[0]?.id;
      if (candidateId === undefined) {
        return null;
      }
      const result = await client.query<OutboxRow>(
        `UPDATE outbox
            SET lease_owner = $2, lease_expires_at = $3,
                lease_fence = lease_fence + 1, attempts = attempts + 1
          WHERE id = $1 AND completed_at IS NULL
          RETURNING id, topic, aggregate_id, aggregate_epoch, payload, lease_fence`,
        [candidateId, workerId, expiresAt],
      );
      const row = result.rows[0];
      return row === undefined
        ? null
        : {
            id: int(row.id),
            topic: row.topic,
            aggregateId: row.aggregate_id,
            aggregateEpoch:
              row.aggregate_epoch === null ? null : int(row.aggregate_epoch),
            payload: row.payload,
            fence: int(row.lease_fence),
            workerId,
          };
    });
  }

  async completeOutbox(lease: OutboxLease, now: Date): Promise<boolean> {
    const result = await this.#transaction((client) =>
      client.query(
        `UPDATE outbox
            SET completed_at = $4, lease_owner = NULL, lease_expires_at = NULL
          WHERE id = $1 AND lease_fence = $2 AND lease_owner = $3
            AND lease_expires_at > $4 AND completed_at IS NULL`,
        [lease.id, lease.fence, lease.workerId, now],
      ),
    );
    return result.rowCount === 1;
  }

  async failOutbox(
    lease: OutboxLease,
    message: string,
    now: Date,
    retrySeconds: number,
  ): Promise<boolean> {
    const availableAt = new Date(now.valueOf() + retrySeconds * 1_000);
    const result = await this.#transaction((client) =>
      client.query(
        `UPDATE outbox
            SET last_error = $4, available_at = $5,
                lease_owner = NULL, lease_expires_at = NULL
          WHERE id = $1 AND lease_fence = $2 AND lease_owner = $3
            AND lease_expires_at > $6 AND completed_at IS NULL`,
        [
          lease.id,
          lease.fence,
          lease.workerId,
          message.slice(0, 2_000),
          availableAt,
          now,
        ],
      ),
    );
    return result.rowCount === 1;
  }

  async beginRepositorySnapshot(
    repository: RepositoryRef,
    now: Date,
  ): Promise<RepositorySnapshotToken> {
    const generation = await this.#transaction(async (client) => {
      await this.#lockGlobalIngest(client);
      return this.#advanceRepositoryGeneration(client, repository.repositoryId, now);
    });
    return {
      repository: { ...repository },
      generation,
    };
  }

  async reconcileOpenPullRequests(
    token: RepositorySnapshotToken,
    pulls: LivePullRequest[],
    deliveryId: string,
    now: Date,
  ): Promise<RepositoryReconcileResult> {
    return this.#transaction(async (client) => {
      await this.#lockGlobalIngest(client);
      const claimed = await client.query<GenerationRow>(
        `UPDATE repository_reconcile_generation
            SET generation = generation + 1, updated_at = $3
          WHERE repository_id = $1 AND generation = $2
          RETURNING generation`,
        [token.repository.repositoryId, token.generation, now],
      );
      if (claimed.rowCount !== 1) {
        return "stale";
      }
      const repository = token.repository;
      const openNumbers = new Set(pulls.map((pull) => pull.number));
      const observed = await client.query<{ pull_number: number } & QueryResultRow>(
        `SELECT pull_number FROM pr_pair
          WHERE repository_id = $1 AND active
          ORDER BY pull_number`,
        [repository.repositoryId],
      );
      const lockedNumbers = new Set([
        ...openNumbers,
        ...observed.rows.map((row) => row.pull_number),
      ]);
      for (const pullNumber of [...lockedNumbers].sort((left, right) => left - right)) {
        await client.query("SELECT pg_advisory_xact_lock($1::bigint)", [
          advisoryKey(repository.repositoryId, "pull_request", String(pullNumber)),
        ]);
      }
      const current = await client.query<PairRow>(
        `SELECT * FROM pr_pair
          WHERE repository_id = $1 AND active
          ORDER BY pull_number
          FOR UPDATE`,
        [repository.repositoryId],
      );
      if (current.rows.some((pair) => !lockedNumbers.has(pair.pull_number))) {
        throw new Error("pull-request set changed while reconciliation locks were acquired");
      }
      for (const pair of current.rows) {
        if (!openNumbers.has(pair.pull_number)) {
          await client.query(
            `UPDATE pr_pair
                SET active = false, pull_state = 'closed', fence = fence + 1,
                    lease_owner = NULL, lease_expires_at = NULL, updated_at = $2
              WHERE id = $1`,
            [pair.id, now],
          );
          const request: IngestRequest = {
            deliveryId,
            eventName: "reconcile",
            payloadSha256: "",
            receivedAt: now,
            impact: {
              kind: "ignored",
              eventName: "reconcile",
              repository,
              reason: "internal",
            },
          };
          await this.#revokeCurrentPairGate(
            client,
            pair.id,
            request,
            "Reconciliation found that this pull request is no longer open.",
            false,
            "failure",
          );
        }
      }
      for (const pull of pulls) {
        const request: IngestRequest = {
          deliveryId,
          eventName: "reconcile",
          payloadSha256: "",
          receivedAt: now,
          impact: {
            kind: "ignored",
            eventName: "reconcile",
            repository,
            reason: "internal",
          },
        };
        const result = await this.#applyPullRequest(client, repository, pull, request, false);
        if (result.disposition === "stale") {
          const pairResult = await client.query<PairRow>(
            `SELECT * FROM pr_pair
              WHERE repository_id = $1 AND pull_number = $2 AND active`,
            [repository.repositoryId, pull.number],
          );
          const pair = pairResult.rows[0];
          if (
            pair !== undefined &&
            !pair.quarantined &&
            pair.base_sha === pull.baseSha &&
            pair.head_sha === pull.headSha
          ) {
            const gateResult = await client.query<GateRow>(
              `SELECT * FROM gate_subject WHERE pair_id = $1 AND active`,
              [pair.id],
            );
            const gateRow = gateResult.rows[0];
            if (gateRow === undefined) {
              throw new Error(`active gate subject is missing for reconciled pair ${pair.id}`);
            }
            await this.#enqueueEvaluation(
              client,
              pair.id,
              int(pair.epoch),
              int(gateRow.revision),
              now,
            );
          }
        }
      }
      return "applied";
    });
  }
}

export function asDatabasePool(pool: Pool): DatabasePool {
  return pool as unknown as DatabasePool;
}

export type PgClient = PoolClient;

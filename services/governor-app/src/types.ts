export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };

export interface RepositoryRef {
  installationId: number;
  repositoryId: number;
  fullName: string;
  owner: string;
  name: string;
}

export interface PullPairRef {
  number: number;
  baseSha: string;
  headSha: string;
  state: "open" | "closed";
  draft: boolean;
  sourceUpdatedAt: Date;
}

export interface EvidenceInput {
  provider: "coderabbit";
  kind: "issue_comment" | "review" | "status";
  sourceId: string;
  sourceUpdatedAt: Date;
  valid: boolean;
  facts: JsonObject;
}

interface EventBase {
  repository: RepositoryRef;
  eventName: string;
}

export interface PullRequestImpact extends EventBase {
  kind: "pull_request";
  action: string;
  pair: PullPairRef;
}

export interface PullEvidenceImpact extends EventBase {
  kind: "pull_evidence";
  pullNumber: number;
  expectedHeadSha: string | null;
  evidence: EvidenceInput;
}

export interface ShaEvidenceImpact extends EventBase {
  kind: "sha_evidence";
  headSha: string;
  evidence: EvidenceInput;
}

export interface RepositoryReconcileImpact extends EventBase {
  kind: "repository_reconcile";
  reason: "push" | "malformed_signed_event";
}

export interface MergeGroupImpact extends EventBase {
  kind: "merge_group";
  action: "checks_requested" | "destroyed";
  headSha: string;
  baseSha: string;
  headRef: string;
}

export interface IgnoredImpact extends EventBase {
  kind: "ignored";
  reason: string;
}

export type GithubImpact =
  | PullRequestImpact
  | PullEvidenceImpact
  | ShaEvidenceImpact
  | RepositoryReconcileImpact
  | MergeGroupImpact
  | IgnoredImpact;

export interface IngestRequest {
  deliveryId: string;
  eventName: string;
  payloadSha256: string;
  receivedAt: Date;
  impact: GithubImpact;
}

export interface UnscopedQuarantineRequest {
  deliveryId: string;
  eventName: string;
  payloadSha256: string;
  receivedAt: Date;
  errorCode: "invalid_json" | "invalid_repository_envelope";
}

export type IngestDisposition = "applied" | "ignored" | "stale";

export interface IngestResult {
  duplicate: boolean;
  disposition: IngestDisposition;
  touchedSubjects: string[];
}

export interface PairLease {
  pairId: string;
  epoch: number;
  fence: number;
  workerId: string;
  expiresAt: Date;
}

export interface GateLease {
  subjectId: string;
  epoch: number;
  revision: number;
  fence: number;
  workerId: string;
  expiresAt: Date;
}

export type GateState = "revoked" | "pending" | "success" | "failure" | "cancelled";

export interface GateSubject {
  id: string;
  installationId: number;
  repositoryId: number;
  repositoryFullName: string;
  subjectType: "pull_request" | "merge_group";
  subjectKey: string;
  pairId: string | null;
  epoch: number;
  revision: number;
  headSha: string;
  baseSha: string;
  active: boolean;
  quarantined: boolean;
  quarantineDeliveryId: string | null;
  desiredState: GateState;
  desiredSummary: string;
  checkRunId: number | null;
  publishedRevision: number | null;
  publishedState: GateState | null;
}

export interface DispatchRecord {
  id: string;
  provider: string;
  state: "planned" | "attempting" | "sent" | "abandoned";
  command: string;
  commandCommentId: number | null;
  dispatchedAt: Date | null;
  evidenceDeadlineAt: Date | null;
  plannedAt: Date;
}

export interface EvidenceRecord extends EvidenceInput {
  pairId: string;
  pairEpoch: number;
}

export interface PairSnapshot {
  id: string;
  installationId: number;
  repositoryId: number;
  repositoryFullName: string;
  pullNumber: number;
  baseSha: string;
  headSha: string;
  epoch: number;
  active: boolean;
  quarantined: boolean;
  quarantineDeliveryId: string | null;
  draft: boolean;
  state: "open" | "closed";
  dispatch: DispatchRecord | null;
  evidence: EvidenceRecord[];
}

export type OutboxTopic =
  | "publish_gate"
  | "evaluate_pair"
  | "dispatch_review"
  | "reconcile_repository";

export interface OutboxLease {
  id: number;
  topic: OutboxTopic;
  aggregateId: string | null;
  aggregateEpoch: number | null;
  payload: JsonObject;
  fence: number;
  workerId: string;
}

export interface LivePullRequest extends PullPairRef {
  merged: boolean;
}

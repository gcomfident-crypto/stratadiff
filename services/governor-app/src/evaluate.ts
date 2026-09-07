import { coderabbitEvidenceConstants } from "./events.js";
import type { EvidenceRecord, GateState, JsonValue, PairSnapshot } from "./types.js";

const MAX_EVIDENCE_SKEW_MS = 5 * 60 * 1_000;

export interface PairDecision {
  state: GateState;
  summary: string;
  needsDispatch: boolean;
}

function fact(record: EvidenceRecord, name: string): JsonValue {
  const value = record.facts[name];
  if (value === undefined) {
    throw new Error(`evidence ${record.kind}/${record.sourceId} is missing ${name}`);
  }
  return value;
}

function authenticated(record: EvidenceRecord): boolean {
  return (
    fact(record, "authorId") === coderabbitEvidenceConstants.botId &&
    fact(record, "authorLogin") === coderabbitEvidenceConstants.botLogin &&
    fact(record, "authorType") === "Bot" &&
    fact(record, "authorHtmlUrl") === coderabbitEvidenceConstants.appUrl
  );
}

function latest(records: EvidenceRecord[], kind: EvidenceRecord["kind"]): EvidenceRecord | null {
  const matching = records.filter((record) => record.kind === kind && authenticated(record));
  matching.sort((left, right) => {
    const time = right.sourceUpdatedAt.valueOf() - left.sourceUpdatedAt.valueOf();
    if (time !== 0) {
      return time;
    }
    const leftId = BigInt(left.sourceId);
    const rightId = BigInt(right.sourceId);
    return rightId > leftId ? 1 : rightId < leftId ? -1 : 0;
  });
  return matching[0] ?? null;
}

function factDate(record: EvidenceRecord, name: string): Date {
  const value = fact(record, name);
  if (typeof value !== "string") {
    throw new Error(`evidence ${record.kind}/${record.sourceId} has non-string ${name}`);
  }
  const parsed = new Date(value);
  if (!Number.isFinite(parsed.valueOf())) {
    throw new Error(`evidence ${record.kind}/${record.sourceId} has invalid ${name}`);
  }
  return parsed;
}

function coderabbitAvatar(value: JsonValue): boolean {
  if (typeof value !== "string") {
    return false;
  }
  if (!URL.canParse(value)) {
    return false;
  }
  const url = new URL(value);
  return (
    url.protocol === "https:" &&
    url.hostname === "avatars.githubusercontent.com" &&
    url.pathname === `/in/${coderabbitEvidenceConstants.appId}` &&
    url.username === "" &&
    url.password === ""
  );
}

export function evaluatePair(
  snapshot: PairSnapshot,
  now: Date,
): PairDecision {
  if (snapshot.quarantined) {
    return {
      state: "failure",
      summary: "This review epoch is quarantined after an unscoped authenticated webhook.",
      needsDispatch: false,
    };
  }
  if (!snapshot.active || snapshot.state === "closed") {
    return {
      state: "failure",
      summary: "The pull request became terminal without current final-head evidence.",
      needsDispatch: false,
    };
  }
  if (snapshot.draft) {
    return {
      state: "pending",
      summary: "Draft pull requests remain fail-closed until they become ready for review.",
      needsDispatch: false,
    };
  }
  if (snapshot.dispatch?.state === "attempting") {
    return {
      state: "pending",
      summary: "The command POST outcome is uncertain; waiting for durable comment recovery.",
      needsDispatch: false,
    };
  }
  if (snapshot.dispatch?.state === "abandoned") {
    return {
      state: "failure",
      summary: "This review epoch was quarantined after malformed signed provider evidence.",
      needsDispatch: false,
    };
  }
  if (
    snapshot.dispatch?.state !== "sent" ||
    snapshot.dispatch.dispatchedAt === null ||
    snapshot.dispatch.evidenceDeadlineAt === null
  ) {
    return {
      state: "pending",
      summary: "The dedicated App has not dispatched the final-head review command yet.",
      needsDispatch: true,
    };
  }

  const dispatchTime = snapshot.dispatch.dispatchedAt;
  const timedOut = now.valueOf() >= snapshot.dispatch.evidenceDeadlineAt.valueOf();
  if (timedOut) {
    return {
      state: "failure",
      summary: "The evidence deadline passed; later evidence cannot reopen this epoch.",
      needsDispatch: false,
    };
  }
  const status = latest(snapshot.evidence, "status");
  const review = latest(snapshot.evidence, "review");
  const acknowledgement = latest(snapshot.evidence, "issue_comment");
  if (status === null || review === null || acknowledgement === null) {
    return {
      state: "pending",
      summary: "Waiting for post-dispatch status, exact-head review, and command acknowledgement.",
      needsDispatch: false,
    };
  }
  if (!status.valid || !review.valid || !acknowledgement.valid) {
    return {
      state: "failure",
      summary: "The newest authenticated provider evidence was revoked or dismissed.",
      needsDispatch: false,
    };
  }

  const statusTime = factDate(status, "createdAt");
  const reviewTime = factDate(review, "submittedAt");
  const acknowledgementCreated = factDate(acknowledgement, "createdAt");
  const acknowledgementUpdated = factDate(acknowledgement, "updatedAt");
  const allPostDispatch =
    statusTime > dispatchTime &&
    reviewTime > dispatchTime &&
    acknowledgementCreated > dispatchTime;
  if (!allPostDispatch) {
    return {
      state: "pending",
      summary: "Only evidence strictly newer than this App dispatch can satisfy the gate.",
      needsDispatch: false,
    };
  }

  const reviewState = fact(review, "state");
  if (reviewState === "CHANGES_REQUESTED") {
    return {
      state: "failure",
      summary: "The newest exact-head CodeRabbit review requested changes.",
      needsDispatch: false,
    };
  }
  const statusState = fact(status, "state");
  if (statusState === "failure" || statusState === "error") {
    return {
      state: "failure",
      summary: "The newest CodeRabbit status is negative.",
      needsDispatch: false,
    };
  }
  const statusComplete =
    statusState === "success" &&
    fact(status, "description") === "Review completed" &&
    fact(status, "context") === "CodeRabbit" &&
    coderabbitAvatar(fact(status, "avatarUrl"));
  const reviewComplete =
    fact(review, "commitId") === snapshot.headSha &&
    fact(review, "hasReviewMarker") === true &&
    new Set(["APPROVED", "COMMENTED"]).has(String(reviewState));
  const acknowledgementComplete =
    fact(acknowledgement, "fullReviewFinished") === true &&
    typeof fact(acknowledgement, "commandInvocationId") === "string";
  const completionTime = Math.max(statusTime.valueOf(), reviewTime.valueOf());
  const correlated =
    Math.abs(statusTime.valueOf() - reviewTime.valueOf()) <= MAX_EVIDENCE_SKEW_MS &&
    acknowledgementCreated.valueOf() <= completionTime &&
    acknowledgementUpdated.valueOf() >= completionTime &&
    acknowledgementUpdated.valueOf() - completionTime <= MAX_EVIDENCE_SKEW_MS;
  if (
    statusComplete &&
    reviewComplete &&
    acknowledgementComplete &&
    correlated
  ) {
    return {
      state: "success",
      summary: `Dedicated-App evidence covers base ${snapshot.baseSha} and head ${snapshot.headSha}.`,
      needsDispatch: false,
    };
  }
  return {
    state: "pending",
    summary: "The newest provider evidence is not complete and correlated yet.",
    needsDispatch: false,
  };
}

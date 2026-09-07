import { createHash } from "node:crypto";

import type {
  EvidenceInput,
  GithubImpact,
  JsonObject,
  PullPairRef,
  RepositoryRef,
} from "./types.js";

type UnknownRecord = Record<string, unknown>;

const OBJECT_ID = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i;
const COMMAND_INVOCATION =
  /<!-- CodeRabbit review command invocation: ([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}) -->/i;
const REVIEW_MARKER = "<!-- This is an auto-generated comment by CodeRabbit for review status -->";

function record(value: unknown, path: string): UnknownRecord {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${path} must be an object`);
  }
  return value as UnknownRecord;
}

function string(value: unknown, path: string): string {
  if (typeof value !== "string") {
    throw new Error(`${path} must be a string`);
  }
  return value;
}

function nullableString(value: unknown, path: string): string | null {
  if (value === null) {
    return null;
  }
  return string(value, path);
}

function integer(value: unknown, path: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 1) {
    throw new Error(`${path} must be a positive safe integer`);
  }
  return value;
}

function boolean(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") {
    throw new Error(`${path} must be a boolean`);
  }
  return value;
}

function date(value: unknown, path: string): Date {
  const text = string(value, path);
  const parsed = new Date(text);
  if (!Number.isFinite(parsed.valueOf())) {
    throw new Error(`${path} must be an ISO timestamp`);
  }
  return parsed;
}

function objectId(value: unknown, path: string): string {
  const id = string(value, path).toLowerCase();
  if (!OBJECT_ID.test(id)) {
    throw new Error(`${path} must be a 40- or 64-character hexadecimal object ID`);
  }
  return id;
}

function repository(payload: UnknownRecord): RepositoryRef {
  const installation = record(payload["installation"], "installation");
  const repositoryValue = record(payload["repository"], "repository");
  const owner = record(repositoryValue["owner"], "repository.owner");
  const fullName = string(repositoryValue["full_name"], "repository.full_name");
  const ownerName = string(owner["login"], "repository.owner.login");
  const name = string(repositoryValue["name"], "repository.name");
  if (fullName !== `${ownerName}/${name}`) {
    throw new Error("repository.full_name does not match repository.owner/login and name");
  }
  return {
    installationId: integer(installation["id"], "installation.id"),
    repositoryId: integer(repositoryValue["id"], "repository.id"),
    fullName,
    owner: ownerName,
    name,
  };
}

function pullPair(pullValue: unknown, fallbackNumber: unknown): PullPairRef {
  const pull = record(pullValue, "pull_request");
  const base = record(pull["base"], "pull_request.base");
  const head = record(pull["head"], "pull_request.head");
  const state = string(pull["state"], "pull_request.state");
  if (state !== "open" && state !== "closed") {
    throw new Error("pull_request.state must be open or closed");
  }
  return {
    number: integer(pull["number"] ?? fallbackNumber, "pull_request.number"),
    baseSha: objectId(base["sha"], "pull_request.base.sha"),
    headSha: objectId(head["sha"], "pull_request.head.sha"),
    state,
    draft: boolean(pull["draft"], "pull_request.draft"),
    sourceUpdatedAt: date(pull["updated_at"], "pull_request.updated_at"),
  };
}

function userFacts(value: unknown, path: string): JsonObject {
  const user = record(value, path);
  return {
    authorId: integer(user["id"], `${path}.id`),
    authorLogin: string(user["login"], `${path}.login`),
    authorType: string(user["type"], `${path}.type`),
    authorHtmlUrl: string(user["html_url"], `${path}.html_url`),
  };
}

function bodyDigest(body: string): string {
  return createHash("sha256").update(body).digest("hex");
}

function reviewEvidence(payload: UnknownRecord, action: string): EvidenceInput {
  const review = record(payload["review"], "review");
  const body = nullableString(review["body"], "review.body");
  const state = string(review["state"], "review.state").toUpperCase();
  const submittedAt = date(review["submitted_at"], "review.submitted_at");
  const commitId = objectId(review["commit_id"], "review.commit_id");
  return {
    provider: "coderabbit",
    kind: "review",
    sourceId: String(integer(review["id"], "review.id")),
    sourceUpdatedAt: submittedAt,
    valid: action === "submitted" && state !== "DISMISSED",
    facts: {
      ...userFacts(review["user"], "review.user"),
      state,
      commitId,
      submittedAt: submittedAt.toISOString(),
      hasReviewMarker: body !== null && body.includes(REVIEW_MARKER),
      bodySha256: body === null ? null : bodyDigest(body),
    },
  };
}

function commentEvidence(payload: UnknownRecord, action: string): EvidenceInput {
  const comment = record(payload["comment"], "comment");
  const body = string(comment["body"], "comment.body");
  const createdAt = date(comment["created_at"], "comment.created_at");
  const updatedAt = date(comment["updated_at"], "comment.updated_at");
  if (updatedAt < createdAt) {
    throw new Error("comment.updated_at predates comment.created_at");
  }
  const invocation = COMMAND_INVOCATION.exec(body);
  return {
    provider: "coderabbit",
    kind: "issue_comment",
    sourceId: String(integer(comment["id"], "comment.id")),
    sourceUpdatedAt: updatedAt,
    valid: action !== "deleted",
    facts: {
      ...userFacts(comment["user"], "comment.user"),
      createdAt: createdAt.toISOString(),
      updatedAt: updatedAt.toISOString(),
      fullReviewFinished: body.includes("Full review finished."),
      commandInvocationId: invocation?.[1]?.toLowerCase() ?? null,
      bodySha256: bodyDigest(body),
    },
  };
}

function statusEvidence(payload: UnknownRecord): EvidenceInput {
  const statusId = integer(payload["id"], "id");
  const createdAt = date(payload["created_at"], "created_at");
  const description = nullableString(payload["description"], "description");
  return {
    provider: "coderabbit",
    kind: "status",
    sourceId: String(statusId),
    sourceUpdatedAt: createdAt,
    valid: true,
    facts: {
      ...userFacts(payload["creator"], "creator"),
      state: string(payload["state"], "state"),
      context: string(payload["context"], "context"),
      description,
      avatarUrl: nullableString(payload["avatar_url"], "avatar_url"),
      createdAt: createdAt.toISOString(),
    },
  };
}

export function normalizeGithubEvent(eventName: string, value: unknown): GithubImpact {
  const payload = record(value, "payload");
  const repositoryRef = repository(payload);
  const actionValue = payload["action"];
  const action = actionValue === undefined ? "" : string(actionValue, "action");

  if (eventName === "pull_request") {
    return {
      kind: "pull_request",
      eventName,
      repository: repositoryRef,
      action,
      pair: pullPair(payload["pull_request"], payload["number"]),
    };
  }

  if (eventName === "pull_request_review") {
    if (!new Set(["submitted", "edited", "dismissed"]).has(action)) {
      throw new Error(`unsupported pull_request_review action ${action}`);
    }
    const pair = pullPair(payload["pull_request"], payload["number"]);
    return {
      kind: "pull_evidence",
      eventName,
      repository: repositoryRef,
      pullNumber: pair.number,
      expectedHeadSha: pair.headSha,
      evidence: reviewEvidence(payload, action),
    };
  }

  if (eventName === "issue_comment") {
    if (!new Set(["created", "edited", "deleted"]).has(action)) {
      throw new Error(`unsupported issue_comment action ${action}`);
    }
    const issue = record(payload["issue"], "issue");
    if (issue["pull_request"] === undefined) {
      return {
        kind: "ignored",
        eventName,
        repository: repositoryRef,
        reason: "ordinary_issue_comment",
      };
    }
    record(issue["pull_request"], "issue.pull_request");
    return {
      kind: "pull_evidence",
      eventName,
      repository: repositoryRef,
      pullNumber: integer(issue["number"], "issue.number"),
      expectedHeadSha: null,
      evidence: commentEvidence(payload, action),
    };
  }

  if (eventName === "status") {
    return {
      kind: "sha_evidence",
      eventName,
      repository: repositoryRef,
      headSha: objectId(payload["sha"], "sha"),
      evidence: statusEvidence(payload),
    };
  }

  if (eventName === "push") {
    return {
      kind: "repository_reconcile",
      eventName,
      repository: repositoryRef,
      reason: "push",
    };
  }

  if (eventName === "merge_group") {
    if (action !== "checks_requested" && action !== "destroyed") {
      throw new Error(`unsupported merge_group action ${action}`);
    }
    const mergeGroup = record(payload["merge_group"], "merge_group");
    return {
      kind: "merge_group",
      eventName,
      repository: repositoryRef,
      action,
      headSha: objectId(mergeGroup["head_sha"], "merge_group.head_sha"),
      baseSha: objectId(mergeGroup["base_sha"], "merge_group.base_sha"),
      headRef: string(mergeGroup["head_ref"], "merge_group.head_ref"),
    };
  }

  return {
    kind: "ignored",
    eventName,
    repository: repositoryRef,
    reason: `unsupported:${eventName}`,
  };
}

export function failClosedImpact(eventName: string, value: unknown): GithubImpact {
  const failClosedEvents = new Set([
    "pull_request",
    "pull_request_review",
    "status",
    "issue_comment",
    "push",
    "merge_group",
  ]);
  if (!failClosedEvents.has(eventName)) {
    throw new Error(`unsupported malformed event ${eventName}`);
  }
  const payload = record(value, "payload");
  return {
    kind: "repository_reconcile",
    eventName,
    repository: repository(payload),
    reason: "malformed_signed_event",
  };
}

export const coderabbitEvidenceConstants = {
  appId: 347564,
  botId: 136622811,
  botLogin: "coderabbitai[bot]",
  appUrl: "https://github.com/apps/coderabbitai",
  reviewMarker: REVIEW_MARKER,
};

import { describe, expect, it } from "vitest";

import { evaluatePair } from "../src/evaluate.js";
import type { EvidenceRecord, JsonObject, PairSnapshot } from "../src/types.js";
import { BASE_A, HEAD_A } from "./helpers.js";

const BOT: JsonObject = {
  authorId: 136622811,
  authorLogin: "coderabbitai[bot]",
  authorType: "Bot",
  authorHtmlUrl: "https://github.com/apps/coderabbitai",
};

function evidence(
  kind: EvidenceRecord["kind"],
  sourceId: string,
  timestamp: string,
  facts: JsonObject,
  valid = true,
): EvidenceRecord {
  return {
    pairId: "2a69c552-ea21-46eb-8383-d8c5bb65dc26",
    pairEpoch: 1,
    provider: "coderabbit",
    kind,
    sourceId,
    sourceUpdatedAt: new Date(timestamp),
    valid,
    facts: { ...BOT, ...facts },
  };
}

function snapshot(
  records: EvidenceRecord[],
  evidenceDeadlineAt = new Date("2026-09-07T12:30:00.000Z"),
): PairSnapshot {
  return {
    id: "2a69c552-ea21-46eb-8383-d8c5bb65dc26",
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
    dispatch: {
      id: "f9f0849c-cb00-4052-b117-a76fd771d921",
      provider: "coderabbit",
      state: "sent",
      command: "@coderabbitai full review",
      commandCommentId: 400,
      dispatchedAt: new Date("2026-09-07T12:00:00.000Z"),
      evidenceDeadlineAt,
      plannedAt: new Date("2026-09-07T11:59:00.000Z"),
    },
    evidence: records,
  };
}

function completeEvidence(): EvidenceRecord[] {
  return [
    evidence("status", "9", "2026-09-07T12:02:00.000Z", {
      state: "success",
      description: "Review completed",
      context: "CodeRabbit",
      avatarUrl: "https://avatars.githubusercontent.com/in/347564?v=4",
      createdAt: "2026-09-07T12:02:00.000Z",
    }),
    evidence("review", "20", "2026-09-07T12:03:00.000Z", {
      state: "COMMENTED",
      commitId: HEAD_A,
      submittedAt: "2026-09-07T12:03:00.000Z",
      hasReviewMarker: true,
    }),
    evidence("issue_comment", "30", "2026-09-07T12:04:00.000Z", {
      createdAt: "2026-09-07T12:01:00.000Z",
      updatedAt: "2026-09-07T12:04:00.000Z",
      fullReviewFinished: true,
      commandInvocationId: "01234567-89ab-cdef-0123-456789abcdef",
    }),
  ];
}

describe("pair evidence decisions", () => {
  it("accepts complete authenticated, post-dispatch, exact-head evidence", () => {
    expect(
      evaluatePair(snapshot(completeEvidence()), new Date("2026-09-07T12:05:00.000Z")),
    ).toMatchObject({ state: "success" });
  });

  it("never reopens a quarantined epoch with otherwise complete evidence", () => {
    const quarantined = snapshot(completeEvidence());
    quarantined.quarantined = true;
    quarantined.quarantineDeliveryId = "global-invalid-json";
    expect(evaluatePair(quarantined, new Date("2026-09-07T12:05:00.000Z"))).toMatchObject({
      state: "failure",
      needsDispatch: false,
    });
  });

  it("keeps an uncertain dispatch fail-closed without requesting another POST", () => {
    const uncertain = snapshot([]);
    uncertain.dispatch = {
      ...uncertain.dispatch!,
      state: "attempting",
      commandCommentId: null,
      dispatchedAt: null,
      evidenceDeadlineAt: null,
    };
    expect(evaluatePair(uncertain, new Date("2026-09-07T12:05:00.000Z"))).toMatchObject({
      state: "pending",
      needsDispatch: false,
    });
  });

  it("lets the numerically newer same-time status revoke an older green status", () => {
    const records = completeEvidence();
    records.push(
      evidence("status", "10", "2026-09-07T12:02:00.000Z", {
        state: "failure",
        description: "Review failed",
        context: "CodeRabbit",
        avatarUrl: "https://avatars.githubusercontent.com/in/347564",
        createdAt: "2026-09-07T12:02:00.000Z",
      }),
    );
    expect(
      evaluatePair(snapshot(records), new Date("2026-09-07T12:05:00.000Z")),
    ).toMatchObject({ state: "failure" });
  });

  it("fails after the configured evidence deadline", () => {
    expect(
      evaluatePair(snapshot([]), new Date("2026-09-07T12:31:00.000Z")),
    ).toMatchObject({ state: "failure" });
  });

  it("does not turn green when complete evidence arrives after the deadline", () => {
    expect(
      evaluatePair(
        snapshot(completeEvidence(), new Date("2026-09-07T12:01:00.000Z")),
        new Date("2026-09-07T12:05:00.000Z"),
      ),
    ).toMatchObject({ state: "failure" });
  });
});

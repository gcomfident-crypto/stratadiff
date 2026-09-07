import { generateKeyPairSync } from "node:crypto";

import { describe, expect, it } from "vitest";

import {
  FINAL_HEAD_CHECK_NAME,
  GithubAppClient,
  gateExternalId,
  paginateOpenPullRequests,
} from "../src/github.js";
import type { GateSubject, LivePullRequest } from "../src/types.js";
import { BASE_A, HEAD_B } from "./helpers.js";

function pull(number: number): LivePullRequest {
  return {
    number,
    baseSha: BASE_A,
    headSha: number.toString(16).padStart(40, "0"),
    state: "open",
    merged: false,
    draft: false,
    sourceUpdatedAt: new Date("2026-09-07T12:00:00.000Z"),
  };
}

describe("GitHub App API", () => {
  it("paginates past the Actions-era 256 item ceiling", async () => {
    const source = Array.from({ length: 301 }, (_, index) => pull(index + 1));
    const pages: number[] = [];
    const result = await paginateOpenPullRequests(async (page, perPage) => {
      pages.push(page);
      return source.slice((page - 1) * perPage, page * perPage);
    });
    expect(result).toHaveLength(301);
    expect(result.at(-1)?.number).toBe(301);
    expect(pages).toEqual([1, 2, 3, 4]);
  });

  it("uses an installation token and creates the fixed check on the merge-group SHA", async () => {
    const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
    const calls: Array<{ url: string; init: RequestInit }> = [];
    const fetchMock = async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input);
      calls.push({ url, init: init ?? {} });
      if (url.endsWith("/app/installations/71/access_tokens")) {
        return new Response(
          JSON.stringify({
            token: "installation-token",
            expires_at: "2026-09-07T14:00:00.000Z",
          }),
          { status: 201, headers: { "content-type": "application/json" } },
        );
      }
      if (url.includes("/commits/") && url.includes("/check-runs?")) {
        return new Response(JSON.stringify({ total_count: 0, check_runs: [] }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response(JSON.stringify({ id: 8123 }), {
        status: 201,
        headers: { "content-type": "application/json" },
      });
    };
    const client = new GithubAppClient({
      appId: "12345",
      privateKey: privateKey.export({ type: "pkcs8", format: "pem" }).toString(),
      apiUrl: "https://api.github.test",
      requestTimeoutMs: 30_000,
      fetch: fetchMock as typeof globalThis.fetch,
      now: () => new Date("2026-09-07T13:00:00.000Z"),
    });
    const subject: GateSubject = {
      id: "cbe18ff5-65fb-4d77-b09f-78f864b1cbf0",
      installationId: 71,
      repositoryId: 99,
      repositoryFullName: "acme/repo",
      subjectType: "merge_group",
      subjectKey: "merge-group:queue-ref",
      pairId: null,
      epoch: 7,
      revision: 1,
      headSha: HEAD_B,
      baseSha: BASE_A,
      active: true,
      quarantined: false,
      quarantineDeliveryId: null,
      desiredState: "revoked",
      desiredSummary: "native merge-group evidence pending",
      checkRunId: null,
      publishedRevision: null,
      publishedState: null,
    };

    await expect(client.publishGate(subject)).resolves.toBe(8123);
    expect(calls).toHaveLength(3);
    expect(calls[0]!.url).toBe("https://api.github.test/app/installations/71/access_tokens");
    expect((calls[0]!.init.headers as Record<string, string>)["authorization"]).toMatch(
      /^Bearer [^.]+\.[^.]+\.[^.]+$/,
    );
    expect(calls[2]!.url).toBe("https://api.github.test/repos/acme/repo/check-runs");
    expect((calls[2]!.init.headers as Record<string, string>)["authorization"]).toBe(
      "Bearer installation-token",
    );
    const body = JSON.parse(String(calls[2]!.init.body)) as Record<string, unknown>;
    expect(body).toMatchObject({
      name: FINAL_HEAD_CHECK_NAME,
      head_sha: HEAD_B,
      external_id: gateExternalId(subject),
      status: "in_progress",
    });
    expect(body).not.toHaveProperty("conclusion");
  });

  it("patches the canonical check run when the desired revision advances", async () => {
    const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
    const calls: Array<{ url: string; init: RequestInit }> = [];
    const fetchMock = async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input);
      calls.push({ url, init: init ?? {} });
      if (url.endsWith("/app/installations/71/access_tokens")) {
        return new Response(
          JSON.stringify({
            token: "installation-token",
            expires_at: "2026-09-07T14:00:00.000Z",
          }),
          { status: 201, headers: { "content-type": "application/json" } },
        );
      }
      return new Response(JSON.stringify({ id: 8123 }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    };
    const client = new GithubAppClient({
      appId: "12345",
      privateKey: privateKey.export({ type: "pkcs8", format: "pem" }).toString(),
      apiUrl: "https://api.github.test",
      requestTimeoutMs: 30_000,
      fetch: fetchMock as typeof globalThis.fetch,
      now: () => new Date("2026-09-07T13:00:00.000Z"),
    });
    const subject: GateSubject = {
      id: "cbe18ff5-65fb-4d77-b09f-78f864b1cbf0",
      installationId: 71,
      repositoryId: 99,
      repositoryFullName: "acme/repo",
      subjectType: "pull_request",
      subjectKey: "pr:1",
      pairId: "2a69c552-ea21-46eb-8383-d8c5bb65dc26",
      epoch: 1,
      revision: 4,
      headSha: HEAD_B,
      baseSha: BASE_A,
      active: true,
      quarantined: false,
      quarantineDeliveryId: null,
      desiredState: "failure",
      desiredSummary: "newer fail-closed decision",
      checkRunId: 8123,
      publishedRevision: 3,
      publishedState: "success",
    };

    await expect(client.publishGate(subject)).resolves.toBe(8123);
    expect(calls).toHaveLength(2);
    expect(calls[1]!.url).toBe("https://api.github.test/repos/acme/repo/check-runs/8123");
    expect(calls[1]!.init.method).toBe("PATCH");
  });
});

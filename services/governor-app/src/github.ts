import { createSign } from "node:crypto";

import type { GateSubject, LivePullRequest, RepositoryRef } from "./types.js";

export const FINAL_HEAD_CHECK_NAME = "StrataDiff Final Head";
const MAX_RESPONSE_BYTES = 8 * 1024 * 1024;
const PAGE_SIZE = 100;

interface TokenCacheEntry {
  token: string;
  expiresAt: Date;
}

interface GithubClientOptions {
  appId: string;
  privateKey: string;
  apiUrl: string;
  requestTimeoutMs: number;
  fetch: typeof globalThis.fetch;
  now: () => Date;
}

interface CommentResponse {
  id: number;
  body: string;
  created_at: string;
  performed_via_github_app: { id: number } | null;
}

interface CheckResponse {
  id: number;
}

function base64url(value: string | Buffer): string {
  return Buffer.from(value).toString("base64url");
}

function validateBaseUrl(value: string, name: string): string {
  const url = new URL(value);
  if (url.protocol !== "https:" || url.username !== "" || url.password !== "") {
    throw new Error(`${name} must be an HTTPS URL without credentials`);
  }
  return value.replace(/\/$/, "");
}

function validateRepository(repository: string): [string, string] {
  const match = /^([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+)$/.exec(repository);
  if (match?.[1] === undefined || match[2] === undefined) {
    throw new Error("repository full name is invalid");
  }
  return [match[1], match[2]];
}

function repositoryPath(repository: string): string {
  const [owner, name] = validateRepository(repository);
  return `${encodeURIComponent(owner)}/${encodeURIComponent(name)}`;
}

function objectId(value: unknown, path: string): string {
  if (typeof value !== "string" || !/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i.test(value)) {
    throw new Error(`${path} is not a Git object ID`);
  }
  return value.toLowerCase();
}

function positiveInteger(value: unknown, path: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 1) {
    throw new Error(`${path} is not a positive safe integer`);
  }
  return value;
}

function boolean(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") {
    throw new Error(`${path} is not a boolean`);
  }
  return value;
}

function string(value: unknown, path: string): string {
  if (typeof value !== "string") {
    throw new Error(`${path} is not a string`);
  }
  return value;
}

function record(value: unknown, path: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${path} is not an object`);
  }
  return value as Record<string, unknown>;
}

function parsePull(value: unknown): LivePullRequest {
  const pull = record(value, "pull response");
  const base = record(pull["base"], "pull.base");
  const head = record(pull["head"], "pull.head");
  const state = string(pull["state"], "pull.state");
  if (state !== "open" && state !== "closed") {
    throw new Error("pull.state is unsupported");
  }
  const updatedAt = new Date(string(pull["updated_at"], "pull.updated_at"));
  if (!Number.isFinite(updatedAt.valueOf())) {
    throw new Error("pull.updated_at is invalid");
  }
  return {
    number: positiveInteger(pull["number"], "pull.number"),
    state,
    merged: boolean(pull["merged"], "pull.merged"),
    draft: boolean(pull["draft"], "pull.draft"),
    sourceUpdatedAt: updatedAt,
    baseSha: objectId(base["sha"], "pull.base.sha"),
    headSha: objectId(head["sha"], "pull.head.sha"),
  };
}

export function gateExternalId(subject: GateSubject): string {
  return `stratadiff-final-head:v1:${subject.subjectType}:${subject.id}:${subject.epoch}:r${subject.revision}`;
}

export async function paginateOpenPullRequests(
  fetchPage: (page: number, perPage: number) => Promise<LivePullRequest[]>,
): Promise<LivePullRequest[]> {
  const pulls: LivePullRequest[] = [];
  for (let page = 1; ; page += 1) {
    const batch = await fetchPage(page, PAGE_SIZE);
    pulls.push(...batch);
    if (batch.length < PAGE_SIZE) {
      return pulls;
    }
  }
}

export interface GovernorGithubClient {
  getPullRequest(
    installationId: number,
    repositoryFullName: string,
    pullNumber: number,
  ): Promise<LivePullRequest>;
  createIssueComment(
    installationId: number,
    repositoryFullName: string,
    pullNumber: number,
    body: string,
  ): Promise<{ id: number; createdAt: Date }>;
  findIssueComment(
    installationId: number,
    repositoryFullName: string,
    pullNumber: number,
    body: string,
    notBefore: Date,
  ): Promise<{ id: number; createdAt: Date } | null>;
  publishGate(subject: GateSubject): Promise<number>;
  listOpenPullRequests(repository: RepositoryRef): Promise<LivePullRequest[]>;
}

export class GithubAppClient implements GovernorGithubClient {
  readonly #appId: string;
  readonly #privateKey: string;
  readonly #apiUrl: string;
  readonly #requestTimeoutMs: number;
  readonly #fetch: typeof globalThis.fetch;
  readonly #now: () => Date;
  readonly #tokens = new Map<number, TokenCacheEntry>();

  constructor(options: GithubClientOptions) {
    if (!/^[1-9][0-9]*$/.test(options.appId)) {
      throw new Error("GitHub App ID must be a positive integer");
    }
    if (!options.privateKey.includes("BEGIN") || !options.privateKey.includes("PRIVATE KEY")) {
      throw new Error("GitHub App private key is not PEM encoded");
    }
    this.#appId = options.appId;
    this.#privateKey = options.privateKey;
    this.#apiUrl = validateBaseUrl(options.apiUrl, "GitHub API URL");
    if (!Number.isSafeInteger(options.requestTimeoutMs) || options.requestTimeoutMs < 1) {
      throw new Error("GitHub request timeout must be a positive integer");
    }
    this.#requestTimeoutMs = options.requestTimeoutMs;
    this.#fetch = options.fetch;
    this.#now = options.now;
  }

  #appJwt(): string {
    const nowSeconds = Math.floor(this.#now().valueOf() / 1_000);
    const header = base64url(JSON.stringify({ alg: "RS256", typ: "JWT" }));
    const payload = base64url(
      JSON.stringify({ iat: nowSeconds - 60, exp: nowSeconds + 540, iss: this.#appId }),
    );
    const unsigned = `${header}.${payload}`;
    const signer = createSign("RSA-SHA256");
    signer.update(unsigned);
    signer.end();
    return `${unsigned}.${signer.sign(this.#privateKey).toString("base64url")}`;
  }

  async #jsonResponse(response: Response, operation: string): Promise<unknown> {
    if (!response.ok) {
      throw new Error(`GitHub API returned ${response.status} for ${operation}`);
    }
    const declaredLength = response.headers.get("content-length");
    if (declaredLength !== null && Number(declaredLength) > MAX_RESPONSE_BYTES) {
      throw new Error(`GitHub API response exceeded ${MAX_RESPONSE_BYTES} bytes`);
    }
    const bytes = Buffer.from(await response.arrayBuffer());
    if (bytes.length > MAX_RESPONSE_BYTES) {
      throw new Error(`GitHub API response exceeded ${MAX_RESPONSE_BYTES} bytes`);
    }
    return JSON.parse(bytes.toString("utf8")) as unknown;
  }

  async #installationToken(installationId: number): Promise<string> {
    const cached = this.#tokens.get(installationId);
    const now = this.#now();
    if (cached !== undefined && cached.expiresAt.valueOf() - now.valueOf() > 60_000) {
      return cached.token;
    }
    const response = await this.#fetch(
      `${this.#apiUrl}/app/installations/${installationId}/access_tokens`,
      {
        method: "POST",
        signal: AbortSignal.timeout(this.#requestTimeoutMs),
        headers: {
          accept: "application/vnd.github+json",
          authorization: `Bearer ${this.#appJwt()}`,
          "user-agent": "stratadiff-governor-app/0.1",
          "x-github-api-version": "2022-11-28",
        },
      },
    );
    const payload = record(await this.#jsonResponse(response, "installation token"), "token");
    const token = string(payload["token"], "token.token");
    const expiresAt = new Date(string(payload["expires_at"], "token.expires_at"));
    if (!Number.isFinite(expiresAt.valueOf()) || expiresAt <= now) {
      throw new Error("GitHub returned an invalid installation token expiry");
    }
    this.#tokens.set(installationId, { token, expiresAt });
    return token;
  }

  async #request(
    installationId: number,
    method: "GET" | "POST" | "PATCH",
    path: string,
    body: Record<string, unknown> | null,
  ): Promise<unknown> {
    const token = await this.#installationToken(installationId);
    const init: RequestInit = {
      method,
      signal: AbortSignal.timeout(this.#requestTimeoutMs),
      headers: {
        accept: "application/vnd.github+json",
        authorization: `Bearer ${token}`,
        "user-agent": "stratadiff-governor-app/0.1",
        "x-github-api-version": "2022-11-28",
      },
    };
    if (body !== null) {
      init.body = JSON.stringify(body);
      (init.headers as Record<string, string>)["content-type"] = "application/json";
    }
    const response = await this.#fetch(`${this.#apiUrl}${path}`, init);
    return this.#jsonResponse(response, `${method} ${path}`);
  }

  async getPullRequest(
    installationId: number,
    repositoryFullName: string,
    pullNumber: number,
  ): Promise<LivePullRequest> {
    const payload = await this.#request(
      installationId,
      "GET",
      `/repos/${repositoryPath(repositoryFullName)}/pulls/${pullNumber}`,
      null,
    );
    return parsePull(payload);
  }

  async createIssueComment(
    installationId: number,
    repositoryFullName: string,
    pullNumber: number,
    body: string,
  ): Promise<{ id: number; createdAt: Date }> {
    const payload = record(
      await this.#request(
        installationId,
        "POST",
        `/repos/${repositoryPath(repositoryFullName)}/issues/${pullNumber}/comments`,
        { body },
      ),
      "comment",
    ) as unknown as CommentResponse;
    const id = positiveInteger(payload.id, "comment.id");
    if (string(payload.body, "comment.body") !== body) {
      throw new Error("GitHub returned a different command comment body");
    }
    const createdAt = new Date(string(payload.created_at, "comment.created_at"));
    if (!Number.isFinite(createdAt.valueOf())) {
      throw new Error("comment.created_at is invalid");
    }
    const performedViaApp = record(
      payload.performed_via_github_app,
      "comment.performed_via_github_app",
    );
    if (positiveInteger(performedViaApp["id"], "comment.performed_via_github_app.id") !== Number(this.#appId)) {
      throw new Error("GitHub did not attribute the command comment to this App");
    }
    return { id, createdAt };
  }

  async findIssueComment(
    installationId: number,
    repositoryFullName: string,
    pullNumber: number,
    body: string,
    notBefore: Date,
  ): Promise<{ id: number; createdAt: Date } | null> {
    const matches: Array<{ id: number; createdAt: Date }> = [];
    for (let page = 1; ; page += 1) {
      const payload = await this.#request(
        installationId,
        "GET",
        `/repos/${repositoryPath(repositoryFullName)}/issues/${pullNumber}/comments?since=${encodeURIComponent(notBefore.toISOString())}&sort=created&direction=asc&per_page=${PAGE_SIZE}&page=${page}`,
        null,
      );
      if (!Array.isArray(payload)) {
        throw new Error("GitHub issue comment response must be an array");
      }
      for (const value of payload) {
        const comment = record(value, "comment");
        const performed = comment["performed_via_github_app"];
        if (performed === null || typeof performed !== "object" || Array.isArray(performed)) {
          continue;
        }
        if (
          (performed as Record<string, unknown>)["id"] !== Number(this.#appId) ||
          comment["body"] !== body
        ) {
          continue;
        }
        const createdAt = new Date(string(comment["created_at"], "comment.created_at"));
        if (!Number.isFinite(createdAt.valueOf()) || createdAt < notBefore) {
          continue;
        }
        matches.push({ id: positiveInteger(comment["id"], "comment.id"), createdAt });
      }
      if (payload.length < PAGE_SIZE) {
        break;
      }
    }
    matches.sort((left, right) => {
      const time = right.createdAt.valueOf() - left.createdAt.valueOf();
      return time === 0 ? right.id - left.id : time;
    });
    return matches[0] ?? null;
  }

  async publishGate(subject: GateSubject): Promise<number> {
    const pending = subject.desiredState === "revoked" || subject.desiredState === "pending";
    const output = {
      title: pending ? "Final-head review pending" : `Final-head review ${subject.desiredState}`,
      summary: subject.desiredSummary,
    };
    const shared: Record<string, unknown> = {
      name: FINAL_HEAD_CHECK_NAME,
      external_id: gateExternalId(subject),
      status: pending ? "in_progress" : "completed",
      output,
    };
    if (!pending) {
      shared["conclusion"] = subject.desiredState;
    }
    const repository = repositoryPath(subject.repositoryFullName);
    const existingCheckRunId =
      subject.checkRunId ?? (await this.#findCheckRun(subject, repository));
    const payload =
      existingCheckRunId === null
        ? await this.#request(
            subject.installationId,
            "POST",
            `/repos/${repository}/check-runs`,
            { ...shared, head_sha: subject.headSha },
          )
        : await this.#request(
            subject.installationId,
            "PATCH",
            `/repos/${repository}/check-runs/${existingCheckRunId}`,
            shared,
          );
    const check = record(payload, "check run") as unknown as CheckResponse;
    return positiveInteger(check.id, "check_run.id");
  }

  async #findCheckRun(subject: GateSubject, repository: string): Promise<number | null> {
    const payload = record(
      await this.#request(
        subject.installationId,
        "GET",
        `/repos/${repository}/commits/${subject.headSha}/check-runs?check_name=${encodeURIComponent(FINAL_HEAD_CHECK_NAME)}&filter=all&per_page=100`,
        null,
      ),
      "check run list",
    );
    const runs = payload["check_runs"];
    if (!Array.isArray(runs)) {
      throw new Error("GitHub check run list is not an array");
    }
    const externalId = gateExternalId(subject);
    const matching = runs
      .map((value) => record(value, "check run"))
      .filter((run) => {
        const app = record(run["app"], "check run.app");
        return run["external_id"] === externalId && app["id"] === Number(this.#appId);
      })
      .map((run) => positiveInteger(run["id"], "check_run.id"))
      .sort((left, right) => right - left);
    return matching[0] ?? null;
  }

  async listOpenPullRequests(repository: RepositoryRef): Promise<LivePullRequest[]> {
    return paginateOpenPullRequests(async (page, perPage) => {
      const payload = await this.#request(
        repository.installationId,
        "GET",
        `/repos/${repositoryPath(repository.fullName)}/pulls?state=open&sort=updated&direction=desc&per_page=${perPage}&page=${page}`,
        null,
      );
      if (!Array.isArray(payload)) {
        throw new Error("GitHub open pull-request response must be an array");
      }
      return payload.map((item) => {
        const pull = record(item, "pull list item");
        return parsePull({ ...pull, merged: false });
      });
    });
  }
}

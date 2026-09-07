export interface AppConfig {
  port: number;
  databaseUrl: string;
  webhookSecret: string;
  githubAppId: string;
  githubPrivateKey: string;
  githubApiUrl: string;
  githubRequestTimeoutMs: number;
  workerId: string;
  workerPollMs: number;
  leaseSeconds: number;
  outboxRetrySeconds: number;
  evidenceTimeoutSeconds: number;
}

function required(environment: NodeJS.ProcessEnv, name: string): string {
  const value = environment[name];
  if (value === undefined || value.length === 0) {
    throw new Error(`${name} is required`);
  }
  return value;
}

function positiveInteger(environment: NodeJS.ProcessEnv, name: string): number {
  const value = required(environment, name);
  if (!/^[1-9][0-9]*$/.test(value)) {
    throw new Error(`${name} must be a positive integer`);
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed)) {
    throw new Error(`${name} exceeds the safe integer range`);
  }
  return parsed;
}

export function loadConfig(environment: NodeJS.ProcessEnv): AppConfig {
  const port = positiveInteger(environment, "PORT");
  if (port > 65_535) {
    throw new Error("PORT must not exceed 65535");
  }
  const githubRequestTimeoutMs = positiveInteger(environment, "GITHUB_REQUEST_TIMEOUT_MS");
  const leaseSeconds = positiveInteger(environment, "LEASE_SECONDS");
  if (leaseSeconds * 1_000 <= githubRequestTimeoutMs) {
    throw new Error("LEASE_SECONDS must exceed GITHUB_REQUEST_TIMEOUT_MS");
  }
  return {
    port,
    databaseUrl: required(environment, "DATABASE_URL"),
    webhookSecret: required(environment, "GITHUB_WEBHOOK_SECRET"),
    githubAppId: required(environment, "GITHUB_APP_ID"),
    githubPrivateKey: required(environment, "GITHUB_PRIVATE_KEY").replaceAll("\\n", "\n"),
    githubApiUrl: required(environment, "GITHUB_API_URL"),
    githubRequestTimeoutMs,
    workerId: required(environment, "WORKER_ID"),
    workerPollMs: positiveInteger(environment, "WORKER_POLL_MS"),
    leaseSeconds,
    outboxRetrySeconds: positiveInteger(environment, "OUTBOX_RETRY_SECONDS"),
    evidenceTimeoutSeconds: positiveInteger(environment, "EVIDENCE_TIMEOUT_SECONDS"),
  };
}

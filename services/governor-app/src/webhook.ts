import { failClosedImpact, normalizeGithubEvent } from "./events.js";
import { verifyWebhookSignature, webhookPayloadSha256 } from "./signature.js";
import type { GovernorStore } from "./store.js";
import type { IngestResult } from "./types.js";

export class WebhookRequestError extends Error {
  readonly statusCode: number;

  constructor(statusCode: number, message: string) {
    super(message);
    this.statusCode = statusCode;
  }
}

export interface WebhookHeaders {
  signature: string | undefined;
  deliveryId: string | undefined;
  eventName: string | undefined;
}

export class WebhookService {
  readonly #secret: string;
  readonly #store: GovernorStore;
  readonly #now: () => Date;

  constructor(secret: string, store: GovernorStore, now: () => Date = () => new Date()) {
    this.#secret = secret;
    this.#store = store;
    this.#now = now;
  }

  async receive(headers: WebhookHeaders, rawBody: Buffer): Promise<IngestResult> {
    if (headers.signature === undefined) {
      throw new WebhookRequestError(401, "X-Hub-Signature-256 is required");
    }
    if (!verifyWebhookSignature(this.#secret, rawBody, headers.signature)) {
      throw new WebhookRequestError(401, "invalid webhook signature");
    }
    if (
      headers.deliveryId === undefined ||
      !/^[A-Za-z0-9_-]{1,200}$/.test(headers.deliveryId)
    ) {
      throw new WebhookRequestError(400, "X-GitHub-Delivery is invalid");
    }
    if (headers.eventName === undefined || !/^[a-z_]{1,100}$/.test(headers.eventName)) {
      throw new WebhookRequestError(400, "X-GitHub-Event is invalid");
    }

    let payload: unknown;
    try {
      payload = JSON.parse(rawBody.toString("utf8")) as unknown;
    } catch {
      throw new WebhookRequestError(400, "webhook body is not valid JSON");
    }
    let impact;
    try {
      impact = normalizeGithubEvent(headers.eventName, payload);
    } catch {
      try {
        impact = failClosedImpact(headers.eventName, payload);
      } catch (error) {
        const message = error instanceof Error ? error.message : "invalid webhook payload";
        throw new WebhookRequestError(400, message);
      }
    }
    return this.#store.ingest({
      deliveryId: headers.deliveryId,
      eventName: headers.eventName,
      payloadSha256: webhookPayloadSha256(rawBody),
      receivedAt: this.#now(),
      impact,
    });
  }
}

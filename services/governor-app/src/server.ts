import { createServer as createHttpServer, type IncomingMessage, type Server } from "node:http";

import { WebhookRequestError, type WebhookService } from "./webhook.js";

const MAX_WEBHOOK_BYTES = 1024 * 1024;

async function readBody(request: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    size += bytes.length;
    if (size > MAX_WEBHOOK_BYTES) {
      throw new WebhookRequestError(413, "webhook body exceeds 1 MiB");
    }
    chunks.push(bytes);
  }
  return Buffer.concat(chunks);
}

function oneHeader(value: string | string[] | undefined): string | undefined {
  if (Array.isArray(value)) {
    return undefined;
  }
  return value;
}

export function createServer(webhookService: WebhookService): Server {
  return createHttpServer(async (request, response) => {
    try {
      const url = new URL(request.url ?? "/", "http://localhost");
      if (request.method === "GET" && url.pathname === "/healthz") {
        response.writeHead(200, { "content-type": "application/json" });
        response.end(JSON.stringify({ ok: true }));
        return;
      }
      if (request.method !== "POST" || url.pathname !== "/webhooks/github") {
        response.writeHead(404, { "content-type": "application/json" });
        response.end(JSON.stringify({ error: "not found" }));
        return;
      }
      const rawBody = await readBody(request);
      const result = await webhookService.receive(
        {
          signature: oneHeader(request.headers["x-hub-signature-256"]),
          deliveryId: oneHeader(request.headers["x-github-delivery"]),
          eventName: oneHeader(request.headers["x-github-event"]),
        },
        rawBody,
      );
      response.writeHead(202, { "content-type": "application/json" });
      response.end(JSON.stringify({ accepted: true, ...result }));
    } catch (error) {
      const statusCode = error instanceof WebhookRequestError ? error.statusCode : 500;
      const message =
        error instanceof WebhookRequestError ? error.message : "internal webhook processing error";
      response.writeHead(statusCode, { "content-type": "application/json" });
      response.end(JSON.stringify({ error: message }));
    }
  });
}

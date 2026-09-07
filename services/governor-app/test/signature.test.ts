import { createHmac } from "node:crypto";

import { describe, expect, it } from "vitest";

import { verifyWebhookSignature } from "../src/signature.js";

describe("webhook HMAC verification", () => {
  const secret = "correct horse battery staple";
  const body = Buffer.from('{"zen":"Keep it logically awesome."}');

  it("accepts the exact SHA-256 HMAC", () => {
    const digest = createHmac("sha256", secret).update(body).digest("hex");
    expect(verifyWebhookSignature(secret, body, `sha256=${digest}`)).toBe(true);
  });

  it("rejects changed bodies, wrong keys, and malformed headers", () => {
    const digest = createHmac("sha256", secret).update(body).digest("hex");
    expect(verifyWebhookSignature(secret, Buffer.from("changed"), `sha256=${digest}`)).toBe(false);
    expect(verifyWebhookSignature("wrong", body, `sha256=${digest}`)).toBe(false);
    expect(verifyWebhookSignature(secret, body, digest)).toBe(false);
    expect(verifyWebhookSignature(secret, body, "sha256=xyz")).toBe(false);
  });
});

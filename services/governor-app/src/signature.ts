import { createHash, createHmac, timingSafeEqual } from "node:crypto";

const SIGNATURE_PREFIX = "sha256=";
const SHA256_HEX_LENGTH = 64;

export function webhookPayloadSha256(rawBody: Buffer): string {
  return createHash("sha256").update(rawBody).digest("hex");
}

export function verifyWebhookSignature(
  secret: string,
  rawBody: Buffer,
  suppliedSignature: string,
): boolean {
  if (!suppliedSignature.startsWith(SIGNATURE_PREFIX)) {
    return false;
  }
  const suppliedHex = suppliedSignature.slice(SIGNATURE_PREFIX.length);
  if (!/^[0-9a-f]{64}$/i.test(suppliedHex) || suppliedHex.length !== SHA256_HEX_LENGTH) {
    return false;
  }
  const expected = createHmac("sha256", secret).update(rawBody).digest();
  const supplied = Buffer.from(suppliedHex, "hex");
  return expected.length === supplied.length && timingSafeEqual(expected, supplied);
}

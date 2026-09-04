import type { LoadedSession, SessionPayload } from '../types'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function getSessionToken(search: string): string {
  const token = new URLSearchParams(search).get('token')
  if (token === null || token.length === 0) {
    throw new Error('Missing viewer session token. Launch this page with `stratadiff view`.')
  }
  return token
}

export function decodeUtf8(bytes: Uint8Array): string | null {
  try {
    return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes)
  } catch (error) {
    if (error instanceof TypeError) return null
    throw error
  }
}

function assertSessionPayload(value: unknown): asserts value is SessionPayload {
  if (!isRecord(value) || !isRecord(value.report)) {
    throw new Error('The viewer returned an invalid session: `report` is missing.')
  }
  if (!isRecord(value.verification) || value.verification.verified !== true || typeof value.verification.message !== 'string') {
    throw new Error('The viewer did not provide a successful verification result.')
  }

  const report = value.report
  if (
    typeof report.schema !== 'string' ||
    typeof report.engine_version !== 'string' ||
    !isRecord(report.parser) ||
    !Array.isArray(report.relations) ||
    !Array.isArray(report.ambiguities) ||
    !Array.isArray(report.changes) ||
    !isRecord(report.patch) ||
    !Array.isArray(report.patch.edits) ||
    !isRecord(report.before) ||
    typeof report.before.path !== 'string' ||
    typeof report.before.byte_len !== 'number' ||
    !isRecord(report.after) ||
    typeof report.after.path !== 'string' ||
    typeof report.after.byte_len !== 'number' ||
    !isRecord(report.certificate) ||
    !isRecord(report.summary)
  ) {
    throw new Error('The viewer returned an invalid StrataDiff report.')
  }
}

export async function fetchSession(search: string, signal?: AbortSignal): Promise<LoadedSession> {
  const token = getSessionToken(search)
  const query = `token=${encodeURIComponent(token)}`
  const request = (path: string, accept: string) => fetch(`${path}?${query}`, {
    signal,
    cache: 'no-store',
    headers: { Accept: accept },
  })
  const [sessionResponse, beforeResponse, afterResponse] = await Promise.all([
    request('/api/session', 'application/json'),
    request('/api/source/before', 'application/octet-stream'),
    request('/api/source/after', 'application/octet-stream'),
  ])

  const responses = [
    ['Session', sessionResponse],
    ['Before source', beforeResponse],
    ['After source', afterResponse],
  ] as const
  for (const [label, response] of responses) {
    if (!response.ok) throw new Error(`${label} request failed (${response.status} ${response.statusText}).`)
  }

  const [payload, beforeBuffer, afterBuffer]: [unknown, ArrayBuffer, ArrayBuffer] = await Promise.all([
    sessionResponse.json(),
    beforeResponse.arrayBuffer(),
    afterResponse.arrayBuffer(),
  ])
  assertSessionPayload(payload)
  const beforeBytes = new Uint8Array(beforeBuffer)
  const afterBytes = new Uint8Array(afterBuffer)

  if (beforeBytes.byteLength !== payload.report.before.byte_len || afterBytes.byteLength !== payload.report.after.byte_len) {
    throw new Error('Session source lengths do not match the verified report.')
  }

  return {
    ...payload,
    decodedBefore: {
      path: payload.report.before.path,
      bytes: beforeBytes,
      text: decodeUtf8(beforeBytes),
    },
    decodedAfter: {
      path: payload.report.after.path,
      bytes: afterBytes,
      text: decodeUtf8(afterBytes),
    },
  }
}

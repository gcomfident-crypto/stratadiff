import type {
  DecodedArtifact,
  FileSessionPayload,
  LoadedFileSession,
  LoadedSession,
  RepositorySessionPayload,
  ReviewFile,
  SessionPayload,
} from '../types'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function getSessionToken(search: string): string {
  const token = new URLSearchParams(search).get('token')
  if (token === null || token.length === 0) {
    throw new Error('Missing viewer session token. Launch this page with `stratadiff view` or `stratadiff review --workbench`.')
  }
  return token
}

function authenticatedQuery(search: string, file?: number, scope?: 'resume' | 'full'): string {
  const params = new URLSearchParams({ token: getSessionToken(search) })
  const selectedFile = file ?? (() => {
    const value = new URLSearchParams(search).get('file')
    return value === null ? undefined : Number(value)
  })()
  if (selectedFile !== undefined) {
    if (!Number.isSafeInteger(selectedFile) || selectedFile < 0) throw new Error('Invalid repository file index.')
    params.set('file', String(selectedFile))
    const selectedScope = scope ?? new URLSearchParams(search).get('scope') ?? 'resume'
    if (selectedScope !== 'resume' && selectedScope !== 'full') throw new Error('Invalid repository review scope.')
    params.set('scope', selectedScope)
  }
  return params.toString()
}

export function decodeUtf8(bytes: Uint8Array): string | null {
  try {
    return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes)
  } catch (error) {
    if (error instanceof TypeError) return null
    throw error
  }
}

function assertDiffSession(value: unknown): asserts value is FileSessionPayload {
  if (!isRecord(value) || value.kind !== 'file_diff' || !isRecord(value.report)) {
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
  if (value.repository_context !== undefined && (
    !isRecord(value.repository_context) ||
    !Number.isSafeInteger(value.repository_context.file_index) ||
    (value.repository_context.file_index as number) < 0 ||
    !['resume', 'full'].includes(String(value.repository_context.scope))
  )) {
    throw new Error('The viewer returned invalid repository navigation context.')
  }
}

function assertRepositorySession(value: unknown): asserts value is RepositorySessionPayload {
  if (!isRecord(value)) throw new Error('The viewer returned an invalid repository review.')
  if (!isRecord(value.review)) throw new Error('The viewer returned an invalid repository review.')
  const review = value.review
  if (!isRecord(review.summary) || !Array.isArray(review.files)) throw new Error('The viewer returned an invalid repository review.')
  if (!isRecord(value.resume_delta)) throw new Error('The viewer returned an invalid checkpoint delta.')
  const resumeDelta = value.resume_delta
  const summary = review.summary
  const reviewFiles = review.files
  if (!isRecord(resumeDelta.summary) || !Array.isArray(resumeDelta.files)) throw new Error('The viewer returned an invalid checkpoint delta.')
  const resumeSummary = resumeDelta.summary
  const resumeFiles = resumeDelta.files
  if (
    typeof review.schema !== 'string' ||
    typeof review.engine_version !== 'string' ||
    typeof review.requested_base !== 'string' ||
    typeof review.requested_head !== 'string' ||
    typeof review.base_commit !== 'string' ||
    typeof review.head_commit !== 'string' ||
    typeof summary.changed_files !== 'number' ||
    summary.changed_files !== reviewFiles.length
  ) {
    throw new Error('The viewer returned inconsistent repository review metadata.')
  }
  if (!isRecord(review.checkpoint) || typeof review.checkpoint.commit !== 'string') {
    throw new Error('The repository workbench requires a resolved checkpoint.')
  }
  if (
    resumeDelta.comparison !== 'snapshot_to_snapshot' ||
    typeof resumeDelta.from_commit !== 'string' ||
    typeof resumeDelta.to_commit !== 'string' ||
    resumeDelta.from_commit !== review.checkpoint.commit ||
    resumeDelta.to_commit !== review.head_commit ||
    typeof resumeSummary.changed_files !== 'number' ||
    resumeSummary.changed_files !== resumeFiles.length
  ) {
    throw new Error('The viewer returned inconsistent checkpoint delta metadata.')
  }
  for (const file of reviewFiles) {
    if (
      !isRecord(file) ||
      typeof file.status !== 'string' ||
      typeof file.priority !== 'string' ||
      typeof file.lane !== 'string' ||
      typeof file.reason !== 'string'
    ) {
      throw new Error('The viewer returned an invalid repository review file.')
    }
  }
  for (const file of resumeFiles) {
    if (
      !isRecord(file) ||
      typeof file.status !== 'string' ||
      typeof file.priority !== 'string' ||
      typeof file.lane !== 'string' ||
      typeof file.reason !== 'string'
    ) {
      throw new Error('The viewer returned an invalid checkpoint delta file.')
    }
  }
  if (
    !isRecord(value.assessment) ||
    value.assessment.status !== 'producer_attested' ||
    value.assessment.basis !== 'exact_git_change_identity' ||
    typeof value.assessment.message !== 'string'
  ) {
    throw new Error('The repository review is missing its attestation boundary.')
  }
}

function assertSessionPayload(value: unknown): asserts value is SessionPayload {
  if (!isRecord(value) || typeof value.kind !== 'string') {
    throw new Error('The viewer returned an invalid session kind.')
  }
  if (value.kind === 'file_diff') assertDiffSession(value)
  else if (value.kind === 'repository_review') assertRepositorySession(value)
  else throw new Error(`The viewer returned an unsupported session kind: ${value.kind}`)
}

async function checkedResponse(response: Response, label: string): Promise<Response> {
  if (!response.ok) throw new Error(`${label} request failed (${response.status} ${response.statusText}).`)
  return response
}

function decodedArtifact(path: string, buffer: ArrayBuffer): DecodedArtifact {
  const bytes = new Uint8Array(buffer)
  return { path, bytes, text: decodeUtf8(bytes) }
}

export async function fetchSession(search: string, signal?: AbortSignal): Promise<LoadedSession> {
  const query = authenticatedQuery(search)
  const sessionResponse = await fetch(`/api/session?${query}`, {
    signal,
    cache: 'no-store',
    headers: { Accept: 'application/json' },
  })
  await checkedResponse(sessionResponse, 'Session')
  const payload: unknown = await sessionResponse.json()
  assertSessionPayload(payload)
  if (payload.kind === 'repository_review') return payload

  const request = (path: string) => fetch(`${path}?${query}`, {
    signal,
    cache: 'no-store',
    headers: { Accept: 'application/octet-stream' },
  })
  const [beforeResponse, afterResponse] = await Promise.all([
    request('/api/source/before'),
    request('/api/source/after'),
  ])
  await Promise.all([
    checkedResponse(beforeResponse, 'Before source'),
    checkedResponse(afterResponse, 'After source'),
  ])
  const [beforeBuffer, afterBuffer] = await Promise.all([
    beforeResponse.arrayBuffer(),
    afterResponse.arrayBuffer(),
  ])
  const before = decodedArtifact(payload.report.before.path, beforeBuffer)
  const after = decodedArtifact(payload.report.after.path, afterBuffer)
  if (before.bytes.byteLength !== payload.report.before.byte_len || after.bytes.byteLength !== payload.report.after.byte_len) {
    throw new Error('Session source lengths do not match the verified report.')
  }

  return { ...payload, decodedBefore: before, decodedAfter: after } satisfies LoadedFileSession
}

export async function fetchReviewFileSources(
  search: string,
  index: number,
  scope: 'resume' | 'full',
  file: ReviewFile,
  signal?: AbortSignal,
): Promise<{ before: DecodedArtifact; after: DecodedArtifact }> {
  const query = authenticatedQuery(search, index, scope)
  const request = (side: 'before' | 'after') => fetch(`/api/source/${side}?${query}`, {
    signal,
    cache: 'no-store',
    headers: { Accept: 'application/octet-stream' },
  })
  const [beforeResponse, afterResponse] = await Promise.all([request('before'), request('after')])
  await Promise.all([
    checkedResponse(beforeResponse, 'Before source'),
    checkedResponse(afterResponse, 'After source'),
  ])
  const [beforeBuffer, afterBuffer] = await Promise.all([
    beforeResponse.arrayBuffer(),
    afterResponse.arrayBuffer(),
  ])
  const before = decodedArtifact(file.before_path ?? '/dev/null', beforeBuffer)
  const after = decodedArtifact(file.after_path ?? '/dev/null', afterBuffer)
  if (file.before_bytes !== undefined && before.bytes.byteLength !== file.before_bytes) {
    throw new Error('Before source length does not match the repository review.')
  }
  if (file.after_bytes !== undefined && after.bytes.byteLength !== file.after_bytes) {
    throw new Error('After source length does not match the repository review.')
  }
  return { before, after }
}

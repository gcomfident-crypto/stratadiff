import type {
  DecodedArtifact,
  FileSessionPayload,
  LoadedFileSession,
  LoadedSession,
  RepositorySessionPayload,
  ReviewCoverageSessionPayload,
  ReviewFile,
  SessionPayload,
} from '../types'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

const reviewDeltaBaselineBases = [
  'checkpoint_snapshot',
  'current_base_no_checkpoint_change',
  'reconstructed_review_baseline',
  'current_base_fallback',
  'checkpoint_head_fallback',
] as const

const reviewDeltaFallbackReasons = [
  'overlap_or_adjacent',
  'binary_nul',
  'source_unavailable',
  'unsupported_change',
  'translation_failed',
  'replay_orders_mismatch',
] as const

const reviewDeltaSchema = 'https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-delta-v1.schema.json'
const reviewCoverageSchema = 'https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-coverage-v1.schema.json'
const coverageStates = ['covered', 'needs_review', 'blocked'] as const
const coverageScopes = ['current_change', 'retired_residue'] as const

function isReviewFile(value: unknown): value is Record<string, unknown> {
  return isRecord(value) &&
    typeof value.status === 'string' &&
    typeof value.priority === 'string' &&
    typeof value.lane === 'string' &&
    typeof value.reason === 'string'
}

function isDeltaSource(value: unknown): value is Record<string, unknown> {
  if (!isRecord(value) || typeof value.kind !== 'string') return false
  if (value.kind === 'empty') return true
  if (value.kind === 'reconstructed_bytes') {
    return typeof value.blake3 === 'string' && typeof value.byte_len === 'number'
  }
  return value.kind === 'git_object' &&
    typeof value.commit === 'string' &&
    typeof value.object_id === 'string' &&
    (value.byte_len === undefined || typeof value.byte_len === 'number')
}

function isUnresolvedRetiredChange(value: unknown): value is Record<string, unknown> {
  return isRecord(value) &&
    typeof value.path === 'string' &&
    ['utf8', 'git_bytes_percent_encoded'].includes(String(value.path_encoding)) &&
    value.reason === 'non_utf8_git_path'
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
    !['resume', 'full'].includes(String(value.repository_context.scope)) ||
    (value.repository_context.checkpoint_state !== undefined &&
      !['needs_review_now', 'unchanged_since_checkpoint'].includes(String(value.repository_context.checkpoint_state))) ||
    (value.repository_context.checkpoint_match_basis !== undefined &&
      !['exact_git_change_identity', 'exact_noninteracting_four_way_byte_replay'].includes(String(value.repository_context.checkpoint_match_basis))) ||
    (value.repository_context.baseline_basis !== undefined &&
      !(reviewDeltaBaselineBases as readonly string[]).includes(String(value.repository_context.baseline_basis))) ||
    (value.repository_context.before_source !== undefined && !isDeltaSource(value.repository_context.before_source)) ||
    (value.repository_context.after_source !== undefined && !isDeltaSource(value.repository_context.after_source))
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
  if (!isRecord(resumeDelta.summary) || !Array.isArray(resumeDelta.entries)) throw new Error('The viewer returned an invalid checkpoint delta.')
  const resumeSummary = resumeDelta.summary
  const resumeEntries = resumeDelta.entries
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
  if (
    !isRecord(review.checkpoint) ||
    typeof review.checkpoint.commit !== 'string' ||
    typeof review.checkpoint.base_commit !== 'string' ||
    typeof review.checkpoint.match_basis !== 'string'
  ) {
    throw new Error('The repository workbench requires a resolved checkpoint.')
  }
  const baseChanged = review.checkpoint.base_commit !== review.base_commit
  const expectedBasis = baseChanged
    ? 'exact_git_change_identity_or_noninteracting_four_way_byte_replay'
    : 'exact_git_change_identity'
  if (
    resumeDelta.schema !== reviewDeltaSchema ||
    typeof resumeDelta.engine_version !== 'string' ||
    resumeDelta.engine_version !== review.engine_version ||
    !['checkpoint_to_head', 'per_file_review_baseline_to_head'].includes(String(resumeDelta.comparison)) ||
    resumeDelta.old_base_commit !== review.checkpoint.base_commit ||
    resumeDelta.checkpoint_commit !== review.checkpoint.commit ||
    resumeDelta.current_base_commit !== review.base_commit ||
    resumeDelta.head_commit !== review.head_commit ||
    !Array.isArray(resumeDelta.unresolved_retired_changes) ||
    !resumeDelta.unresolved_retired_changes.every(isUnresolvedRetiredChange) ||
    typeof resumeSummary.displayable_files !== 'number' ||
    resumeSummary.displayable_files !== resumeEntries.length ||
    typeof resumeSummary.unresolved_retired_changes !== 'number' ||
    resumeSummary.unresolved_retired_changes !== resumeDelta.unresolved_retired_changes.length ||
    typeof resumeSummary.needs_review_files !== 'number' ||
    resumeSummary.needs_review_files !== resumeEntries.length + resumeDelta.unresolved_retired_changes.length ||
    typeof resumeSummary.gate_passed !== 'boolean' ||
    resumeSummary.gate_passed !== (resumeSummary.needs_review_files === 0)
  ) {
    throw new Error('The viewer returned inconsistent checkpoint delta metadata.')
  }
  if (
    (resumeDelta.comparison === 'checkpoint_to_head' && baseChanged) ||
    (resumeDelta.comparison === 'per_file_review_baseline_to_head' && !baseChanged)
  ) {
    throw new Error('The viewer returned an invalid review-residue scope.')
  }
  for (const file of reviewFiles) {
    if (
      !isReviewFile(file)
    ) {
      throw new Error('The viewer returned an invalid repository review file.')
    }
    if (
      (file.checkpoint_state === 'unchanged_since_checkpoint' &&
        !['exact_git_change_identity', 'exact_noninteracting_four_way_byte_replay'].includes(String(file.checkpoint_match_basis))) ||
      (file.checkpoint_state === 'needs_review_now' && file.checkpoint_match_basis !== undefined)
    ) {
      throw new Error('The viewer returned inconsistent checkpoint carry evidence.')
    }
  }
  for (const entry of resumeEntries) {
    if (
      !isRecord(entry) ||
      !isReviewFile(entry.file) ||
      !(reviewDeltaBaselineBases as readonly string[]).includes(String(entry.baseline_basis)) ||
      !isDeltaSource(entry.before_source) ||
      !isDeltaSource(entry.after_source) ||
      (entry.fallback_reason !== undefined &&
        !(reviewDeltaFallbackReasons as readonly string[]).includes(String(entry.fallback_reason)))
    ) {
      throw new Error('The viewer returned an invalid checkpoint delta file.')
    }
    const fallback = entry.baseline_basis === 'current_base_fallback' || entry.baseline_basis === 'checkpoint_head_fallback'
    if (fallback !== (entry.fallback_reason !== undefined)) {
      throw new Error('The viewer returned inconsistent checkpoint delta fallback evidence.')
    }
    const reconstructed = entry.baseline_basis === 'reconstructed_review_baseline'
    if (reconstructed) {
      const evidence = entry.baseline_reconstruction
      if (
        entry.before_source.kind !== 'reconstructed_bytes' ||
        !isRecord(evidence) ||
        evidence.algorithm !== 'bidirectional_noninteracting_byte_replay_v1' ||
        typeof evidence.reviewed_on_current_base_blake3 !== 'string' ||
        evidence.reviewed_on_current_base_blake3 !== evidence.upstream_on_checkpoint_blake3 ||
        evidence.reconstructed_blake3 !== evidence.reviewed_on_current_base_blake3 ||
        evidence.reconstructed_blake3 !== entry.before_source.blake3 ||
        evidence.byte_len !== entry.before_source.byte_len
      ) {
        throw new Error('The viewer returned invalid reconstructed-baseline evidence.')
      }
    } else if (entry.baseline_reconstruction !== undefined || entry.before_source.kind === 'reconstructed_bytes') {
      throw new Error('The viewer returned reconstructed bytes without reconstruction evidence.')
    }
    const expectedBeforeCommit = entry.baseline_basis === 'checkpoint_snapshot' || entry.baseline_basis === 'checkpoint_head_fallback'
      ? review.checkpoint.commit
      : review.base_commit
    if (entry.before_source.kind === 'git_object' && entry.before_source.commit !== expectedBeforeCommit) {
      throw new Error('The viewer returned a checkpoint delta from the wrong Git commit.')
    }
    if (entry.after_source.kind === 'git_object' && entry.after_source.commit !== review.head_commit) {
      throw new Error('The viewer returned a checkpoint delta targeting the wrong Git commit.')
    }
  }
  if (
    !isRecord(value.assessment) ||
    value.assessment.status !== 'producer_attested' ||
    value.assessment.basis !== expectedBasis ||
    review.checkpoint.match_basis !== expectedBasis ||
    typeof value.assessment.message !== 'string'
  ) {
    throw new Error('The repository review is missing its attestation boundary.')
  }
}

function isNonNegativeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0
}

function isIdList(value: unknown): value is number[] {
  return Array.isArray(value) && value.every((entry) => Number.isSafeInteger(entry) && entry > 0)
}

function isCodeowner(value: unknown): boolean {
  if (!isRecord(value)) return false
  if (value.kind === 'user') return typeof value.login === 'string' && value.login.length > 0
  if (value.kind === 'team') {
    return typeof value.organization === 'string' && value.organization.length > 0 &&
      typeof value.slug === 'string' && value.slug.length > 0
  }
  return value.kind === 'email' && typeof value.address === 'string' && value.address.length > 0
}

function isOwnerCoverage(value: unknown): boolean {
  return isRecord(value) &&
    isCodeowner(value.owner) &&
    isIdList(value.eligible_reviewer_ids) &&
    isIdList(value.active_review_ids) &&
    isIdList(value.covering_review_ids) &&
    Array.isArray(value.blockers) &&
    value.blockers.every((blocker) => typeof blocker === 'string' && blocker.length > 0)
}

function isCoverageFile(value: unknown): boolean {
  if (!isRecord(value) || !isRecord(value.change)) return false
  const matchingRule = value.matching_rule
  return (coverageScopes as readonly unknown[]).includes(value.scope) &&
    typeof value.change.status === 'string' &&
    typeof value.path === 'string' && value.path.length > 0 &&
    ['utf8', 'git_bytes_percent_encoded'].includes(String(value.path_encoding)) &&
    (matchingRule === undefined || (
      isRecord(matchingRule) &&
      Number.isSafeInteger(matchingRule.line) && (matchingRule.line as number) > 0 &&
      typeof matchingRule.pattern === 'string' && matchingRule.pattern.length > 0 &&
      Array.isArray(matchingRule.owner_alternatives) && matchingRule.owner_alternatives.every(isCodeowner)
    )) &&
    Array.isArray(value.owner_alternatives) && value.owner_alternatives.every(isOwnerCoverage) &&
    (coverageStates as readonly unknown[]).includes(value.state) &&
    typeof value.reason === 'string' && value.reason.length > 0
}

function assertReviewCoverageSession(value: unknown): asserts value is ReviewCoverageSessionPayload {
  if (!isRecord(value) || value.kind !== 'review_coverage_passport' || !isRecord(value.passport)) {
    throw new Error('The viewer returned an invalid review coverage passport.')
  }
  if (!isRecord(value.verification) || value.verification.verified !== true || typeof value.verification.message !== 'string') {
    throw new Error('The review coverage passport was not independently verified.')
  }
  const passport = value.passport
  if (passport.schema !== reviewCoverageSchema || !isRecord(passport.body) || !isRecord(passport.attestation)) {
    throw new Error('The viewer returned an unsupported review coverage passport.')
  }
  const body = passport.body
  const summary = body.summary
  if (
    typeof body.engine_version !== 'string' ||
    typeof body.protected_base_commit !== 'string' ||
    typeof body.merge_base_commit !== 'string' ||
    typeof body.head_commit !== 'string' ||
    !isRecord(body.ledger) ||
    !isRecord(body.ledger.repository) ||
    typeof body.ledger.repository.full_name !== 'string' ||
    !isRecord(body.ledger.pull_request) ||
    !Number.isSafeInteger(body.ledger.pull_request.number) ||
    !isRecord(body.ledger.receiver) ||
    body.ledger.receiver.algorithm !== 'ed25519' ||
    typeof body.ledger.receiver.key_id !== 'string' ||
    typeof body.ledger.receiver.public_key !== 'string' ||
    !Array.isArray(body.ledger.review_receipts) ||
    !Array.isArray(body.ledger.dismissals) ||
    !isRecord(body.ownership) ||
    body.ownership.base_commit !== body.protected_base_commit ||
    typeof body.ownership.observed_at !== 'string' ||
    !Array.isArray(body.checkpoint_proofs) ||
    !Array.isArray(body.files) || !body.files.every(isCoverageFile) ||
    !Array.isArray(body.unresolved_residue) ||
    !body.unresolved_residue.every((entry) => isRecord(entry) &&
      typeof entry.checkpoint_commit === 'string' &&
      typeof entry.path === 'string' &&
      ['utf8', 'git_bytes_percent_encoded'].includes(String(entry.path_encoding)) &&
      typeof entry.reason === 'string') ||
    !Array.isArray(body.non_claims) || !body.non_claims.every((claim) => typeof claim === 'string' && claim.length > 0) ||
    !isRecord(summary)
  ) {
    throw new Error('The viewer returned incomplete review coverage evidence.')
  }
  const summaryFields = [
    summary.current_files,
    summary.retired_residue_files,
    summary.unresolved_residue,
    summary.total_requirements,
    summary.covered_files,
    summary.needs_review_files,
    summary.blocked_files,
    summary.active_review_receipts,
    summary.unique_checkpoint_proofs,
  ]
  const covered = body.files.filter((file) => isRecord(file) && file.state === 'covered').length
  const needsReview = body.files.filter((file) => isRecord(file) && file.state === 'needs_review').length
  const blocked = body.files.filter((file) => isRecord(file) && file.state === 'blocked').length + body.unresolved_residue.length
  const current = body.files.filter((file) => isRecord(file) && file.scope === 'current_change').length
  const retired = body.files.filter((file) => isRecord(file) && file.scope === 'retired_residue').length
  if (
    !summaryFields.every(isNonNegativeInteger) ||
    summary.current_files !== current ||
    summary.retired_residue_files !== retired ||
    summary.unresolved_residue !== body.unresolved_residue.length ||
    summary.total_requirements !== body.files.length + body.unresolved_residue.length ||
    summary.covered_files !== covered ||
    summary.needs_review_files !== needsReview ||
    summary.blocked_files !== blocked ||
    typeof summary.gate_passed !== 'boolean' ||
    summary.gate_passed !== (summary.covered_files === summary.total_requirements) ||
    passport.attestation.algorithm !== 'ed25519' ||
    passport.attestation.key_id !== body.ledger.receiver.key_id ||
    typeof passport.attestation.body_sha256 !== 'string' ||
    typeof passport.attestation.signature !== 'string'
  ) {
    throw new Error('The viewer returned inconsistent review coverage metadata.')
  }
  if (body.codeowners_source !== undefined && (
    !isRecord(body.codeowners_source) ||
    body.codeowners_source.base_commit !== body.protected_base_commit ||
    !['.github/CODEOWNERS', 'CODEOWNERS', 'docs/CODEOWNERS'].includes(String(body.codeowners_source.path)) ||
    typeof body.codeowners_source.blob_oid !== 'string' ||
    typeof body.codeowners_source.blake3 !== 'string' ||
    !isNonNegativeInteger(body.codeowners_source.byte_len)
  )) {
    throw new Error('The viewer returned invalid CODEOWNERS provenance.')
  }
}

function assertSessionPayload(value: unknown): asserts value is SessionPayload {
  if (!isRecord(value) || typeof value.kind !== 'string') {
    throw new Error('The viewer returned an invalid session kind.')
  }
  if (value.kind === 'file_diff') assertDiffSession(value)
  else if (value.kind === 'repository_review') assertRepositorySession(value)
  else if (value.kind === 'review_coverage_passport') assertReviewCoverageSession(value)
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
  if (payload.kind !== 'file_diff') return payload

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

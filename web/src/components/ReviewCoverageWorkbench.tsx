import {
  AlertTriangle,
  Check,
  CircleSlash2,
  Download,
  FileKey2,
  GitBranch,
  Search,
  ShieldCheck,
  ShieldX,
  Users,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { getSessionToken } from '../lib/session'
import type {
  CodeownerIdentity,
  CoverageState,
  FileCoverage,
  OwnerCoverage,
  ReviewCoverageSessionPayload,
} from '../types'

type CoverageFilter = 'all' | CoverageState

function shortHash(value: string): string {
  return value.length <= 14 ? value : `${value.slice(0, 8)}…${value.slice(-6)}`
}

function ownerLabel(owner: CodeownerIdentity): string {
  if (owner.kind === 'user') return `@${owner.login}`
  if (owner.kind === 'team') return `@${owner.organization}/${owner.slug}`
  return owner.address
}

function ownerState(owner: OwnerCoverage): CoverageState {
  if (owner.blockers.length > 0) return 'blocked'
  return owner.covering_review_ids.length > 0 ? 'covered' : 'needs_review'
}

function stateLabel(state: CoverageState): string {
  if (state === 'covered') return 'Covered'
  if (state === 'needs_review') return 'Needs review'
  return 'Blocked'
}

function StateIcon({ state }: { state: CoverageState }) {
  if (state === 'covered') return <Check size={14} />
  if (state === 'blocked') return <CircleSlash2 size={14} />
  return <AlertTriangle size={14} />
}

function OwnerCell({ coverage }: { coverage: OwnerCoverage }) {
  const state = ownerState(coverage)
  return (
    <article className={`coverage-owner ${state}`}>
      <div className="coverage-owner-heading">
        <strong>{ownerLabel(coverage.owner)}</strong>
        <span className={`coverage-state ${state}`}><StateIcon state={state} />{stateLabel(state)}</span>
      </div>
      <dl>
        <div><dt>Eligible reviewer IDs</dt><dd>{coverage.eligible_reviewer_ids.length > 0 ? coverage.eligible_reviewer_ids.join(', ') : 'none'}</dd></div>
        <div><dt>Active review IDs</dt><dd>{coverage.active_review_ids.length > 0 ? coverage.active_review_ids.join(', ') : 'none'}</dd></div>
        <div><dt>Covering review IDs</dt><dd>{coverage.covering_review_ids.length > 0 ? coverage.covering_review_ids.join(', ') : 'none'}</dd></div>
      </dl>
      {coverage.blockers.length > 0 && (
        <ul className="coverage-blockers" aria-label={`Blockers for ${ownerLabel(coverage.owner)}`}>
          {coverage.blockers.map((blocker, index) => <li key={`${blocker}-${index}`}>{blocker}</li>)}
        </ul>
      )}
    </article>
  )
}

function CoverageFileCard({ file }: { file: FileCoverage }) {
  return (
    <article className={`coverage-file-card ${file.state}`}>
      <header>
        <div className="coverage-path">
          <span className={`coverage-state ${file.state}`}><StateIcon state={file.state} />{stateLabel(file.state)}</span>
          <h2>{file.path}</h2>
        </div>
        <div className="coverage-file-tags">
          <span>{file.scope === 'retired_residue' ? 'Retired residue' : 'Current change'}</span>
          <span>{file.change.status.replaceAll('_', ' ')}</span>
          {file.path_encoding !== 'utf8' && <span>Encoded Git path</span>}
        </div>
      </header>
      <p className="coverage-reason">{file.reason}</p>
      <div className="coverage-rule">
        <FileKey2 size={15} />
        {file.matching_rule === undefined
          ? <span>No usable CODEOWNERS rule was established.</span>
          : <span><strong>Line {file.matching_rule.line}</strong><code>{file.matching_rule.pattern}</code>Any one listed owner may satisfy this rule.</span>}
      </div>
      {file.owner_alternatives.length > 0
        ? <div className="coverage-owners">{file.owner_alternatives.map((owner) => <OwnerCell key={ownerLabel(owner.owner)} coverage={owner} />)}</div>
        : <div className="coverage-no-owner"><Users size={15} />No eligible owner alternative is available for this requirement.</div>}
    </article>
  )
}

export function ReviewCoverageWorkbench({ session }: { session: ReviewCoverageSessionPayload }) {
  const { passport, verification } = session
  const { body, attestation } = passport
  const { summary } = body
  const [filter, setFilter] = useState<CoverageFilter>('all')
  const [query, setQuery] = useState('')
  const normalizedQuery = query.trim().toLocaleLowerCase()
  const files = useMemo(() => body.files.filter((file) => {
    if (filter !== 'all' && file.state !== filter) return false
    if (normalizedQuery.length === 0) return true
    return file.path.toLocaleLowerCase().includes(normalizedQuery) ||
      file.reason.toLocaleLowerCase().includes(normalizedQuery) ||
      file.owner_alternatives.some((owner) => ownerLabel(owner.owner).toLocaleLowerCase().includes(normalizedQuery))
  }), [body.files, filter, normalizedQuery])
  const token = getSessionToken(window.location.search)
  const downloadHref = `/api/passport?token=${encodeURIComponent(token)}`
  const gateState: CoverageState = summary.gate_passed ? 'covered' : summary.blocked_files > 0 ? 'blocked' : 'needs_review'

  return (
    <div className="coverage-shell">
      <header className="coverage-header">
        <div className="brand-block">
          <div className="brand-mark"><span /><span /><span /></div>
          <div><div className="eyebrow">STRATADIFF</div><div className="brand-title">Review Coverage</div></div>
        </div>
        <div className="coverage-repository">
          <strong>{body.ledger.repository.full_name}</strong>
          <span>PR #{body.ledger.pull_request.number}</span>
        </div>
        <span className="coverage-verified-chip"><ShieldCheck size={14} />OFFLINE VERIFIED</span>
        <a className="export-button" href={downloadHref} download="review-coverage-passport.json"><Download size={15} />Download passport</a>
      </header>

      <main className="coverage-main">
        <section className={`coverage-hero ${gateState}`} aria-label="Review coverage gate">
          <div className="coverage-hero-copy">
            <span className="eyebrow">REVIEWER × CODEOWNERS COVERAGE</span>
            <div className="coverage-gate-title">
              {summary.gate_passed ? <ShieldCheck size={30} /> : <ShieldX size={30} />}
              <h1>Gate {summary.gate_passed ? 'PASSED' : 'FAILED'}</h1>
            </div>
            <p>{summary.gate_passed
              ? 'Every current and retired review requirement is covered by verified evidence.'
              : 'At least one required owner review is missing or blocked; the exact residue remains visible below.'}</p>
            <small>This is an independently verified coverage decision. It does not create, restore, or replace GitHub approval.</small>
          </div>
          <div className="coverage-stats" aria-label="Coverage summary">
            <div className="covered"><span>Covered</span><strong>{summary.covered_files}</strong></div>
            <div className="needs"><span>Needs review</span><strong>{summary.needs_review_files}</strong></div>
            <div className="blocked"><span>Blocked</span><strong>{summary.blocked_files}</strong></div>
            <div><span>Retired</span><strong>{summary.retired_residue_files}</strong></div>
            <div><span>Unresolved</span><strong>{summary.unresolved_residue}</strong></div>
          </div>
        </section>

        <section className="coverage-provenance" aria-label="Verified policy provenance">
          <div className="coverage-provenance-title"><GitBranch size={16} /><div><span>EXACT POLICY SOURCE</span><strong>{body.codeowners_source?.path ?? 'CODEOWNERS unavailable'}</strong></div></div>
          <dl>
            <div><dt>Protected base</dt><dd title={body.protected_base_commit}>{shortHash(body.protected_base_commit)}</dd></div>
            <div><dt>Head</dt><dd title={body.head_commit}>{shortHash(body.head_commit)}</dd></div>
            <div><dt>CODEOWNERS blob</dt><dd title={body.codeowners_source?.blob_oid}>{body.codeowners_source === undefined ? 'not established' : shortHash(body.codeowners_source.blob_oid)}</dd></div>
            <div><dt>Ownership observed</dt><dd>{body.ownership.observed_at}</dd></div>
          </dl>
        </section>

        <section className="coverage-toolbar" aria-label="Coverage filters">
          <label><Search size={15} /><span className="sr-only">Search paths or owners</span><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search paths or owners" /></label>
          <div className="coverage-filter-buttons">
            {(['all', 'needs_review', 'blocked', 'covered'] as const).map((value) => (
              <button key={value} type="button" className={filter === value ? 'active' : ''} onClick={() => setFilter(value)}>
                {value === 'all' ? 'All requirements' : stateLabel(value)}
              </button>
            ))}
          </div>
        </section>

        <div className="coverage-content">
          <section className="coverage-file-list" aria-label="File and owner coverage">
            <div className="coverage-section-heading"><span>FILE × OWNER MATRIX</span><strong>{files.length} / {body.files.length}</strong></div>
            {files.length > 0
              ? files.map((file, index) => <CoverageFileCard key={`${file.scope}-${file.path}-${index}`} file={file} />)
              : <div className="coverage-empty">No file requirements match this view.</div>}
            {body.unresolved_residue.length > 0 && filter !== 'covered' && filter !== 'needs_review' && (
              <section className="coverage-unresolved">
                <h2><CircleSlash2 size={16} />Unresolved retired residue</h2>
                <p>These requirements could not be mapped to a displayable file row and remain blocking.</p>
                {body.unresolved_residue.map((entry) => (
                  <article key={`${entry.checkpoint_commit}-${entry.path}`}>
                    <strong>{entry.path}</strong><span>{entry.reason}</span><code>{shortHash(entry.checkpoint_commit)}</code>
                  </article>
                ))}
              </section>
            )}
          </section>

          <aside className="coverage-proof-panel" aria-label="Passport verification">
            <section>
              <h2><ShieldCheck size={15} />Passport verified</h2>
              <p>{verification.message}</p>
              <dl>
                <div><dt>Attestation</dt><dd>{attestation.algorithm}</dd></div>
                <div><dt>Receiver key</dt><dd title={body.ledger.receiver.public_key}>{attestation.key_id}</dd></div>
                <div><dt>Body SHA-256</dt><dd title={attestation.body_sha256}>{shortHash(attestation.body_sha256)}</dd></div>
                <div><dt>Active receipts</dt><dd>{summary.active_review_receipts}</dd></div>
                <div><dt>Checkpoint proofs</dt><dd>{summary.unique_checkpoint_proofs}</dd></div>
              </dl>
            </section>
            <section>
              <h2><AlertTriangle size={15} />Claim boundary</h2>
              <p>No source code is served by this view. The passport proves its recorded coverage decision against the supplied Git object store.</p>
              <ul>{body.non_claims.map((claim, index) => <li key={`${claim}-${index}`}>{claim}</li>)}</ul>
            </section>
          </aside>
        </div>
      </main>
    </div>
  )
}

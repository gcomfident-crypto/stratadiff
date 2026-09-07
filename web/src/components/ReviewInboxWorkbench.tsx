import {
  AlertTriangle,
  ArrowRight,
  CheckCircle2,
  ExternalLink,
  GitPullRequest,
  Inbox,
  LoaderCircle,
  LockKeyhole,
  RotateCcw,
  ShieldCheck,
} from 'lucide-react'
import { useState } from 'react'
import { continueInboxReview } from '../lib/session'
import type { ReviewInboxSessionPayload, ReviewInboxUnobservableItem } from '../types'

const unobservableCopy: Record<ReviewInboxUnobservableItem['reason'], string> = {
  head_oid_unavailable: 'The current head commit was unavailable.',
  checkpoint_base_oid_unavailable: 'The base commit recorded with the review was unavailable.',
  current_base_oid_unavailable: 'The current base commit was unavailable.',
  resume_review_limit_exceeded: 'The bounded review history was too large to verify safely.',
}

function formatTimestamp(value: string): string {
  return value.replace('T', ' ').replace(/(?:\.[0-9]+)?Z$/, ' UTC')
}

function formatObservedAt(seconds: number): string {
  const date = new Date(seconds * 1000)
  return Number.isNaN(date.getTime()) ? `Unix time ${seconds}` : formatTimestamp(date.toISOString())
}

function shortOid(oid: string): string {
  return oid.slice(0, 9)
}

function emptyState(session: ReviewInboxSessionPayload) {
  if (session.summary.status === 'up_to_date') {
    return {
      icon: <CheckCircle2 size={22} />,
      title: 'Reviewed pull requests are current',
      detail: 'No reviewed pull request in this complete scan has changed since its latest eligible checkpoint.',
      tone: 'current',
    }
  }
  if (session.summary.status === 'insufficient_evidence') {
    return {
      icon: <AlertTriangle size={22} />,
      title: 'No safe resume point',
      detail: 'StrataDiff found reviewed pull requests it could not observe completely. They remain listed below for manual follow-up.',
      tone: 'warning',
    }
  }
  if (session.summary.status === 'partial') {
    return {
      icon: <AlertTriangle size={22} />,
      title: 'No safe continuation in the inspected results',
      detail: 'The scan ended before the complete queue was observed. This is not a clean-inbox result.',
      tone: 'warning',
    }
  }
  return {
    icon: <Inbox size={22} />,
    title: 'No completed review checkpoints found',
    detail: 'The complete scan found no eligible completed review from which to continue.',
    tone: 'neutral',
  }
}

export function ReviewInboxWorkbench({ session }: { session: ReviewInboxSessionPayload }) {
  const [submittingEvent, setSubmittingEvent] = useState<string | null>(null)
  const [acceptedEvent, setAcceptedEvent] = useState<string | null>(null)
  const [failure, setFailure] = useState<{ eventId: string; message: string } | null>(null)
  const empty = emptyState(session)
  const provider = new URL(session.scope.provider_url).host
  const actionableCount = session.actionable.length

  async function continueReview(eventId: string): Promise<void> {
    if (submittingEvent !== null || acceptedEvent !== null) return
    setFailure(null)
    setSubmittingEvent(eventId)
    try {
      await continueInboxReview(window.location.search, eventId)
      setAcceptedEvent(eventId)
    } catch (reason) {
      setFailure({
        eventId,
        message: reason instanceof Error ? reason.message : 'The local viewer could not start review revalidation.',
      })
    } finally {
      setSubmittingEvent(null)
    }
  }

  return (
    <div className="inbox-workbench-shell">
      <header className="inbox-header">
        <div className="brand-block">
          <div className="brand-mark" aria-hidden="true"><span /><span /><span /></div>
          <div>
            <span className="eyebrow">STRATADIFF</span>
            <div className="brand-title">Review Inbox</div>
          </div>
        </div>
        <div className="inbox-scope" title={session.scope.repository ?? 'All accessible repositories'}>
          <span>{session.scope.repository ?? 'All repositories'}</span>
          <code>@{session.scope.reviewer_login}</code>
        </div>
        <div className={`inbox-scan-chip ${session.collection.status}`}>
          {session.collection.status === 'partial' ? <AlertTriangle size={13} /> : <ShieldCheck size={13} />}
          {session.collection.status === 'partial' ? 'Partial scan' : 'Complete scan'}
        </div>
      </header>

      <main className="inbox-main">
        <section className="inbox-hero" aria-labelledby="inbox-heading">
          <div className="inbox-hero-copy">
            <span className="surface-kicker">REVIEW CONTINUITY QUEUE</span>
            <h1 id="inbox-heading">
              {actionableCount === 0
                ? 'No pull request is ready to resume'
                : `${actionableCount} pull request${actionableCount === 1 ? '' : 's'} ready to resume`}
            </h1>
            <p>
              Continue from a review checkpoint only after StrataDiff revalidates the bound pull request, reviewer, review, base, head, and review request.
            </p>
          </div>
          <dl className="inbox-stats" aria-label="Review Inbox summary">
            <div><dt>Ready</dt><dd>{session.summary.resume_available_prs}</dd></div>
            <div><dt>Current</dt><dd>{session.summary.up_to_date_prs}</dd></div>
            <div><dt>Unobservable</dt><dd>{session.summary.unobservable_review_prs}</dd></div>
            <div><dt>Inspected</dt><dd>{session.collection.inspected_candidates}<small> / {session.collection.search_candidates}</small></dd></div>
          </dl>
        </section>

        {session.collection.status === 'partial' && (
          <section className="inbox-notice partial" role="status">
            <AlertTriangle size={17} />
            <div>
              <strong>Partial queue — not a clean result</strong>
              <p>Only the entries below were individually revalidated. More reviewed pull requests may exist outside this bounded scan.</p>
            </div>
          </section>
        )}

        {acceptedEvent !== null && (
          <section className="inbox-notice accepted" role="status">
            <LoaderCircle className="spin" size={17} />
            <div>
              <strong>Continue accepted</strong>
              <p>StrataDiff is revalidating live GitHub state and will open the review Workbench when the evidence is still valid.</p>
            </div>
          </section>
        )}

        <div className="inbox-content-grid">
          <section className="inbox-queue" aria-labelledby="ready-heading">
            <div className="inbox-section-heading">
              <div>
                <span className="surface-kicker">ACTIONABLE</span>
                <h2 id="ready-heading">Continue a review</h2>
              </div>
              <span>{actionableCount}</span>
            </div>

            {session.actionable.length === 0 ? (
              <div className={`inbox-empty ${empty.tone}`}>
                {empty.icon}
                <div><h3>{empty.title}</h3><p>{empty.detail}</p></div>
              </div>
            ) : (
              <div className="inbox-action-list">
                {session.actionable.map((item) => {
                  const submitting = submittingEvent === item.event_id
                  const accepted = acceptedEvent === item.event_id
                  const disabled = submittingEvent !== null || acceptedEvent !== null
                  return (
                    <article className="inbox-action-card" key={item.event_id}>
                      <div className="inbox-action-main">
                        <div className="inbox-pr-icon" aria-hidden="true"><GitPullRequest size={18} /></div>
                        <div className="inbox-action-copy">
                          <div className="inbox-pr-heading">
                            <a href={item.url} target="_blank" rel="noreferrer">
                              {item.repository} <strong>#{item.number}</strong><ExternalLink size={12} />
                            </a>
                            {item.is_draft && <span className="inbox-label">Draft</span>}
                            {item.review_request_active && <span className="inbox-label requested">Re-requested</span>}
                          </div>
                          <p>
                            Prior review: <strong>{item.checkpoint.review_state === 'approved' ? 'approved' : 'changes requested'}</strong>
                            <span aria-hidden="true"> · </span>
                            Updated <time dateTime={item.updated_at}>{formatTimestamp(item.updated_at)}</time>
                          </p>
                          <div className="inbox-commit-route" aria-label={`Checkpoint ${item.checkpoint.commit_id} to head ${item.head_oid}`}>
                            <code title={item.checkpoint.commit_id}>{shortOid(item.checkpoint.commit_id)}</code>
                            <ArrowRight size={13} />
                            <code title={item.head_oid}>{shortOid(item.head_oid)}</code>
                            <span>head changed</span>
                          </div>
                        </div>
                      </div>
                      <div className="inbox-action-controls">
                        <button
                          type="button"
                          disabled={disabled}
                          onClick={() => void continueReview(item.event_id)}
                          aria-label={`Continue review for ${item.repository} pull request ${item.number}`}
                        >
                          {submitting ? <LoaderCircle className="spin" size={15} /> : accepted ? <CheckCircle2 size={15} /> : <RotateCcw size={15} />}
                          {submitting ? 'Revalidating…' : accepted ? 'Accepted' : 'Continue review'}
                        </button>
                        {failure?.eventId === item.event_id && <p className="inbox-action-error" role="alert">{failure.message}</p>}
                      </div>
                    </article>
                  )
                })}
              </div>
            )}
          </section>

          <aside className="inbox-context" aria-label="Inbox context">
            <section>
              <h2><LockKeyhole size={15} /> Local handoff</h2>
              <p>The browser sends only the selected event identifier to this one-time local session. Resume credentials stay in the local process.</p>
            </section>
            <section>
              <h2><ShieldCheck size={15} /> Collection boundary</h2>
              <dl>
                <div><dt>Provider</dt><dd>{provider}</dd></div>
                <div><dt>Observed</dt><dd><time>{formatObservedAt(session.observed_at_unix_seconds)}</time></dd></div>
                <div><dt>Source code</dt><dd>Not collected</dd></div>
                <div><dt>PR text</dt><dd>Not collected</dd></div>
              </dl>
              <p>Continue does not submit, restore, or imply a GitHub approval.</p>
            </section>
          </aside>
        </div>

        {session.unobservable.length > 0 && (
          <section className="inbox-unobservable" aria-labelledby="unobservable-heading">
            <div className="inbox-section-heading">
              <div>
                <span className="surface-kicker">MANUAL FOLLOW-UP</span>
                <h2 id="unobservable-heading">Could not establish a safe checkpoint</h2>
              </div>
              <span>{session.unobservable.length}</span>
            </div>
            <div className="inbox-unobservable-list">
              {session.unobservable.map((item) => (
                <article key={`${item.repository}#${item.number}`}>
                  <AlertTriangle size={15} />
                  <div>
                    <a href={item.url} target="_blank" rel="noreferrer">{item.repository} <strong>#{item.number}</strong><ExternalLink size={11} /></a>
                    <p>{unobservableCopy[item.reason]}</p>
                  </div>
                  <time dateTime={item.updated_at}>{formatTimestamp(item.updated_at)}</time>
                </article>
              ))}
            </div>
          </section>
        )}
      </main>
    </div>
  )
}

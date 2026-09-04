import {
  AlertTriangle,
  ArrowLeftRight,
  Check,
  ChevronLeft,
  ChevronRight,
  CircleDot,
  Download,
  Eye,
  FileCode2,
  FileSearch,
  GitCompareArrows,
  LoaderCircle,
  Search,
  ShieldAlert,
  X,
} from 'lucide-react'
import { MultiFileDiff } from '@pierre/diffs/react'
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { useMediaQuery } from '../hooks/useMediaQuery'
import { compactNumber, formatBytes, shortHash, titleCase } from '../lib/format'
import { fetchReviewFileSources } from '../lib/session'
import { visibleInlineText, visibleSourceText } from '../lib/visibleText'
import type { DecodedArtifact, RepositorySessionPayload, ReviewFile } from '../types'

type ReviewScope = 'resume' | 'full'
type FullFilter = 'needs' | 'carried' | 'all'
type SourceState =
  | { status: 'idle' | 'loading' }
  | { status: 'ready'; before: DecodedArtifact; after: DecodedArtifact }
  | { status: 'error'; message: string }

const PAGE_SIZE = 100
const MAX_INTERACTIVE_BYTES_PER_SIDE = 2 * 1024 * 1024
const MAX_INTERACTIVE_LINES_PER_SIDE = 5_000
const MAX_VISIBLE_SOURCE_CHARACTERS = 8 * 1024 * 1024

function displayPath(file: ReviewFile): string {
  if (file.before_path !== undefined && file.after_path !== undefined && file.before_path !== file.after_path) {
    return `${visibleInlineText(file.before_path)} → ${visibleInlineText(file.after_path)}`
  }
  return visibleInlineText(file.after_path ?? file.before_path ?? '<unknown>')
}

function lineSummary(file: ReviewFile): string {
  const lines = file.line_change_envelope
  return lines === undefined ? 'line count unavailable' : `+${lines.additions.toLocaleString('en')} −${lines.deletions.toLocaleString('en')}`
}

function checkpointStateLabel(file: ReviewFile): string | null {
  if (file.checkpoint_state === 'needs_review_now') return 'Needs review now'
  if (file.checkpoint_match_basis === 'exact_noninteracting_four_way_byte_replay') return 'Four-way carry'
  if (file.checkpoint_match_basis === 'exact_git_change_identity') return 'Exact-identity carry'
  return null
}

function checkpointCarryDetail(file: ReviewFile, scope: ReviewScope, resumeComparison: RepositorySessionPayload['resume_delta']['comparison']): string {
  if (file.checkpoint_match_basis === 'exact_noninteracting_four_way_byte_replay') return 'four-way carry'
  if (file.checkpoint_match_basis === 'exact_git_change_identity') return 'exact-identity carry'
  if (file.checkpoint_state === 'needs_review_now') return 'not carried'
  if (scope === 'resume' && resumeComparison === 'snapshot_to_snapshot') return 'changed after checkpoint'
  return 'not evaluated'
}

function exportSession(session: RepositorySessionPayload): void {
  const blob = new Blob([JSON.stringify(session)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = 'stratadiff-review-session-v1.json'
  anchor.click()
  URL.revokeObjectURL(url)
}

function lineCount(bytes: Uint8Array): number {
  let count = 1
  for (const byte of bytes) if (byte === 0x0a) count += 1
  return count
}

function escapedPreview(bytes: Uint8Array): string {
  return Array.from(bytes.slice(0, 512), (byte) => {
    if (byte === 0x0a) return '\n\n'
    if (byte === 0x0d) return '\r'
    if (byte === 0x09) return '\t'
    if (byte >= 0x20 && byte <= 0x7e) return String.fromCharCode(byte)
    return `\\x${byte.toString(16).padStart(2, '0')}`
  }).join('')
}

function SourceOnlyDiff({ before, after }: { before: DecodedArtifact; after: DecodedArtifact }) {
  const tooLarge = before.bytes.byteLength > MAX_INTERACTIVE_BYTES_PER_SIDE ||
    after.bytes.byteLength > MAX_INTERACTIVE_BYTES_PER_SIDE ||
    lineCount(before.bytes) > MAX_INTERACTIVE_LINES_PER_SIDE ||
    lineCount(after.bytes) > MAX_INTERACTIVE_LINES_PER_SIDE
  const preparedBefore = before.text === null || tooLarge ? null : visibleSourceText(before.text, MAX_VISIBLE_SOURCE_CHARACTERS)
  const preparedAfter = after.text === null || tooLarge ? null : visibleSourceText(after.text, MAX_VISIBLE_SOURCE_CHARACTERS)

  if (preparedBefore === null || preparedAfter === null || preparedBefore.text === null || preparedAfter.text === null) {
    return (
      <div className="review-source-fallback">
        <ShieldAlert size={22} />
        <h3>Bounded source preview</h3>
        <p>{tooLarge ? 'This file is too large for the interactive renderer.' : 'At least one side is not valid UTF-8.'} Exact Git objects remain the source of truth.</p>
        <div className="review-binary-panes">
          <pre>{escapedPreview(before.bytes)}</pre>
          <pre>{escapedPreview(after.bytes)}</pre>
        </div>
      </div>
    )
  }

  return (
    <div className="review-raw-diff" data-testid="review-file-diff">
      <MultiFileDiff
        oldFile={{ name: before.path, contents: preparedBefore.text }}
        newFile={{ name: after.path, contents: preparedAfter.text }}
        options={{
          diffStyle: 'split',
          diffIndicators: 'bars',
          lineDiffType: 'char',
          enableLineSelection: false,
          expandUnchanged: false,
          disableFileHeader: true,
          overflow: 'scroll',
          lineHoverHighlight: 'line',
          themeType: 'dark',
          theme: 'github-dark-default',
        }}
      />
    </div>
  )
}

function ReviewFileDetail({ file, scope, resumeComparison, index, sources, drawer, open, onClose }: {
  file: ReviewFile
  scope: ReviewScope
  resumeComparison: RepositorySessionPayload['resume_delta']['comparison']
  index: number
  sources: SourceState
  drawer: boolean
  open: boolean
  onClose: () => void
}) {
  const inspectorRef = useRef<HTMLElement>(null)
  const closeRef = useRef<HTMLButtonElement>(null)

  useLayoutEffect(() => {
    if (!drawer || !open) return
    closeRef.current?.focus()
  }, [drawer, open])

  function handleKeyDown(event: React.KeyboardEvent<HTMLElement>): void {
    if (!drawer || !open) return
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      onClose()
      return
    }
    if (event.key !== 'Tab') return
    const focusable = Array.from(inspectorRef.current?.querySelectorAll<HTMLElement>('button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])') ?? [])
    const first = focusable[0]
    const last = focusable[focusable.length - 1]
    if (first === undefined || last === undefined) return
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault()
      last.focus()
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault()
      first.focus()
    }
  }

  const openEvidence = (): void => {
    const params = new URLSearchParams(window.location.search)
    params.set('file', String(index))
    params.set('scope', scope)
    window.location.assign(`/?${params.toString()}`)
  }
  const fields = [
    ['Before object', file.before_blob],
    ['After object', file.after_blob],
    ['Before mode', file.before_mode],
    ['After mode', file.after_mode],
  ] as const

  return (
    <aside
      ref={inspectorRef}
      className={`review-inspector ${open ? 'open' : ''}`}
      aria-label="Selected file details"
      aria-hidden={drawer && !open}
      aria-modal={drawer && open ? true : undefined}
      inert={drawer && !open}
      role={drawer ? 'dialog' : undefined}
      onKeyDown={handleKeyDown}
    >
      <div className="review-inspector-heading">
        <div>
          <span className="eyebrow">SELECTED CHANGE</span>
          <h2>{displayPath(file)}</h2>
          <div className="review-inspector-badges">
            <span>{titleCase(file.status)}</span>
            <span>{titleCase(file.lane)}</span>
          </div>
        </div>
        <button ref={closeRef} className="review-inspector-close" type="button" onClick={onClose} aria-label="Close details and evidence"><X size={17} /></button>
      </div>
      <section>
        <h3><Eye size={14} /> Why it is here</h3>
        <p>{visibleInlineText(file.reason)}</p>
        <p className="review-scope-note">
          {scope === 'resume'
            ? resumeComparison === 'snapshot_to_snapshot'
              ? 'This row compares the declared checkpoint snapshot directly with the current head.'
              : 'This row is a current PR change carried by neither exact identity nor non-interacting four-way replay; upstream-only files are excluded.'
            : file.checkpoint_state === 'unchanged_since_checkpoint'
              ? file.checkpoint_match_basis === 'exact_noninteracting_four_way_byte_replay'
                ? 'The reviewed byte edits and upstream base edits were non-interacting, and both replay orders reproduced this exact current blob.'
                : 'This complete PR change identity exactly matches the declared checkpoint.'
              : 'This current PR change identity was not carried from the checkpoint.'}
        </p>
      </section>
      {file.evidence !== undefined && (
        <section>
          <h3><Check size={14} /> File evidence</h3>
          <dl>
            <div><dt>Checkpoint carry</dt><dd>{checkpointCarryDetail(file, scope, resumeComparison)}</dd></div>
            <div><dt>Diff reconstruction</dt><dd>{file.evidence.replay_check_passed_during_analysis ? 'target matched' : 'not matched'}</dd></div>
            <div><dt>Byte edits</dt><dd>{file.evidence.byte_edits}</dd></div>
            <div><dt>Ambiguities</dt><dd>{file.evidence.ambiguity_groups}</dd></div>
            <div><dt>Report</dt><dd title={file.evidence.report_blake3}>{shortHash(file.evidence.report_blake3)}</dd></div>
          </dl>
          <button className="review-evidence-button" type="button" disabled={sources.status !== 'ready'} onClick={openEvidence}>
            <FileSearch size={15} /> Inspect verified evidence
          </button>
        </section>
      )}
      <section>
        <h3><GitCompareArrows size={14} /> Git identity</h3>
        <dl>
          {fields.map(([label, value]) => (
            <div key={label}><dt>{label}</dt><dd title={value}>{value === undefined ? 'absent' : shortHash(value)}</dd></div>
          ))}
          <div><dt>Language</dt><dd>{file.language ?? 'not analyzed'}</dd></div>
          <div><dt>Size</dt><dd>{file.before_bytes === undefined ? '—' : formatBytes(file.before_bytes)} → {file.after_bytes === undefined ? '—' : formatBytes(file.after_bytes)}</dd></div>
        </dl>
      </section>
      <div className="review-nonclaim"><AlertTriangle size={14} /> No row establishes approval, semantic safety, or absence of cross-file effects.</div>
    </aside>
  )
}

export function ReviewWorkbench({ session }: { session: RepositorySessionPayload }) {
  const checkpoint = session.review.checkpoint
  const checkpointSummary = session.review.summary.checkpoint
  if (checkpoint === undefined || checkpointSummary === undefined) throw new Error('Review Resume requires checkpoint metadata.')

  const [scope, setScope] = useState<ReviewScope>('resume')
  const [fullFilter, setFullFilter] = useState<FullFilter>('needs')
  const [query, setQuery] = useState('')
  const [selectedIndex, setSelectedIndex] = useState<number | null>(session.resume_delta.files.length === 0 ? null : 0)
  const [page, setPage] = useState(0)
  const [sources, setSources] = useState<SourceState>({ status: 'idle' })
  const [detailsOpen, setDetailsOpen] = useState(false)
  const searchRef = useRef<HTMLInputElement>(null)
  const detailsButtonRef = useRef<HTMLButtonElement>(null)
  const detailsAreDrawer = useMediaQuery('(max-width: 1279px)')
  const baseChanged = session.resume_delta.comparison === 'current_pr_unmatched_identities'
  const exactIdentityCarries = session.review.files.filter((file) => file.checkpoint_match_basis === 'exact_git_change_identity').length
  const fourWayReplayCarries = session.review.files.filter((file) => file.checkpoint_match_basis === 'exact_noninteracting_four_way_byte_replay').length
  const files = scope === 'resume' ? session.resume_delta.files : session.review.files
  const entries = useMemo(() => files.map((file, index) => ({ file, index })).filter(({ file }) => {
    if (scope === 'full' && fullFilter !== 'all') {
      const wanted = fullFilter === 'needs' ? 'needs_review_now' : 'unchanged_since_checkpoint'
      if (file.checkpoint_state !== wanted) return false
    }
    const needle = query.trim().toLocaleLowerCase()
    return needle.length === 0 || displayPath(file).toLocaleLowerCase().includes(needle)
  }), [files, fullFilter, query, scope])
  const pageCount = Math.max(1, Math.ceil(entries.length / PAGE_SIZE))
  const safePage = Math.min(page, pageCount - 1)
  const visibleEntries = entries.slice(safePage * PAGE_SIZE, (safePage + 1) * PAGE_SIZE)
  const selectedEntry = entries.find(({ index }) => index === selectedIndex)
  const selectedFile = selectedEntry?.file

  function closeDetails(): void {
    if (detailsOpen) detailsButtonRef.current?.focus()
    setDetailsOpen(false)
  }

  useLayoutEffect(() => {
    setPage(0)
    setSelectedIndex(entries[0]?.index ?? null)
    setDetailsOpen(false)
  }, [scope, fullFilter, query])

  useEffect(() => {
    if (selectedFile === undefined || selectedIndex === null) {
      setSources({ status: 'idle' })
      return
    }
    const controller = new AbortController()
    setSources({ status: 'loading' })
    fetchReviewFileSources(window.location.search, selectedIndex, scope, selectedFile, controller.signal)
      .then((value) => setSources({ status: 'ready', ...value }))
      .catch((reason: unknown) => {
        if (reason instanceof DOMException && reason.name === 'AbortError') return
        setSources({ status: 'error', message: reason instanceof Error ? reason.message : 'Source materialization failed.' })
      })
    return () => controller.abort()
  }, [scope, selectedFile, selectedIndex])

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent): void {
      const target = event.target
      const typing = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement
      if (event.metaKey || event.ctrlKey || event.altKey) return
      if (detailsAreDrawer && detailsOpen) {
        if (event.key === 'Escape') closeDetails()
        return
      }
      if (!typing && event.key === '/') {
        event.preventDefault()
        searchRef.current?.focus()
        return
      }
      if (typing || (event.key !== 'j' && event.key !== 'k') || entries.length === 0) return
      event.preventDefault()
      const selectedPosition = entries.findIndex(({ index }) => index === selectedIndex)
      const nextPosition = event.key === 'j'
        ? Math.min(entries.length - 1, selectedPosition + 1)
        : Math.max(0, selectedPosition <= 0 ? 0 : selectedPosition - 1)
      const next = entries[nextPosition]
      if (next !== undefined) {
        setSelectedIndex(next.index)
        setPage(Math.floor(nextPosition / PAGE_SIZE))
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [detailsAreDrawer, detailsOpen, entries, selectedIndex])

  return (
    <div className="review-shell">
      <header className="review-header">
        <div className="brand-block">
          <div className="brand-mark" aria-hidden="true"><span /><span /><span /></div>
          <div><div className="eyebrow">STRATADIFF</div><div className="brand-title">Review Resume</div></div>
        </div>
        <div className="review-range" title={`${checkpoint.commit} → ${session.review.head_commit}`}>
          <span>{shortHash(checkpoint.commit)}</span><ArrowLeftRight size={14} /><span>{shortHash(session.review.head_commit)}</span>
        </div>
        <div className="review-header-actions">
          <span className="attested-chip"><CircleDot size={13} /> {baseChanged ? 'Exact identity + four-way carry' : 'Exact identity'} · caller-attested checkpoint</span>
          <button className="export-button" type="button" onClick={() => exportSession(session)}><Download size={15} /> Export session</button>
        </div>
      </header>

      <section className="resume-hero">
        <div className="resume-copy">
          <span className="eyebrow">{baseChanged ? 'PR-RELATIVE RESIDUE AFTER BASE CHANGE' : 'WHAT CHANGED AFTER YOUR CHECKPOINT'}</span>
          <h1>{session.resume_delta.files.length === 0
            ? baseChanged ? 'No review residue' : 'No changes between checkpoint and head'
            : baseChanged
              ? `${session.resume_delta.files.length.toLocaleString('en')} current PR ${session.resume_delta.files.length === 1 ? 'file needs' : 'files need'} review`
              : `${session.resume_delta.files.length.toLocaleString('en')} ${session.resume_delta.files.length === 1 ? 'file' : 'files'} changed since checkpoint`}</h1>
          <p>{baseChanged
            ? 'The merge base moved. This queue excludes upstream-only files and keeps changes that failed exact identity and non-interacting four-way replay.'
            : 'Start with the checkpoint → head delta. Switch to full PR context whenever you need the original base → head story.'}</p>
        </div>
        <div className="resume-stats" aria-label="Review resume summary">
          <div className="attention"><span>Need review now</span><strong>{compactNumber(checkpointSummary.needs_review_now_files)}</strong></div>
          <div className="carried"><span>Exact-identity carry</span><strong>{compactNumber(exactIdentityCarries)}</strong></div>
          <div className="carried"><span>Four-way carry</span><strong>{compactNumber(fourWayReplayCarries)}</strong></div>
          <div className="retired"><span>Retired identities</span><strong>{compactNumber(checkpointSummary.retired_change_count)}</strong></div>
        </div>
      </section>

      <div className="review-scope-bar">
        <div className="review-scope-switch" aria-label="Diff scope">
          <button type="button" className={scope === 'resume' ? 'active' : ''} onClick={() => setScope('resume')}>{baseChanged ? 'Review residue' : 'Since checkpoint'} <small>{baseChanged ? "B' → H" : 'R → H'}</small></button>
          <button type="button" className={scope === 'full' ? 'active' : ''} onClick={() => setScope('full')}>Full PR context <small>B → H</small></button>
        </div>
        <p><ShieldAlert size={14} /> {session.assessment.message}</p>
      </div>
      <div className="review-trust-banner"><ShieldAlert size={13} /> {baseChanged ? 'Base changed · exact-identity or four-way carry · unresolved changes need review' : 'Exact Git identity only · caller-attested checkpoint · not approval or semantic safety'}</div>

      <div className="review-workspace">
        <aside className="review-file-panel">
          <div className="review-file-tools">
            <label><Search size={14} /><span className="sr-only">Search files</span><input ref={searchRef} value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search changed files" /></label>
            {scope === 'full' && (
              <div className="review-file-filters" aria-label="Checkpoint state filter">
                {(['needs', 'carried', 'all'] as const).map((filter) => <button type="button" className={fullFilter === filter ? 'active' : ''} key={filter} onClick={() => setFullFilter(filter)}>{filter === 'needs' ? 'Needs now' : filter === 'carried' ? 'Carried' : 'All'}</button>)}
              </div>
            )}
          </div>
          <div className="review-list-heading"><span>{scope === 'resume' ? baseChanged ? 'Review residue' : 'Incremental queue' : 'Current PR files'}</span><strong>{entries.length}</strong></div>
          <div className="review-file-list">
            {visibleEntries.map(({ file, index }) => (
              <button type="button" className={`review-file-row ${selectedIndex === index ? 'selected' : ''}`} key={`${scope}-${index}`} onClick={() => setSelectedIndex(index)}>
                <span aria-hidden="true" className={`review-state-dot ${file.checkpoint_state === 'unchanged_since_checkpoint' ? 'carried' : 'attention'}`} />
                <span className="review-file-copy"><strong>{displayPath(file)}</strong><small>{titleCase(file.status)} · {titleCase(file.lane)}{checkpointStateLabel(file) === null ? '' : ` · ${checkpointStateLabel(file)}`}</small></span>
                <span className="review-line-count">{lineSummary(file)}</span>
              </button>
            ))}
            {entries.length === 0 && <div className="review-empty"><Check size={22} /><strong>Nothing in this view</strong><span>Try another scope or clear the file search.</span></div>}
          </div>
          {pageCount > 1 && <div className="review-pagination"><button type="button" disabled={safePage === 0} onClick={() => setPage(safePage - 1)} aria-label="Previous file page"><ChevronLeft size={14} /></button><span>{safePage + 1} / {pageCount}</span><button type="button" disabled={safePage === pageCount - 1} onClick={() => setPage(safePage + 1)} aria-label="Next file page"><ChevronRight size={14} /></button></div>}
        </aside>

        <main className="review-diff-panel">
          {selectedFile === undefined ? (
            entries.length === 0 && files.length > 0 ? (
              <div className="review-empty large"><FileSearch size={28} /><strong>No files match this view</strong><span>Clear the search or choose another checkpoint-state filter.</span></div>
            ) : scope === 'resume' ? baseChanged ? (
              <div className="review-empty large"><Check size={28} /><strong>No review residue</strong><span>Every current PR change was carried by exact identity or non-interacting four-way replay.</span></div>
            ) : (
              <div className="review-empty large"><Check size={28} /><strong>No changes between checkpoint and head</strong><span>There is no incremental file delta to inspect.</span></div>
            ) : (
              <div className="review-empty large"><Check size={28} /><strong>No current PR changes</strong><span>The merge-base and head snapshots have no file delta.</span></div>
            )
          ) : (
            <>
              <div className="review-diff-heading">
                <div><span className="surface-kicker">{scope === 'resume' ? baseChanged ? 'CURRENT PR · NEEDS REVIEW' : 'CHECKPOINT → HEAD' : 'MERGE BASE → HEAD'}</span><h2>{displayPath(selectedFile)}</h2></div>
                <div className="review-diff-actions">
                  <span>{lineSummary(selectedFile)}</span>
                  <button ref={detailsButtonRef} type="button" className="review-detail-trigger" onClick={() => setDetailsOpen(true)} aria-label="Open details and evidence"><Eye size={14} /> Details &amp; evidence</button>
                </div>
              </div>
              {sources.status === 'loading' && <div className="review-source-state"><LoaderCircle className="spin" size={22} /> Loading immutable Git objects…</div>}
              {sources.status === 'error' && <div className="review-source-state error"><ShieldAlert size={22} /><strong>Source preview unavailable</strong><span>{sources.message}</span></div>}
              {sources.status === 'ready' && <SourceOnlyDiff before={sources.before} after={sources.after} />}
            </>
          )}
        </main>

        {detailsAreDrawer && detailsOpen && <button type="button" className="drawer-backdrop review-details-backdrop" onClick={closeDetails} aria-label="Close details and evidence" />}
        {selectedFile !== undefined && <ReviewFileDetail file={selectedFile} scope={scope} resumeComparison={session.resume_delta.comparison} index={selectedIndex ?? 0} sources={sources} drawer={detailsAreDrawer} open={detailsOpen} onClose={closeDetails} />}
      </div>
      <footer className="review-footer"><FileCode2 size={13} /> review-v1 is a producer-attested focus summary; only opened per-file reports receive independent structural verification.</footer>
    </div>
  )
}

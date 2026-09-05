import { ArrowLeft, Check, Download, FileCode2, PanelRightOpen, ShieldCheck } from 'lucide-react'
import type { Ref } from 'react'
import type { LoadedFileSession } from '../types'
import { visibleInlineText } from '../lib/visibleText'

interface HeaderProps {
  session: LoadedFileSession
  onOpenInspector: () => void
  inspectorButtonRef: Ref<HTMLButtonElement>
}

function fileName(path: string): string {
  const normalized = path.replaceAll('\\', '/')
  return visibleInlineText(normalized.slice(normalized.lastIndexOf('/') + 1))
}

function exportReport(session: LoadedFileSession): void {
  const blob = new Blob([JSON.stringify(session.report)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = 'stratadiff-report-v3.json'
  anchor.click()
  URL.revokeObjectURL(url)
}

function reviewQueueUrl(): string {
  const params = new URLSearchParams(window.location.search)
  params.delete('file')
  return `/?${params.toString()}`
}

function reviewCoverage(session: LoadedFileSession): { label: string; tone: 'carried' | 'needs' | 'changed' } | null {
  const context = session.repository_context
  if (context === undefined) return null
  if (context.scope === 'base') {
    return { label: 'Base drift context · not PR coverage', tone: 'changed' }
  }
  if (context.checkpoint_match_basis === 'exact_git_change_identity') {
    return { label: 'Coverage: exact-identity carry', tone: 'carried' }
  }
  if (context.checkpoint_match_basis === 'exact_noninteracting_four_way_byte_replay') {
    return { label: 'Coverage: four-way carry', tone: 'carried' }
  }
  if (context.checkpoint_state === 'needs_review_now') {
    return { label: 'Coverage: needs review', tone: 'needs' }
  }
  return { label: 'Coverage: changed after checkpoint', tone: 'changed' }
}

export function Header({ session, onOpenInspector, inspectorButtonRef }: HeaderProps) {
  const { report } = session
  const coverage = reviewCoverage(session)
  const beforePath = visibleInlineText(report.before.path)
  const afterPath = visibleInlineText(report.after.path)
  return (
    <header className="app-header">
      <div className="brand-block">
        <div className="brand-mark" aria-hidden="true">
          <span />
          <span />
          <span />
        </div>
        <div>
          <div className="eyebrow">STRATADIFF</div>
          <div className="brand-title">Evidence Workbench</div>
        </div>
      </div>

      <div className="file-pair" title={`${beforePath} → ${afterPath}`}>
        <FileCode2 size={16} aria-hidden="true" />
        <span className="file-name old-file">{fileName(session.report.before.path)}</span>
        <span className="path-arrow" aria-hidden="true">→</span>
        <span className="file-name new-file">{fileName(session.report.after.path)}</span>
      </div>

      <div className="header-meta" aria-label="Report metadata">
        <span className="meta-chip language-chip">{report.parser.language}</span>
        <span className="meta-chip">report v3</span>
        {coverage !== null && <span className={`meta-chip review-coverage-chip ${coverage.tone}`}>{coverage.label}</span>}
        <span className="verified-chip" title={session.verification.message}>
          <ShieldCheck size={14} aria-hidden="true" />
          <span>Diff evidence verified</span>
          <Check size={12} aria-hidden="true" />
        </span>
      </div>

      <div className="header-actions">
        {session.repository_context !== undefined && (
          <a className="export-button" href={reviewQueueUrl()}>
            <ArrowLeft size={15} aria-hidden="true" /> Back to queue
          </a>
        )}
        <button ref={inspectorButtonRef} className="icon-button inspector-toggle" type="button" onClick={onOpenInspector} aria-label="Open evidence inspector">
          <PanelRightOpen size={17} />
        </button>
        <button className="export-button" type="button" onClick={() => exportReport(session)}>
          <Download size={15} aria-hidden="true" />
          Export report
        </button>
      </div>
    </header>
  )
}

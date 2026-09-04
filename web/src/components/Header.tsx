import { Check, Download, FileCode2, PanelRightOpen, ShieldCheck } from 'lucide-react'
import type { LoadedSession } from '../types'
import { visibleInlineText } from '../lib/visibleText'

interface HeaderProps {
  session: LoadedSession
  onOpenInspector: () => void
}

function fileName(path: string): string {
  const normalized = path.replaceAll('\\', '/')
  return visibleInlineText(normalized.slice(normalized.lastIndexOf('/') + 1))
}

function exportReport(session: LoadedSession): void {
  const blob = new Blob([JSON.stringify(session.report)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = 'stratadiff-report-v3.json'
  anchor.click()
  URL.revokeObjectURL(url)
}

export function Header({ session, onOpenInspector }: HeaderProps) {
  const { report } = session
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
        <span className="verified-chip" title={session.verification.message}>
          <ShieldCheck size={14} aria-hidden="true" />
          <span>Verified</span>
          <Check size={12} aria-hidden="true" />
        </span>
      </div>

      <div className="header-actions">
        <button className="icon-button inspector-toggle" type="button" onClick={onOpenInspector} aria-label="Open evidence inspector">
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

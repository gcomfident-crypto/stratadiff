import { Binary, Check, ChevronLeft, ChevronRight, Clipboard, Fingerprint, Scissors } from 'lucide-react'
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { base64DecodedLength, base64Preview, byteRange, editAfterRanges, formatBytes } from '../lib/format'
import { evidenceByteRanges } from '../lib/selection'
import { visibleInlineText } from '../lib/visibleText'
import type { ByteEdit, DecodedArtifact, DiffReport, EvidenceSelection } from '../types'

const BYTES_PER_ROW = 16
const ROWS_PER_PAGE = 24
const BYTES_PER_PAGE = BYTES_PER_ROW * ROWS_PER_PAGE
const EDITS_PER_PAGE = 80

interface HexViewerProps {
  artifact: DecodedArtifact
  side: 'before' | 'after'
  highlight: [number, number] | null
}

function HexViewer({ artifact, side, highlight }: HexViewerProps) {
  const pageCount = Math.max(1, Math.ceil(artifact.bytes.byteLength / BYTES_PER_PAGE))
  const pageForHighlight = highlight === null ? 0 : Math.min(pageCount - 1, Math.floor(highlight[0] / BYTES_PER_PAGE))
  const [page, setPage] = useState(pageForHighlight)
  const visiblePath = visibleInlineText(artifact.path)

  useEffect(() => {
    if (highlight !== null) setPage(pageForHighlight)
  }, [highlight, pageForHighlight])

  const start = page * BYTES_PER_PAGE
  const end = Math.min(start + BYTES_PER_PAGE, artifact.bytes.byteLength)
  const rows = useMemo(() => {
    const output: Array<{ offset: number; bytes: number[] }> = []
    for (let offset = start; offset < end; offset += BYTES_PER_ROW) {
      output.push({ offset, bytes: Array.from(artifact.bytes.slice(offset, Math.min(offset + BYTES_PER_ROW, end))) })
    }
    return output
  }, [artifact.bytes, end, start])

  return (
    <div className={`hex-viewer ${side}`}>
      <div className="hex-header">
        <div><Binary size={16} /><strong>{side === 'before' ? 'Before bytes' : 'After bytes'}</strong></div>
        <span>{formatBytes(artifact.bytes.byteLength)}</span>
      </div>
      <div className="hex-path" title={visiblePath}>{visiblePath}</div>
      <div className="hex-columns" aria-hidden="true">
        <span>OFFSET</span><span>00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F</span><span>ASCII</span>
      </div>
      <div className="hex-rows">
        {rows.map((row) => (
          <div className="hex-row" key={row.offset}>
            <span className="hex-offset">{row.offset.toString(16).padStart(8, '0')}</span>
            <span className="hex-values">
              {row.bytes.map((byte, index) => {
                const offset = row.offset + index
                const active = highlight !== null && offset >= highlight[0] && offset < highlight[1]
                return <i className={active ? 'highlight' : ''} key={offset}>{byte.toString(16).padStart(2, '0')}</i>
              })}
            </span>
            <span className="ascii-values">
              {row.bytes.map((byte, index) => {
                const offset = row.offset + index
                const active = highlight !== null && offset >= highlight[0] && offset < highlight[1]
                return <i className={active ? 'highlight' : ''} key={offset}>{byte >= 32 && byte <= 126 ? String.fromCharCode(byte) : '·'}</i>
              })}
            </span>
          </div>
        ))}
        {rows.length === 0 && <div className="empty-bytes">Empty file · zero bytes</div>}
      </div>
      <div className="hex-pagination">
        <button type="button" disabled={page === 0} onClick={() => setPage(page - 1)} aria-label={`Previous ${side} byte page`}><ChevronLeft size={14} /></button>
        <span>{start}–{end} / {artifact.bytes.byteLength}</span>
        <button type="button" disabled={page >= pageCount - 1} onClick={() => setPage(page + 1)} aria-label={`Next ${side} byte page`}><ChevronRight size={14} /></button>
      </div>
    </div>
  )
}

function CopyBase64Button({ base64 }: { base64: string }) {
  const [copied, setCopied] = useState(false)
  async function copy(): Promise<void> {
    await navigator.clipboard.writeText(base64)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1_500)
  }
  return <button className="copy-button" type="button" onClick={copy}>{copied ? <Check size={13} /> : <Clipboard size={13} />}{copied ? 'Copied' : 'Copy Base64'}</button>
}

interface ByteViewProps {
  report: DiffReport
  before: DecodedArtifact
  after: DecodedArtifact
  selection: EvidenceSelection
  onSelect: (selection: EvidenceSelection) => void
}

function EditCard({ edit, index, afterRange, active, onSelect }: { edit: ByteEdit; index: number; afterRange: [number, number]; active: boolean; onSelect: () => void }) {
  const replacementLength = base64DecodedLength(edit.replacement_base64)
  const [afterStart, afterEnd] = afterRange
  return (
    <button className={`byte-edit-card ${active ? 'active' : ''}`} type="button" onClick={onSelect}>
      <span className="edit-sequence">{String(index + 1).padStart(2, '0')}</span>
      <div className="edit-range old-range"><small>OLD RANGE</small><strong>{byteRange(edit.old_start, edit.old_end)}</strong></div>
      <span className="edit-operation"><Scissors size={14} /><i>{edit.old_end - edit.old_start} → {replacementLength}</i></span>
      <div className="edit-range new-range"><small>REPLAY RANGE</small><strong>{byteRange(afterStart, afterEnd)}</strong></div>
    </button>
  )
}

export function ByteView({ report, before, after, selection, onSelect }: ByteViewProps) {
  const surfaceRef = useRef<HTMLDivElement>(null)
  const selectedIndex = selection.type === 'edit' ? selection.index : null
  const edit = selectedIndex === null ? null : report.patch.edits[selectedIndex] ?? null
  const afterRanges = useMemo(() => editAfterRanges(report.patch.edits), [report.patch.edits])
  const evidenceRanges = useMemo(
    () => evidenceByteRanges(report, selection, afterRanges),
    [afterRanges, report, selection],
  )
  const oldHighlight = evidenceRanges.before
  const afterHighlight = evidenceRanges.after
  const [editPage, setEditPage] = useState(selectedIndex === null ? 0 : Math.floor(selectedIndex / EDITS_PER_PAGE))
  const editPageCount = Math.max(1, Math.ceil(report.patch.edits.length / EDITS_PER_PAGE))
  const firstEdit = editPage * EDITS_PER_PAGE
  const visibleEdits = report.patch.edits.slice(firstEdit, firstEdit + EDITS_PER_PAGE)

  useEffect(() => {
    if (selectedIndex !== null) setEditPage(Math.floor(selectedIndex / EDITS_PER_PAGE))
  }, [selectedIndex])

  useLayoutEffect(() => {
    const active = surfaceRef.current?.querySelector<HTMLElement>('.byte-edit-card.active')
    if (active !== null && active !== undefined && typeof active.scrollIntoView === 'function') active.scrollIntoView({ block: 'nearest', inline: 'center' })
  }, [editPage, selectedIndex])

  return (
    <div ref={surfaceRef} className="surface byte-surface">
      <div className="surface-toolbar byte-toolbar">
        <div>
          <span className="surface-kicker">LOSSLESS REPLAY LAYER</span>
          <h2>Exact bytes</h2>
        </div>
        <div className="algorithm-chip"><Fingerprint size={14} /><span>{report.patch.algorithm}</span></div>
      </div>

      <div className="byte-facts">
        <div><small>BEFORE</small><strong>{formatBytes(before.bytes.byteLength)}</strong><code>{report.before.blake3}</code></div>
        <div className="replay-arrow"><span>{report.patch.edits.length} edit{report.patch.edits.length === 1 ? '' : 's'}</span><i /></div>
        <div><small>RECONSTRUCTED</small><strong>{formatBytes(after.bytes.byteLength)}</strong><code>{report.certificate.reconstructed_blake3}</code></div>
        <div className="certificate-stamp"><Check size={15} /><span>HASH MATCH</span></div>
      </div>

      <div className="byte-edit-strip">
        {visibleEdits.map((item, pageIndex) => {
          const index = firstEdit + pageIndex
          const afterRange = afterRanges[index]
          if (afterRange === undefined) throw new Error(`Byte edit ${index} has no replay range.`)
          return (
          <EditCard
            edit={item}
            index={index}
            afterRange={afterRange}
            active={selectedIndex === index}
            onSelect={() => onSelect({ type: 'edit', index })}
            key={`byte-edit-card-${index}`}
          />
          )
        })}
        {report.patch.edits.length === 0 && <div className="zero-edits"><Check size={15} />Identical byte streams; replay needs no edits.</div>}
      </div>
      {editPageCount > 1 && (
        <div className="byte-edit-pagination" aria-label="Byte edit pages">
          <button type="button" disabled={editPage === 0} onClick={() => setEditPage(editPage - 1)} aria-label="Previous byte edit page"><ChevronLeft size={14} /></button>
          <span>Edits {firstEdit + 1}–{Math.min(firstEdit + EDITS_PER_PAGE, report.patch.edits.length)} of {report.patch.edits.length}</span>
          <button type="button" disabled={editPage >= editPageCount - 1} onClick={() => setEditPage(editPage + 1)} aria-label="Next byte edit page"><ChevronRight size={14} /></button>
        </div>
      )}

      {edit !== null && (
        <div className="replacement-detail">
          <div><span className="eyebrow">SELECTED REPLACEMENT</span><strong>{base64DecodedLength(edit.replacement_base64)} exact bytes</strong></div>
          <code>{edit.replacement_base64.length === 0 ? '∅ (deletion)' : base64Preview(edit.replacement_base64)}</code>
          <CopyBase64Button base64={edit.replacement_base64} />
        </div>
      )}

      <div className="hex-grid">
        <HexViewer artifact={before} side="before" highlight={oldHighlight} />
        <HexViewer artifact={after} side="after" highlight={afterHighlight} />
      </div>
      <div className="claim-boundary">
        Offsets are bytes, never characters. Structural selections show covered ranges, not causality. Hex and Base64 remain lossless for NULs, invalid UTF-8, and mixed line endings.
      </div>
    </div>
  )
}

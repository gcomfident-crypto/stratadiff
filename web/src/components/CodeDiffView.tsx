import { AlertOctagon, Columns2, Rows3 } from 'lucide-react'
import { MultiFileDiff } from '@pierre/diffs/react'
import { useMemo } from 'react'
import type { DecodedArtifact, DiffReport, DiffStyle, EvidenceSelection } from '../types'
import { editAfterRanges, formatBytes } from '../lib/format'
import { buildByteLineIndex, evidenceByteRanges, evidenceLineSelection } from '../lib/selection'
import { visibleInlineText, visibleSourceText } from '../lib/visibleText'

const MAX_INTERACTIVE_BYTES_PER_SIDE = 2 * 1024 * 1024
const MAX_INTERACTIVE_LINES_PER_SIDE = 5_000
const MAX_VISIBLE_SOURCE_CHARACTERS = 8 * 1024 * 1024

interface CodeDiffViewProps {
  before: DecodedArtifact
  after: DecodedArtifact
  diffStyle: DiffStyle
  onDiffStyleChange: (style: DiffStyle) => void
  compact: boolean
  report: DiffReport
  selection: EvidenceSelection
}

function escapedPreview(bytes: Uint8Array): string {
  return Array.from(bytes.slice(0, 384), (byte) => {
    if (byte === 0x0a) return '\\n\n'
    if (byte === 0x0d) return '\\r'
    if (byte === 0x09) return '\\t'
    if (byte === 0x5c) return '\\\\'
    if (byte >= 0x20 && byte <= 0x7e) return String.fromCharCode(byte)
    return `\\x${byte.toString(16).padStart(2, '0')}`
  }).join('')
}

function NonUtf8Pane({ artifact, side }: { artifact: DecodedArtifact; side: 'before' | 'after' }) {
  return (
    <div className={`binary-code-pane ${side}`}>
      <div className="binary-pane-header">
        <strong>{side === 'before' ? 'Before' : 'After'}</strong>
        <span>{visibleInlineText(artifact.path)}</span>
        <small>{formatBytes(artifact.bytes.byteLength)}</small>
      </div>
      <pre>{escapedPreview(artifact.bytes)}</pre>
      {artifact.bytes.byteLength > 384 && <div className="preview-truncated">Preview limited to 384 bytes. Exact bytes remain available in view 3.</div>}
    </div>
  )
}

function exceedsLineLimit(bytes: Uint8Array): boolean {
  let lines = 1
  for (const byte of bytes) {
    if (byte === 0x0a) lines += 1
    if (lines > MAX_INTERACTIVE_LINES_PER_SIDE) return true
  }
  return false
}

export function CodeDiffView({ before, after, diffStyle, onDiffStyleChange, compact, report, selection }: CodeDiffViewProps) {
  const containsNonUtf8 = before.text === null || after.text === null
  const effectiveStyle: DiffStyle = compact ? 'unified' : diffStyle
  const sourceTooLarge = useMemo(
    () => before.bytes.byteLength > MAX_INTERACTIVE_BYTES_PER_SIDE || after.bytes.byteLength > MAX_INTERACTIVE_BYTES_PER_SIDE || exceedsLineLimit(before.bytes) || exceedsLineLimit(after.bytes),
    [after.bytes, before.bytes],
  )
  const preparedBefore = useMemo(
    () => before.text === null || sourceTooLarge ? null : visibleSourceText(before.text, MAX_VISIBLE_SOURCE_CHARACTERS),
    [before.text, sourceTooLarge],
  )
  const preparedAfter = useMemo(
    () => after.text === null || sourceTooLarge ? null : visibleSourceText(after.text, MAX_VISIBLE_SOURCE_CHARACTERS),
    [after.text, sourceTooLarge],
  )
  const visualizationTooLarge = preparedBefore?.text === null || preparedAfter?.text === null
  const renderInteractiveDiff = !containsNonUtf8 && !sourceTooLarge && !visualizationTooLarge
  const afterRanges = useMemo(() => renderInteractiveDiff && selection.type === 'edit' ? editAfterRanges(report.patch.edits) : [], [renderInteractiveDiff, report.patch.edits, selection.type])
  const beforeLineIndex = useMemo(() => renderInteractiveDiff ? buildByteLineIndex(before.bytes) : null, [before.bytes, renderInteractiveDiff])
  const afterLineIndex = useMemo(() => renderInteractiveDiff ? buildByteLineIndex(after.bytes) : null, [after.bytes, renderInteractiveDiff])
  const selectedLines = useMemo(() => {
    if (beforeLineIndex === null || afterLineIndex === null) return null
    const ranges = evidenceByteRanges(report, selection, afterRanges)
    return evidenceLineSelection(ranges, beforeLineIndex, afterLineIndex)
  }, [afterLineIndex, afterRanges, beforeLineIndex, report, selection])
  const oldFile = useMemo(
    () => ({ name: visibleInlineText(before.path), contents: preparedBefore?.text ?? '' }),
    [before.path, preparedBefore?.text],
  )
  const newFile = useMemo(
    () => ({ name: visibleInlineText(after.path), contents: preparedAfter?.text ?? '' }),
    [after.path, preparedAfter?.text],
  )
  const visualizedControls = preparedBefore?.visualized === true || preparedAfter?.visualized === true
  const unavailableTitle = containsNonUtf8
    ? 'Source is not valid UTF-8'
    : sourceTooLarge
      ? 'Source exceeds the interactive Code limit'
      : 'Control-character display exceeds the safe limit'
  const unavailableDetail = containsNonUtf8
    ? 'The code renderer is disabled to avoid lossy replacement characters.'
    : sourceTooLarge
      ? `Interactive Code is limited to ${MAX_INTERACTIVE_BYTES_PER_SIDE / 1024 / 1024} MiB and ${MAX_INTERACTIVE_LINES_PER_SIDE.toLocaleString('en')} lines per side.`
      : 'Expanding every hidden character into a visible token would create an unsafe amount of display text.'

  return (
    <div className="surface code-surface">
      <div className="surface-toolbar">
        <div>
          <span className="surface-kicker">LINE-LEVEL TRANSFORMATION</span>
          <h2>Code</h2>
        </div>
        <div className="segmented-control" aria-label="Diff layout">
          <button
            type="button"
            className={effectiveStyle === 'split' ? 'active' : ''}
            disabled={compact}
            onClick={() => onDiffStyleChange('split')}
          >
            <Columns2 size={14} /> Split
          </button>
          <button
            type="button"
            className={effectiveStyle === 'unified' ? 'active' : ''}
            onClick={() => onDiffStyleChange('unified')}
          >
            <Rows3 size={14} /> Unified
          </button>
        </div>
      </div>

      {!renderInteractiveDiff ? (
        <div className="binary-diff">
          <div className="binary-warning">
            <AlertOctagon size={17} aria-hidden="true" />
            <div>
              <strong>{unavailableTitle}</strong>
              <p>{unavailableDetail} This bounded preview is byte-safe; use Exact bytes for complete Hex and Base64.</p>
            </div>
          </div>
          <div className={`binary-panes ${effectiveStyle}`}>
            <NonUtf8Pane artifact={before} side="before" />
            <NonUtf8Pane artifact={after} side="after" />
          </div>
        </div>
      ) : (
        <div className="pierre-diff" data-testid="code-diff">
          <MultiFileDiff
            oldFile={oldFile}
            newFile={newFile}
            selectedLines={selectedLines}
            options={{
              diffStyle: effectiveStyle,
              diffIndicators: 'bars',
              lineDiffType: 'char',
              enableLineSelection: true,
              expandUnchanged: true,
              disableFileHeader: true,
              overflow: 'scroll',
              themeType: 'dark',
              theme: 'github-dark-default',
            }}
          />
        </div>
      )}
      {visualizedControls && (
        <div className="control-visibility-note" role="note">
          Hidden Unicode and control characters are shown as labeled tokens. Source bytes remain unchanged.
        </div>
      )}
      <div className="claim-boundary">
        {renderInteractiveDiff
          ? 'This is an exact snapshot comparison—not a claim of authorship, semantic equivalence, or a minimal patch.'
          : 'This preview is intentionally bounded. Exact bytes preserves the complete snapshots and lossless replay evidence.'}
      </div>
    </div>
  )
}

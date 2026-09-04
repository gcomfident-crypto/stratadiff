import { AlertTriangle, Check, ChevronLeft, ChevronRight, CircleDashed, Eye, Fingerprint, Link2, Sparkles, X } from 'lucide-react'
import { useLayoutEffect, useMemo, useRef, useState } from 'react'
import { base64DecodedLength, base64Preview, byteRange, editAfterRanges, nodeLabel, nodeLocation, predicateLabel, shortHash, titleCase } from '../lib/format'
import type { AmbiguityPair, EvidenceSelection, LoadedFileSession, NodeRef } from '../types'

const PAIRS_PER_PAGE = 80

interface InspectorProps {
  session: LoadedFileSession
  selection: EvidenceSelection
  open: boolean
  drawer: boolean
  onClose: () => void
}

function InspectorSection({ icon, title, children, tone = '' }: { icon: React.ReactNode; title: string; children: React.ReactNode; tone?: string }) {
  return (
    <section className={`inspector-section ${tone}`}>
      <div className="inspector-section-title">{icon}<h3>{title}</h3></div>
      {children}
    </section>
  )
}

function NodeSummary({ node, label }: { node: NodeRef; label: string }) {
  return (
    <div className="inspector-node">
      <small>{label}</small>
      <strong>{nodeLabel(node)}</strong>
      <span>{nodeLocation(node)} · bytes {byteRange(node.span.start_byte, node.span.end_byte)}</span>
    </div>
  )
}

function ClaimBoundary({ children }: { children: React.ReactNode }) {
  return <p className="not-claimed"><AlertTriangle size={14} />{children}</p>
}

function PossiblePairs({ pairs }: { pairs: AmbiguityPair[] }) {
  const [page, setPage] = useState(0)
  const pageCount = Math.max(1, Math.ceil(pairs.length / PAIRS_PER_PAGE))
  const safePage = Math.min(page, pageCount - 1)
  const firstPair = safePage * PAIRS_PER_PAGE
  const visiblePairs = pairs.slice(firstPair, firstPair + PAIRS_PER_PAGE)
  return (
    <>
      <div className="inspector-pairs">
        {visiblePairs.map((pair, index) => <span key={`${firstPair + index}-${pair.before_id}-${pair.after_id}`}>#{pair.before_id} → #{pair.after_id}</span>)}
      </div>
      {pageCount > 1 && (
        <div className="inspector-pair-pagination" aria-label="Possible pair pages">
          <button type="button" disabled={safePage === 0} onClick={() => setPage(safePage - 1)} aria-label="Previous possible pairs"><ChevronLeft size={13} /></button>
          <span>{firstPair + 1}–{Math.min(firstPair + PAIRS_PER_PAGE, pairs.length)} / {pairs.length}</span>
          <button type="button" disabled={safePage >= pageCount - 1} onClick={() => setPage(safePage + 1)} aria-label="Next possible pairs"><ChevronRight size={13} /></button>
        </div>
      )}
    </>
  )
}

export function Inspector({ session, selection, open, drawer, onClose }: InspectorProps) {
  const inspectorRef = useRef<HTMLElement>(null)
  const closeRef = useRef<HTMLButtonElement>(null)
  const afterRanges = useMemo(() => selection.type === 'edit' ? editAfterRanges(session.report.patch.edits) : [], [selection.type, session.report.patch.edits])

  useLayoutEffect(() => {
    if (!drawer || !open) return
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    closeRef.current?.focus()
    return () => previousFocus?.focus()
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

  const { report } = session
  let heading = 'Evidence'
  let subheading = 'Selected report item'
  let content: React.ReactNode

  if (selection.type === 'relation') {
    const relation = report.relations[selection.index]
    if (relation === undefined) throw new Error(`Relation ${selection.index} is missing.`)
    heading = predicateLabel(relation.predicate)
    subheading = `Relation R${selection.index + 1}`
    const selectionTitle = relation.correspondence === 'input_pair'
      ? 'Declared input'
      : relation.correspondence === 'model_forced'
        ? 'Forced by model'
        : 'Suggested by model'
    content = (
      <>
        <InspectorSection icon={<Eye size={15} />} title="Observed">
          <div className="inspector-node-pair"><NodeSummary node={relation.before} label="BEFORE" /><ChevronRight size={15} /><NodeSummary node={relation.after} label="AFTER" /></div>
          <dl><div><dt>Predicate</dt><dd>{predicateLabel(relation.predicate)}</dd></div><div><dt>Displayed location</dt><dd>1-based line · 0-based byte column</dd></div><div><dt>Report coordinates</dt><dd>{report.parser.coordinate_unit}</dd></div></dl>
        </InspectorSection>
        <InspectorSection icon={<Sparkles size={15} />} title={selectionTitle} tone={relation.correspondence === 'model_forced' ? 'forced-section' : ''}>
          <div className={`claim-badge ${relation.correspondence}`}>{titleCase(relation.correspondence)}</div>
          <ul className="evidence-list">{relation.evidence.map((item) => <li key={item}>{titleCase(item)}</li>)}</ul>
        </InspectorSection>
        <InspectorSection icon={<CircleDashed size={15} />} title="Not claimed">
          <ClaimBoundary>This pair is not proof of historical identity, author intent, or semantic equivalence.</ClaimBoundary>
        </InspectorSection>
      </>
    )
  } else if (selection.type === 'ambiguity') {
    const ambiguity = report.ambiguities[selection.index]
    if (ambiguity === undefined) throw new Error(`Ambiguity ${selection.index} is missing.`)
    const constraint = ambiguity.constraint
    const symbolic = constraint.kind === 'symbolic_abstention'
    heading = symbolic ? 'Symbolic abstention' : 'Ordered ambiguity'
    subheading = `Ambiguity A${selection.index + 1}`
    content = (
      <>
        <InspectorSection icon={<Eye size={15} />} title="Observed">
          <p>{ambiguity.reason}</p>
          <dl>
            <div><dt>Before endpoints</dt><dd>{ambiguity.before.length}</dd></div>
            <div><dt>After endpoints</dt><dd>{ambiguity.after.length}</dd></div>
            <div><dt>Parent pair</dt><dd>#{ambiguity.parent_before} → #{ambiguity.parent_after}</dd></div>
          </dl>
        </InspectorSection>
        <InspectorSection icon={symbolic ? <CircleDashed size={15} /> : <Link2 size={15} />} title={symbolic ? 'Model abstention' : 'Declared constraint'} tone="ambiguity-inspector">
          {constraint.kind === 'symbolic_abstention' ? (
            <><div className="claim-badge abstention">Abstained · {titleCase(constraint.cause)}</div><p>The endpoint arrays define scope only. No pair was selected.</p></>
          ) : (
            <><div className="claim-badge ordered">Exact constraint</div><p>Any resolution selects exactly {constraint.required_matches} listed pair{constraint.required_matches === 1 ? '' : 's'}, uses endpoints once, and preserves order.</p><PossiblePairs key={selection.index} pairs={constraint.possible_pairs} /></>
          )}
        </InspectorSection>
        <InspectorSection icon={<CircleDashed size={15} />} title="Not claimed">
          <ClaimBoundary>{symbolic ? '`pair_claims: none` means the report does not enumerate or assert any pair between these endpoint sets.' : 'Possible pairs are a coupled constraint, not independent or historical identity claims.'}</ClaimBoundary>
        </InspectorSection>
      </>
    )
  } else if (selection.type === 'edit') {
    const edit = report.patch.edits[selection.index]
    if (edit === undefined) throw new Error(`Byte edit ${selection.index} is missing.`)
    const replacementLength = base64DecodedLength(edit.replacement_base64)
    const afterRange = afterRanges[selection.index]
    if (afterRange === undefined) throw new Error(`Byte edit ${selection.index} has no replay range.`)
    const [afterStart, afterEnd] = afterRange
    heading = `Byte edit ${String(selection.index + 1).padStart(2, '0')}`
    subheading = 'Lossless replay operation'
    content = (
      <>
        <InspectorSection icon={<Eye size={15} />} title="Observed">
          <dl>
            <div><dt>Old range</dt><dd>{byteRange(edit.old_start, edit.old_end)}</dd></div>
            <div><dt>Replay range</dt><dd>{byteRange(afterStart, afterEnd)}</dd></div>
            <div><dt>Removed</dt><dd>{edit.old_end - edit.old_start} bytes</dd></div>
            <div><dt>Inserted</dt><dd>{replacementLength} bytes</dd></div>
          </dl>
          <code className="base64-value">{base64Preview(edit.replacement_base64)}</code>
        </InspectorSection>
        <InspectorSection icon={<Fingerprint size={15} />} title="Selected by model">
          <p>Replay applies this exact replacement under <code>{report.patch.algorithm}</code>.</p>
        </InspectorSection>
        <InspectorSection icon={<CircleDashed size={15} />} title="Not claimed">
          <ClaimBoundary>The byte edit sequence is exact, but is not claimed to be the unique or minimal patch.</ClaimBoundary>
        </InspectorSection>
      </>
    )
  } else {
    const change = report.changes[selection.index]
    if (change === undefined) throw new Error(`Structural change ${selection.index} is missing.`)
    heading = titleCase(change.kind)
    subheading = `Structural event E${selection.index + 1}`
    content = (
      <>
        <InspectorSection icon={<Eye size={15} />} title="Observed">
          <p>{change.detail}</p>
          {change.before !== undefined && <NodeSummary node={change.before} label="BEFORE" />}
          {change.after !== undefined && <NodeSummary node={change.after} label="AFTER" />}
        </InspectorSection>
        <InspectorSection icon={<Sparkles size={15} />} title="Selected by model" tone={change.kind.includes('model_forced') ? 'forced-section' : ''}>
          <div className={`claim-badge ${change.kind}`}>{titleCase(change.kind)}</div>
          <p>Derived from verified structure under the report’s declared correspondence model.</p>
        </InspectorSection>
        <InspectorSection icon={<CircleDashed size={15} />} title="Not claimed">
          <ClaimBoundary>This label does not prove author intent, semantic behavior, or a historical edit action.</ClaimBoundary>
        </InspectorSection>
      </>
    )
  }

  return (
    <aside ref={inspectorRef} className={`inspector ${open ? 'open' : ''}`} aria-label="Evidence inspector" aria-hidden={drawer && !open} aria-modal={drawer && open ? true : undefined} inert={drawer && !open} role={drawer ? 'dialog' : undefined} onKeyDown={handleKeyDown}>
      <div className="inspector-header">
        <div><span className="eyebrow">{subheading}</span><h2>{heading}</h2></div>
        <button ref={closeRef} type="button" className="inspector-close" onClick={onClose} aria-label="Close evidence inspector"><X size={17} /></button>
      </div>
      <div className="inspector-content">{content}</div>
      <div className="verification-trace">
        <div className="trace-title"><Check size={15} /><div><span>Verification trace</span><strong>{session.verification.message}</strong></div></div>
        <dl>
          <div><dt>Before</dt><dd title={report.certificate.before_blake3}>{shortHash(report.certificate.before_blake3)}</dd></div>
          <div><dt>Target</dt><dd title={report.certificate.after_blake3}>{shortHash(report.certificate.after_blake3)}</dd></div>
          <div><dt>Replay</dt><dd title={report.certificate.reconstructed_blake3}>{shortHash(report.certificate.reconstructed_blake3)}</dd></div>
        </dl>
        <p><Check size={12} /> Replay digest equals target digest</p>
      </div>
    </aside>
  )
}

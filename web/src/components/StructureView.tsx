import { AlertTriangle, ArrowRight, Braces, ChevronLeft, ChevronRight, CircleDashed, Link2, Network, ShieldQuestion } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { matchCount, matchPosition, pageIndices, type EvidenceSearchIndex } from '../lib/evidenceNavigation'
import { compactNumber, nodeLabel, nodeLocation, predicateLabel, titleCase } from '../lib/format'
import type { AmbiguityGroup, DiffReport, EvidenceSelection, NodeRef } from '../types'

interface StructureViewProps {
  report: DiffReport
  selection: EvidenceSelection
  onSelect: (selection: EvidenceSelection) => void
  searchIndex: EvidenceSearchIndex
}

export const RELATIONS_PER_PAGE = 120
export const AMBIGUITIES_PER_PAGE = 80
export const CHANGES_PER_PAGE = 160

function NodeCard({ node, side }: { node: NodeRef; side: 'before' | 'after' }) {
  return (
    <div className={`node-card node-${side}`}>
      <div className="node-id">#{node.id}</div>
      <div className="node-main">
        <strong>{node.kind}</strong>
        <span>{node.field ? `${node.field} · ` : ''}{nodeLocation(node)}</span>
      </div>
      <small>{node.subtree_size}n</small>
    </div>
  )
}

function AmbiguityCard({ ambiguity, index, active, onSelect }: { ambiguity: AmbiguityGroup; index: number; active: boolean; onSelect: () => void }) {
  const constraint = ambiguity.constraint
  const isAbstention = constraint.kind === 'symbolic_abstention'
  return (
    <button className={`ambiguity-card ${isAbstention ? 'symbolic' : 'ordered'} ${active ? 'active' : ''}`} type="button" onClick={onSelect}>
      <div className="ambiguity-card-header">
        <span className="ambiguity-symbol">{isAbstention ? <CircleDashed size={18} /> : <ShieldQuestion size={18} />}</span>
        <div>
          <span className="eyebrow">AMBIGUITY A{index + 1}</span>
          <strong>{isAbstention ? 'Symbolic abstention' : 'Exact ordered alignment'}</strong>
        </div>
        <span className="scope-count">{ambiguity.before.length} × {ambiguity.after.length}</span>
      </div>
      <p>{ambiguity.reason}</p>
      {constraint.kind === 'symbolic_abstention' ? (
        <div className="no-pairs-block">
          <div className="endpoint-group">
            {ambiguity.before.slice(0, 4).map((node) => <span key={node.id}>{nodeLabel(node)}</span>)}
          </div>
          <div className="no-link">
            <span>NO PAIR CLAIMS</span>
            <small>pair_claims: none</small>
          </div>
          <div className="endpoint-group after">
            {ambiguity.after.slice(0, 4).map((node) => <span key={node.id}>{nodeLabel(node)}</span>)}
          </div>
        </div>
      ) : (
        <div className="candidate-pairs">
          <div className="constraint-rule">
            Choose exactly {constraint.required_matches} · preserve order · unique endpoints
          </div>
          <div className="pair-chips">
            {constraint.possible_pairs.slice(0, 12).map((pair) => (
              <span key={`${pair.before_id}-${pair.after_id}`}>
                #{pair.before_id} <ArrowRight size={11} /> #{pair.after_id}
              </span>
            ))}
            {constraint.possible_pairs.length > 12 && <small>+{constraint.possible_pairs.length - 12} candidates</small>}
          </div>
        </div>
      )}
    </button>
  )
}

export function StructureView({ report, selection, onSelect, searchIndex }: StructureViewProps) {
  const surfaceRef = useRef<HTMLDivElement>(null)
  const [relationPage, setRelationPage] = useState(0)
  const [ambiguityPage, setAmbiguityPage] = useState(0)
  const [changePage, setChangePage] = useState(0)
  const relationCount = matchCount(searchIndex.relations, report.relations.length)
  const ambiguityCount = matchCount(searchIndex.ambiguities, report.ambiguities.length)
  const changeCount = matchCount(searchIndex.changes, report.changes.length)
  const relationPageCount = Math.max(1, Math.ceil(relationCount / RELATIONS_PER_PAGE))
  const ambiguityPageCount = Math.max(1, Math.ceil(ambiguityCount / AMBIGUITIES_PER_PAGE))
  const changePageCount = Math.max(1, Math.ceil(changeCount / CHANGES_PER_PAGE))
  const selectedRelationPosition = selection.type === 'relation'
    ? matchPosition(searchIndex.relations, report.relations.length, selection.index)
    : -1
  const selectedAmbiguityPosition = selection.type === 'ambiguity' ? matchPosition(searchIndex.ambiguities, report.ambiguities.length, selection.index) : -1
  const selectedChangePosition = selection.type === 'change' ? matchPosition(searchIndex.changes, report.changes.length, selection.index) : -1
  const relationPageStart = relationPage * RELATIONS_PER_PAGE
  const ambiguityPageStart = ambiguityPage * AMBIGUITIES_PER_PAGE
  const changePageStart = changePage * CHANGES_PER_PAGE
  const visibleRelations = pageIndices(searchIndex.relations, report.relations.length, relationPageStart, RELATIONS_PER_PAGE)
  const visibleAmbiguities = pageIndices(searchIndex.ambiguities, report.ambiguities.length, ambiguityPageStart, AMBIGUITIES_PER_PAGE)
  const visibleChanges = pageIndices(searchIndex.changes, report.changes.length, changePageStart, CHANGES_PER_PAGE)
  let selectionPage: number | null = null
  if (selection.type === 'relation') selectionPage = relationPage
  else if (selection.type === 'ambiguity') selectionPage = ambiguityPage
  else if (selection.type === 'change') selectionPage = changePage

  useEffect(() => {
    setRelationPage((current) => {
      if (selectedRelationPosition >= 0) return Math.floor(selectedRelationPosition / RELATIONS_PER_PAGE)
      return Math.min(current, relationPageCount - 1)
    })
  }, [relationPageCount, searchIndex, selectedRelationPosition])

  useEffect(() => {
    setAmbiguityPage((current) => {
      if (selectedAmbiguityPosition >= 0) return Math.floor(selectedAmbiguityPosition / AMBIGUITIES_PER_PAGE)
      return Math.min(current, ambiguityPageCount - 1)
    })
  }, [ambiguityPageCount, searchIndex, selectedAmbiguityPosition])

  useEffect(() => {
    setChangePage((current) => {
      if (selectedChangePosition >= 0) return Math.floor(selectedChangePosition / CHANGES_PER_PAGE)
      return Math.min(current, changePageCount - 1)
    })
  }, [changePageCount, searchIndex, selectedChangePosition])

  useLayoutEffect(() => {
    const active = surfaceRef.current?.querySelector<HTMLElement>('[aria-current="true"], .ambiguity-card.active, .change-card.active')
    if (active !== null && active !== undefined && typeof active.scrollIntoView === 'function') active.scrollIntoView({ block: 'center', inline: 'nearest' })
  }, [searchIndex, selection, selectionPage])

  return (
    <div ref={surfaceRef} className="surface structure-surface">
      <div className="surface-toolbar structure-toolbar">
        <div>
          <span className="surface-kicker">DECLARED CORRESPONDENCE MODEL</span>
          <h2>Structure</h2>
        </div>
        <div className="structure-legend" aria-label="Structure legend">
          <span><i className="legend-line exact" /> observed</span>
          <span><i className="legend-line forced" /> model-forced</span>
          <span><i className="legend-fill ambiguous" /> ambiguous</span>
        </div>
      </div>

      {report.parser.language === 'universal' && (
        <div className="universal-note">
          <Braces size={17} />
          <div><strong>Byte-defined structure, not AST.</strong><span>Universal mode groups lines and byte-token runs without language semantics.</span></div>
        </div>
      )}

      <div className="structure-summary">
        <div><Network size={17} /><strong>{compactNumber(relationCount)}</strong><span>visible relations</span></div>
        <div><AlertTriangle size={17} /><strong>{compactNumber(ambiguityCount)}</strong><span>ambiguity groups</span></div>
        <div><GitChangeIcon /><strong>{compactNumber(changeCount)}</strong><span>structural events</span></div>
      </div>

      <section className="structure-section">
        <div className="structure-section-heading">
          <div><Link2 size={16} /><h3>Relations</h3></div>
          <span>before node / checked predicate / after node</span>
        </div>
        <div className="relation-table">
          <div className="relation-columns" aria-hidden="true"><span>BEFORE</span><span>EVIDENCE</span><span>AFTER</span></div>
          {visibleRelations.map((index) => {
            const relation = report.relations[index]
            if (relation === undefined) throw new Error(`Relation ${index} is missing.`)
            return (
            <button
              className={`relation-row relation-${relation.correspondence} ${selection.type === 'relation' && selection.index === index ? 'active' : ''}`}
              key={`relation-${index}`}
              type="button"
              onClick={() => onSelect({ type: 'relation', index })}
              aria-current={selection.type === 'relation' && selection.index === index}
            >
              <NodeCard node={relation.before} side="before" />
              <div className="relation-link">
                <span className="relation-index">R{index + 1}</span>
                <span className="connection-line"><i /></span>
                <strong>{predicateLabel(relation.predicate)}</strong>
                <small>{titleCase(relation.correspondence)}</small>
              </div>
              <NodeCard node={relation.after} side="after" />
            </button>
            )
          })}
          {relationCount === 0 && <div className="empty-surface">No relations match the current search.</div>}
          {relationCount > RELATIONS_PER_PAGE && (
            <nav className="relation-pagination" aria-label="Relation pages">
              <span>
                Showing {relationPageStart + 1}–{Math.min(relationPageStart + RELATIONS_PER_PAGE, relationCount)} of {compactNumber(relationCount)}
              </span>
              <button type="button" onClick={() => setRelationPage((page) => page - 1)} disabled={relationPage === 0} aria-label="Previous relation page">
                <ChevronLeft size={14} />
              </button>
              <span>Page {relationPage + 1} of {relationPageCount}</span>
              <button type="button" onClick={() => setRelationPage((page) => page + 1)} disabled={relationPage + 1 >= relationPageCount} aria-label="Next relation page">
                <ChevronRight size={14} />
              </button>
            </nav>
          )}
        </div>
      </section>

      {ambiguityCount > 0 && (
        <section className="structure-section ambiguity-section">
          <div className="structure-section-heading">
            <div><AlertTriangle size={16} /><h3>Ambiguity ledger</h3></div>
            <span>explicit choices and abstentions</span>
          </div>
          <div className="ambiguity-grid">
            {visibleAmbiguities.map((index) => {
              const ambiguity = report.ambiguities[index]
              if (ambiguity === undefined) throw new Error(`Ambiguity ${index} is missing.`)
              return (
              <AmbiguityCard
                key={`ambiguity-card-${index}`}
                ambiguity={ambiguity}
                index={index}
                active={selection.type === 'ambiguity' && selection.index === index}
                onSelect={() => onSelect({ type: 'ambiguity', index })}
              />
              )
            })}
          </div>
          {ambiguityCount > AMBIGUITIES_PER_PAGE && (
            <nav className="relation-pagination" aria-label="Ambiguity pages">
              <span>Showing {ambiguityPageStart + 1}–{Math.min(ambiguityPageStart + AMBIGUITIES_PER_PAGE, ambiguityCount)} of {compactNumber(ambiguityCount)}</span>
              <button type="button" onClick={() => setAmbiguityPage((page) => page - 1)} disabled={ambiguityPage === 0} aria-label="Previous ambiguity page"><ChevronLeft size={14} /></button>
              <span>Page {ambiguityPage + 1} of {ambiguityPageCount}</span>
              <button type="button" onClick={() => setAmbiguityPage((page) => page + 1)} disabled={ambiguityPage + 1 >= ambiguityPageCount} aria-label="Next ambiguity page"><ChevronRight size={14} /></button>
            </nav>
          )}
        </section>
      )}

      {changeCount > 0 && (
        <section className="structure-section change-section">
          <div className="structure-section-heading">
            <div><GitChangeIcon /><h3>Derived events</h3></div>
            <span>interpretations under the declared model</span>
          </div>
          <div className="change-grid">
            {visibleChanges.map((index) => {
              const change = report.changes[index]
              if (change === undefined) throw new Error(`Structural change ${index} is missing.`)
              return (
              <button
                type="button"
                className={`change-card change-${change.kind} ${selection.type === 'change' && selection.index === index ? 'active' : ''}`}
                key={`change-card-${index}`}
                onClick={() => onSelect({ type: 'change', index })}
              >
                <span className="change-number">{String(index + 1).padStart(2, '0')}</span>
                <div><strong>{titleCase(change.kind)}</strong><p>{change.detail}</p></div>
                <ArrowRight size={15} />
              </button>
              )
            })}
          </div>
          {changeCount > CHANGES_PER_PAGE && (
            <nav className="relation-pagination" aria-label="Structural event pages">
              <span>Showing {changePageStart + 1}–{Math.min(changePageStart + CHANGES_PER_PAGE, changeCount)} of {compactNumber(changeCount)}</span>
              <button type="button" onClick={() => setChangePage((page) => page - 1)} disabled={changePage === 0} aria-label="Previous structural event page"><ChevronLeft size={14} /></button>
              <span>Page {changePage + 1} of {changePageCount}</span>
              <button type="button" onClick={() => setChangePage((page) => page + 1)} disabled={changePage + 1 >= changePageCount} aria-label="Next structural event page"><ChevronRight size={14} /></button>
            </nav>
          )}
        </section>
      )}
    </div>
  )
}

function GitChangeIcon() {
  return <span className="git-change-icon" aria-hidden="true"><i /><i /><i /></span>
}

import { AlertTriangle, ChevronLeft, ChevronRight, Filter, GitCommitHorizontal, Search, Split, X } from 'lucide-react'
import { forwardRef, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { matchCount, matchPosition, pageIndices, type EvidenceSearchIndex } from '../lib/evidenceNavigation'
import { byteRange, compactNumber, titleCase } from '../lib/format'
import type { DiffReport, EvidenceSelection } from '../types'

export interface SidebarFilters {
  changes: boolean
  ambiguities: boolean
  edits: boolean
}

const ITEMS_PER_PAGE = 100

interface SidebarProps {
  report: DiffReport
  selection: EvidenceSelection
  onSelect: (selection: EvidenceSelection) => void
  query: string
  onQueryChange: (query: string) => void
  showFilters: boolean
  onToggleFilters: () => void
  filters: SidebarFilters
  onFiltersChange: (filters: SidebarFilters) => void
  searchIndex: EvidenceSearchIndex
  drawer: boolean
  open: boolean
  onClose: () => void
}

function selected(selection: EvidenceSelection, type: EvidenceSelection['type'], index: number): boolean {
  return selection.type === type && selection.index === index
}

function PageControls({ label, page, total, onPage }: { label: string; page: number; total: number; onPage: (page: number) => void }) {
  const pageCount = Math.max(1, Math.ceil(total / ITEMS_PER_PAGE))
  const start = page * ITEMS_PER_PAGE
  if (pageCount === 1) return null
  return (
    <div className="nav-pagination" aria-label={`${label} pages`}>
      <button type="button" disabled={page === 0} onClick={() => onPage(page - 1)} aria-label={`Previous ${label} page`}><ChevronLeft size={12} /></button>
      <span>{start + 1}–{Math.min(start + ITEMS_PER_PAGE, total)} / {total}</span>
      <button type="button" disabled={page + 1 >= pageCount} onClick={() => onPage(page + 1)} aria-label={`Next ${label} page`}><ChevronRight size={12} /></button>
    </div>
  )
}

export const Sidebar = forwardRef<HTMLInputElement, SidebarProps>(function Sidebar(
  { report, selection, onSelect, query, onQueryChange, showFilters, onToggleFilters, filters, onFiltersChange, searchIndex, drawer, open, onClose },
  searchRef,
) {
  const asideRef = useRef<HTMLElement>(null)
  const [changePage, setChangePage] = useState(0)
  const [ambiguityPage, setAmbiguityPage] = useState(0)
  const [editPage, setEditPage] = useState(0)

  useLayoutEffect(() => {
    if (!drawer || !open) return
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
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
    const focusable = Array.from(asideRef.current?.querySelectorAll<HTMLElement>('button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])') ?? [])
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

  const changeCount = matchCount(searchIndex.changes, report.changes.length)
  const ambiguityCount = matchCount(searchIndex.ambiguities, report.ambiguities.length)
  const editCount = matchCount(searchIndex.edits, report.patch.edits.length)
  const selectedChangePosition = selection.type === 'change' ? matchPosition(searchIndex.changes, report.changes.length, selection.index) : -1
  const selectedAmbiguityPosition = selection.type === 'ambiguity' ? matchPosition(searchIndex.ambiguities, report.ambiguities.length, selection.index) : -1
  const selectedEditPosition = selection.type === 'edit' ? matchPosition(searchIndex.edits, report.patch.edits.length, selection.index) : -1
  const changes = pageIndices(searchIndex.changes, report.changes.length, changePage * ITEMS_PER_PAGE, ITEMS_PER_PAGE)
  const ambiguities = pageIndices(searchIndex.ambiguities, report.ambiguities.length, ambiguityPage * ITEMS_PER_PAGE, ITEMS_PER_PAGE)
  const edits = pageIndices(searchIndex.edits, report.patch.edits.length, editPage * ITEMS_PER_PAGE, ITEMS_PER_PAGE)

  useEffect(() => {
    setChangePage((current) => selectedChangePosition >= 0
      ? Math.floor(selectedChangePosition / ITEMS_PER_PAGE)
      : Math.min(current, Math.max(0, Math.ceil(changeCount / ITEMS_PER_PAGE) - 1)))
  }, [changeCount, selectedChangePosition])
  useEffect(() => {
    setAmbiguityPage((current) => selectedAmbiguityPosition >= 0
      ? Math.floor(selectedAmbiguityPosition / ITEMS_PER_PAGE)
      : Math.min(current, Math.max(0, Math.ceil(ambiguityCount / ITEMS_PER_PAGE) - 1)))
  }, [ambiguityCount, selectedAmbiguityPosition])
  useEffect(() => {
    setEditPage((current) => selectedEditPosition >= 0
      ? Math.floor(selectedEditPosition / ITEMS_PER_PAGE)
      : Math.min(current, Math.max(0, Math.ceil(editCount / ITEMS_PER_PAGE) - 1)))
  }, [editCount, selectedEditPosition])

  return (
    <aside ref={asideRef} className={`sidebar ${open ? 'open' : ''}`} aria-label="Evidence navigation" aria-hidden={drawer && !open} inert={drawer && !open} onKeyDown={handleKeyDown}>
      <div className="sidebar-heading">
        <div>
          <span className="eyebrow">REPORT INDEX</span>
          <h2>Evidence trail</h2>
        </div>
        <span className="keyboard-hint" title="Next / previous item">J K</span>
        {drawer && <button className="sidebar-close" type="button" onClick={onClose} aria-label="Close evidence navigation"><X size={16} /></button>}
      </div>

      <div className="sidebar-tools">
        <label className="search-field">
          <Search size={14} aria-hidden="true" />
          <span className="sr-only">Search report</span>
          <input
            ref={searchRef}
            aria-label="Search report"
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder="Search evidence"
          />
          {query.length > 0 && (
            <button type="button" onClick={() => onQueryChange('')} aria-label="Clear search">
              <X size={13} />
            </button>
          )}
          <kbd>/</kbd>
        </label>
        <button
          className={`filter-button ${showFilters ? 'active' : ''}`}
          type="button"
          onClick={onToggleFilters}
          aria-expanded={showFilters}
          aria-label="Toggle filters"
        >
          <Filter size={15} />
          <span>Filter</span>
          <kbd>F</kbd>
        </button>
      </div>

      {showFilters && (
        <div className="filter-panel">
          {([
            ['changes', 'Events'],
            ['ambiguities', 'Ambiguities'],
            ['edits', 'Byte edits'],
          ] as const).map(([key, label]) => (
            <label key={key}>
              <input
                type="checkbox"
                checked={filters[key]}
                onChange={() => onFiltersChange({ ...filters, [key]: !filters[key] })}
              />
              <span>{label}</span>
            </label>
          ))}
        </div>
      )}

      <nav className="evidence-sections">
        {filters.changes && (
          <section className="nav-section">
            <div className="section-label">
              <GitCommitHorizontal size={15} aria-hidden="true" />
              <span>Events</span>
              <span className="section-count">{compactNumber(changeCount)}</span>
            </div>
            <div className="nav-items">
              {changes.map((index) => {
                const change = report.changes[index]
                if (change === undefined) throw new Error(`Structural change ${index} is missing.`)
                return (
                <button
                  className={`nav-item change-${change.kind} ${selected(selection, 'change', index) ? 'selected' : ''}`}
                  key={`change-${index}`}
                  type="button"
                  onClick={() => onSelect({ type: 'change', index })}
                  aria-current={selected(selection, 'change', index)}
                >
                  <span className="nav-marker" />
                  <span className="nav-copy">
                    <strong>{titleCase(change.kind)}</strong>
                    <small>{change.detail}</small>
                  </span>
                  <span className="nav-index">{String(index + 1).padStart(2, '0')}</span>
                </button>
                )
              })}
              <PageControls label="event" page={changePage} total={changeCount} onPage={setChangePage} />
            </div>
          </section>
        )}

        {filters.ambiguities && (
          <section className="nav-section">
            <div className="section-label ambiguity-label">
              <AlertTriangle size={15} aria-hidden="true" />
              <span>Ambiguities</span>
              <span className="section-count">{compactNumber(ambiguityCount)}</span>
            </div>
            <div className="nav-items">
              {ambiguities.map((index) => {
                const ambiguity = report.ambiguities[index]
                if (ambiguity === undefined) throw new Error(`Ambiguity ${index} is missing.`)
                return (
                <button
                  className={`nav-item ambiguity-item ${selected(selection, 'ambiguity', index) ? 'selected' : ''}`}
                  key={`ambiguity-${index}`}
                  type="button"
                  onClick={() => onSelect({ type: 'ambiguity', index })}
                  aria-current={selected(selection, 'ambiguity', index)}
                >
                  <span className="nav-marker" />
                  <span className="nav-copy">
                    <strong>{ambiguity.constraint.kind === 'symbolic_abstention' ? 'Abstention' : 'Ordered choice'}</strong>
                    <small>{ambiguity.reason}</small>
                  </span>
                  <span className="nav-index">A{index + 1}</span>
                </button>
                )
              })}
              <PageControls label="ambiguity" page={ambiguityPage} total={ambiguityCount} onPage={setAmbiguityPage} />
            </div>
          </section>
        )}

        {filters.edits && (
          <section className="nav-section">
            <div className="section-label bytes-label">
              <Split size={15} aria-hidden="true" />
              <span>Byte edits</span>
              <span className="section-count">{compactNumber(editCount)}</span>
            </div>
            <div className="nav-items">
              {edits.map((index) => {
                const edit = report.patch.edits[index]
                if (edit === undefined) throw new Error(`Byte edit ${index} is missing.`)
                return (
                <button
                  className={`nav-item byte-item ${selected(selection, 'edit', index) ? 'selected' : ''}`}
                  key={`edit-${index}`}
                  type="button"
                  onClick={() => onSelect({ type: 'edit', index })}
                  aria-current={selected(selection, 'edit', index)}
                >
                  <span className="nav-marker" />
                  <span className="nav-copy">
                    <strong>Edit {String(index + 1).padStart(2, '0')}</strong>
                    <small>old bytes {byteRange(edit.old_start, edit.old_end)}</small>
                  </span>
                  <span className="nav-index">B{index + 1}</span>
                </button>
                )
              })}
              <PageControls label="byte edit" page={editPage} total={editCount} onPage={setEditPage} />
            </div>
          </section>
        )}

        {changeCount + ambiguityCount + editCount === 0 && (
          <div className="empty-nav">No events, ambiguities, or byte edits match this filter. Relations remain available in Structure.</div>
        )}
      </nav>
      <div className="sidebar-footer">
        <span>{report.parser.root_kind}</span>
        <span>{report.parser.before_nodes} → {report.parser.after_nodes} nodes</span>
      </div>
    </aside>
  )
})

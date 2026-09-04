import { AlertCircle, LoaderCircle, RefreshCw } from 'lucide-react'
import { useDeferredValue, useEffect, useMemo, useRef, useState } from 'react'
import { ByteView } from './components/ByteView'
import { CodeDiffView } from './components/CodeDiffView'
import { Header } from './components/Header'
import { HelpDialog } from './components/HelpDialog'
import { Inspector } from './components/Inspector'
import { Sidebar, type SidebarFilters } from './components/Sidebar'
import { StructureView } from './components/StructureView'
import { TrustStrip } from './components/TrustStrip'
import { ViewTabs } from './components/ViewTabs'
import { useMediaQuery } from './hooks/useMediaQuery'
import { buildEvidenceNavigation, buildEvidenceSearchIndex, stepEvidence } from './lib/evidenceNavigation'
import { fetchSession } from './lib/session'
import type { DiffReport, DiffStyle, EvidenceSelection, LoadedSession, ViewMode } from './types'

function defaultSelection(report: DiffReport): EvidenceSelection {
  if (report.changes.length > 0) return { type: 'change', index: 0 }
  if (report.ambiguities.length > 0) return { type: 'ambiguity', index: 0 }
  if (report.patch.edits.length > 0) return { type: 'edit', index: 0 }
  return { type: 'relation', index: 0 }
}

function LoadingScreen() {
  return (
    <main className="state-screen">
      <div className="state-mark loading"><LoaderCircle size={24} /></div>
      <span className="eyebrow">LOCAL VERIFIED SESSION</span>
      <h1>Opening evidence workbench</h1>
      <p>Loading the report and both source snapshots…</p>
    </main>
  )
}

function ErrorScreen({ message }: { message: string }) {
  return (
    <main className="state-screen error-state" role="alert">
      <div className="state-mark"><AlertCircle size={24} /></div>
      <span className="eyebrow">SESSION UNAVAILABLE</span>
      <h1>Could not open this report</h1>
      <p>{message}</p>
      <code>stratadiff view &lt;before&gt; &lt;after&gt;</code>
      <button type="button" onClick={() => window.location.reload()}><RefreshCw size={15} />Retry</button>
      <small>For safety, this viewer requires the one-time token created by the local StrataDiff server.</small>
    </main>
  )
}

function Workbench({ session }: { session: LoadedSession }) {
  const [view, setView] = useState<ViewMode>('code')
  const [diffStyle, setDiffStyle] = useState<DiffStyle>('split')
  const [selection, setSelection] = useState<EvidenceSelection>(() => defaultSelection(session.report))
  const [query, setQuery] = useState('')
  const deferredQuery = useDeferredValue(query)
  const [showFilters, setShowFilters] = useState(false)
  const [filters, setFilters] = useState<SidebarFilters>({ changes: true, ambiguities: true, edits: true })
  const [inspectorOpen, setInspectorOpen] = useState(false)
  const [sidebarOpen, setSidebarOpen] = useState(false)
  const [helpOpen, setHelpOpen] = useState(false)
  const searchRef = useRef<HTMLInputElement>(null)
  const viewStageRef = useRef<HTMLDivElement>(null)
  const compact = useMediaQuery('(max-width: 899px)')
  const inspectorIsDrawer = useMediaQuery('(max-width: 1279px)')
  const sidebarIsDrawer = useMediaQuery('(max-width: 1279px)')

  const searchIndex = useMemo(
    () => buildEvidenceSearchIndex(session.report, deferredQuery),
    [deferredQuery, session.report],
  )
  const navigation = useMemo(
    () => buildEvidenceNavigation(session.report, searchIndex, filters),
    [filters, searchIndex, session.report],
  )

  function changeView(next: ViewMode): void {
    if (viewStageRef.current !== null) viewStageRef.current.scrollTop = 0
    setView(next)
  }

  function selectEvidence(next: EvidenceSelection): void {
    setSelection(next)
    setSidebarOpen(false)
    if (next.type === 'edit') changeView('bytes')
    else changeView('structure')
    if (inspectorIsDrawer) setInspectorOpen(true)
  }

  function openEvidenceNavigation(focusSearch: boolean): void {
    setInspectorOpen(false)
    setSidebarOpen(sidebarIsDrawer)
    if (focusSearch) window.requestAnimationFrame(() => searchRef.current?.focus())
  }

  function navigateEvidence(direction: 1 | -1): void {
    const next = stepEvidence(navigation, selection, direction)
    if (next !== null) selectEvidence(next)
  }

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent): void {
      const target = event.target
      const typing = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement
      const modified = event.metaKey || event.ctrlKey || event.altKey
      if (helpOpen) {
        if (event.key === 'Escape') setHelpOpen(false)
        return
      }
      if (inspectorIsDrawer && inspectorOpen) {
        if (event.key === 'Escape') setInspectorOpen(false)
        return
      }
      if (sidebarIsDrawer && sidebarOpen) {
        if (event.key === 'Escape') setSidebarOpen(false)
        else if (!typing && !modified && event.key.toLocaleLowerCase() === 'f') {
          event.preventDefault()
          setShowFilters((value) => !value)
        } else if (!typing && !modified && (event.key === 'j' || event.key === 'k')) {
          event.preventDefault()
          navigateEvidence(event.key === 'j' ? 1 : -1)
        }
        return
      }
      if (event.key === 'Escape') {
        if (inspectorOpen) setInspectorOpen(false)
        else if (sidebarIsDrawer && sidebarOpen) setSidebarOpen(false)
        else if (query.length > 0) setQuery('')
        return
      }
      if (typing || modified) return
      if (event.key === '/') {
        event.preventDefault()
        openEvidenceNavigation(true)
      } else if (event.key.toLocaleLowerCase() === 'f') {
        event.preventDefault()
        if (sidebarIsDrawer) openEvidenceNavigation(true)
        setShowFilters((value) => !value)
      } else if (event.key === '?') {
        event.preventDefault()
        setHelpOpen(true)
      } else if (event.key === '1') changeView('code')
      else if (event.key === '2') changeView('structure')
      else if (event.key === '3') changeView('bytes')
      else if (event.key === 'j' || event.key === 'k') {
        event.preventDefault()
        navigateEvidence(event.key === 'j' ? 1 : -1)
      } else if (event.key === '[' || event.key === ']') {
        if (session.report.ambiguities.length === 0) return
        event.preventDefault()
        const direction = event.key === ']' ? 1 : -1
        const next = selection.type === 'ambiguity'
          ? (selection.index + direction + session.report.ambiguities.length) % session.report.ambiguities.length
          : event.key === ']' ? 0 : session.report.ambiguities.length - 1
        selectEvidence({ type: 'ambiguity', index: next })
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [helpOpen, inspectorIsDrawer, inspectorOpen, navigation, query.length, selection, session.report.ambiguities.length, sidebarIsDrawer, sidebarOpen])

  return (
    <div className="workbench-shell">
      <div className="workbench-content" aria-hidden={helpOpen} inert={helpOpen}>
        <Header session={session} onOpenInspector={() => { setSidebarOpen(false); setInspectorOpen(true) }} />
        <TrustStrip report={session.report} />
        <div className="workspace">
          <Sidebar
            ref={searchRef}
            report={session.report}
            selection={selection}
            onSelect={selectEvidence}
            query={query}
            onQueryChange={setQuery}
            showFilters={showFilters}
            onToggleFilters={() => setShowFilters((value) => !value)}
            filters={filters}
            onFiltersChange={setFilters}
            searchIndex={searchIndex}
            drawer={sidebarIsDrawer}
            open={sidebarOpen}
            onClose={() => setSidebarOpen(false)}
          />
          <main className="main-workspace">
            <ViewTabs view={view} onChange={changeView} onHelp={() => setHelpOpen(true)} onOpenEvidence={() => openEvidenceNavigation(true)} />
            <div ref={viewStageRef} className="view-stage" id="evidence-view-panel" role="tabpanel" aria-label={`${view} evidence`}>
              {view === 'code' && (
                <CodeDiffView
                  before={session.decodedBefore}
                  after={session.decodedAfter}
                  diffStyle={diffStyle}
                  onDiffStyleChange={setDiffStyle}
                  compact={compact}
                  report={session.report}
                  selection={selection}
                />
              )}
              {view === 'structure' && <StructureView report={session.report} selection={selection} onSelect={selectEvidence} searchIndex={searchIndex} />}
              {view === 'bytes' && <ByteView report={session.report} before={session.decodedBefore} after={session.decodedAfter} selection={selection} onSelect={selectEvidence} />}
            </div>
          </main>
          {sidebarIsDrawer && sidebarOpen && <button type="button" className="drawer-backdrop sidebar-backdrop" onClick={() => setSidebarOpen(false)} aria-label="Close evidence navigation" />}
          {inspectorIsDrawer && inspectorOpen && <button type="button" className="drawer-backdrop" onClick={() => setInspectorOpen(false)} aria-label="Close evidence inspector" />}
          <Inspector session={session} selection={selection} open={inspectorOpen} drawer={inspectorIsDrawer} onClose={() => setInspectorOpen(false)} />
        </div>
      </div>
      <HelpDialog open={helpOpen} onClose={() => setHelpOpen(false)} />
    </div>
  )
}

export default function App() {
  const [session, setSession] = useState<LoadedSession | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const controller = new AbortController()
    fetchSession(window.location.search, controller.signal)
      .then(setSession)
      .catch((reason: unknown) => {
        if (reason instanceof DOMException && reason.name === 'AbortError') return
        setError(reason instanceof Error ? reason.message : 'The local viewer returned an unknown error.')
      })
    return () => controller.abort()
  }, [])

  if (error !== null) return <ErrorScreen message={error} />
  if (session === null) return <LoadingScreen />
  return <Workbench session={session} />
}

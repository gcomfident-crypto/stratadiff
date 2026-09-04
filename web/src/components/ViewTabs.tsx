import { Binary, Braces, Code2, ListFilter } from 'lucide-react'
import type { ViewMode } from '../types'

interface ViewTabsProps {
  view: ViewMode
  onChange: (view: ViewMode) => void
  onHelp: () => void
  onOpenEvidence: () => void
}

const tabs = [
  { id: 'code', label: 'Code', key: '1', icon: Code2 },
  { id: 'structure', label: 'Structure', key: '2', icon: Braces },
  { id: 'bytes', label: 'Exact bytes', key: '3', icon: Binary },
] as const

export function ViewTabs({ view, onChange, onHelp, onOpenEvidence }: ViewTabsProps) {
  function handleTabKeyDown(event: React.KeyboardEvent<HTMLButtonElement>, index: number): void {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight' && event.key !== 'Home' && event.key !== 'End') return
    event.preventDefault()
    const nextIndex = event.key === 'Home'
      ? 0
      : event.key === 'End'
        ? tabs.length - 1
        : (index + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length
    const next = tabs[nextIndex]
    if (next === undefined) throw new Error(`Evidence tab ${nextIndex} is missing.`)
    onChange(next.id)
    event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[nextIndex]?.focus()
  }

  return (
    <div className="view-tabs">
      <div className="view-tablist" role="tablist" aria-label="Evidence views">
        {tabs.map(({ id, label, key, icon: Icon }, index) => (
          <button
            type="button"
            role="tab"
            aria-selected={view === id}
            aria-controls="evidence-view-panel"
            tabIndex={view === id ? 0 : -1}
            className={view === id ? 'active' : ''}
            onClick={() => onChange(id)}
            onKeyDown={(event) => handleTabKeyDown(event, index)}
            key={id}
          >
            <Icon size={15} /><span>{label}</span><kbd>{key}</kbd>
          </button>
        ))}
      </div>
      <button className="evidence-trigger" type="button" onClick={onOpenEvidence} aria-label="Open evidence search and navigation" title="Evidence search and navigation"><ListFilter size={15} /></button>
      <button className="help-trigger" type="button" onClick={onHelp} aria-label="Keyboard shortcuts" title="Keyboard shortcuts">?</button>
    </div>
  )
}

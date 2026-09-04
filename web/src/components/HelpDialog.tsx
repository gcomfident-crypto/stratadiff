import { X } from 'lucide-react'
import { useLayoutEffect, useRef } from 'react'

interface HelpDialogProps {
  open: boolean
  onClose: () => void
}

const shortcuts = [
  ['J / K', 'Next / previous evidence item'],
  ['[ / ]', 'Previous / next ambiguity'],
  ['1 / 2 / 3', 'Code / Structure / Exact bytes'],
  ['/', 'Focus evidence search'],
  ['F', 'Show or hide filters'],
  ['Esc', 'Clear search or close a panel'],
  ['?', 'Open this shortcut guide'],
]

export function HelpDialog({ open, onClose }: HelpDialogProps) {
  const dialogRef = useRef<HTMLElement>(null)
  const closeRef = useRef<HTMLButtonElement>(null)

  useLayoutEffect(() => {
    if (!open) return
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    closeRef.current?.focus()
    return () => previousFocus?.focus()
  }, [open])

  function handleKeyDown(event: React.KeyboardEvent<HTMLElement>): void {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      onClose()
      return
    }
    if (event.key !== 'Tab') return
    const focusable = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>('button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])') ?? [])
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

  if (!open) return null
  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={onClose}>
      <section ref={dialogRef} className="help-dialog" role="dialog" aria-modal="true" aria-labelledby="help-title" onKeyDown={handleKeyDown} onMouseDown={(event) => event.stopPropagation()}>
        <div className="dialog-header"><div><span className="eyebrow">KEYBOARD</span><h2 id="help-title">Move through the evidence</h2></div><button ref={closeRef} type="button" onClick={onClose} aria-label="Close shortcuts"><X size={18} /></button></div>
        <div className="shortcut-list">
          {shortcuts.map(([key, label]) => <div key={key}><kbd>{key}</kbd><span>{label}</span></div>)}
        </div>
        <p>Shortcuts pause while typing in an input.</p>
      </section>
    </div>
  )
}

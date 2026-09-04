import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { sessionFixture } from '../test/fixture'
import type { LoadedSession } from '../types'
import { Inspector } from './Inspector'

function loadedSession(): LoadedSession {
  const payload = sessionFixture()
  const beforeText = 'const before = 1\n'
  const afterText = 'const after = 2\n'
  return {
    ...payload,
    decodedBefore: { path: payload.report.before.path, bytes: new TextEncoder().encode(beforeText), text: beforeText },
    decodedAfter: { path: payload.report.after.path, bytes: new TextEncoder().encode(afterText), text: afterText },
  }
}

describe('Inspector bounded evidence rendering', () => {
  it('renders possible pairs one bounded page at a time', () => {
    const session = loadedSession()
    const ambiguity = session.report.ambiguities[0]
    if (ambiguity === undefined) throw new Error('Fixture ambiguity is missing.')
    const possiblePairs = Array.from({ length: 161 }, (_, index) => ({ before_id: index + 1, after_id: index + 1_001 }))
    session.report.ambiguities = [{
      ...ambiguity,
      constraint: {
        kind: 'exact_ordered_alignment',
        predicate: 'syntax_equal',
        required_matches: 1,
        possible_pairs: possiblePairs,
      },
    }]

    const { container } = render(<Inspector session={session} selection={{ type: 'ambiguity', index: 0 }} open drawer={false} onClose={vi.fn()} />)
    const pairList = container.querySelector('.inspector-pairs')
    if (pairList === null) throw new Error('Possible-pairs list was not rendered.')

    expect(pairList.children).toHaveLength(80)
    expect(within(pairList as HTMLElement).getByText('#1 → #1001')).toBeInTheDocument()
    expect(within(pairList as HTMLElement).queryByText('#81 → #1081')).not.toBeInTheDocument()
    expect(screen.getByText('1–80 / 161')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Next possible pairs' }))
    expect(pairList.children).toHaveLength(80)
    expect(within(pairList as HTMLElement).getByText('#81 → #1081')).toBeInTheDocument()
    expect(within(pairList as HTMLElement).queryByText('#1 → #1001')).not.toBeInTheDocument()
    expect(screen.getByText('81–160 / 161')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Next possible pairs' }))
    expect(pairList.children).toHaveLength(1)
    expect(within(pairList as HTMLElement).getByText('#161 → #1161')).toBeInTheDocument()
    expect(screen.getByText('161–161 / 161')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Next possible pairs' })).toBeDisabled()

    fireEvent.click(screen.getByRole('button', { name: 'Previous possible pairs' }))
    expect(pairList.children).toHaveLength(80)
    expect(screen.getByText('81–160 / 161')).toBeInTheDocument()
  })

  it('renders only a bounded Base64 preview for a large replacement', () => {
    const session = loadedSession()
    const longBase64 = 'QUJD'.repeat(100)
    const edit = session.report.patch.edits[0]
    if (edit === undefined) throw new Error('Fixture byte edit is missing.')
    session.report.patch.edits = [{ ...edit, replacement_base64: longBase64 }]

    const { container } = render(<Inspector session={session} selection={{ type: 'edit', index: 0 }} open drawer={false} onClose={vi.fn()} />)
    const preview = container.querySelector('.base64-value')
    if (preview === null) throw new Error('Base64 preview was not rendered.')

    expect(preview.textContent).toBe(`${longBase64.slice(0, 160)}… (400 Base64 characters)`)
    expect(container.textContent).not.toContain(longBase64)
  })
})

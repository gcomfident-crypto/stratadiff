import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { buildEvidenceSearchIndex } from '../lib/evidenceNavigation'
import { sessionFixture } from '../test/fixture'
import { StructureView } from './StructureView'

describe('StructureView relations', () => {
  const scrollIntoView = vi.fn()

  beforeEach(() => {
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
      configurable: true,
      value: scrollIntoView,
    })
  })

  afterEach(() => {
    scrollIntoView.mockClear()
    delete (HTMLElement.prototype as { scrollIntoView?: typeof HTMLElement.prototype.scrollIntoView }).scrollIntoView
  })

  it('makes every matching relation reachable with pagination', async () => {
    const fixture = sessionFixture()
    const template = fixture.report.relations[0]
    if (template === undefined) throw new Error('Fixture relation is missing.')
    const report = {
      ...fixture.report,
      relations: Array.from({ length: 125 }, (_, index) => ({
        ...template,
        before: { ...template.before, id: index * 2 + 1 },
        after: { ...template.after, id: index * 2 + 2 },
      })),
    }
    const onSelect = vi.fn()

    const { rerender } = render(<StructureView report={report} selection={{ type: 'change', index: 0 }} onSelect={onSelect} searchIndex={buildEvidenceSearchIndex(report, '')} />)

    expect(screen.getByText('R1')).toBeInTheDocument()
    expect(screen.getByText('R120')).toBeInTheDocument()
    expect(screen.queryByText('R121')).not.toBeInTheDocument()
    expect(screen.getByText('Showing 1–120 of 125')).toBeInTheDocument()

    scrollIntoView.mockClear()
    fireEvent.click(screen.getByRole('button', { name: 'Next relation page' }))

    expect(screen.queryByText('R1')).not.toBeInTheDocument()
    expect(screen.getByText('R121')).toBeInTheDocument()
    expect(screen.getByText('R125')).toBeInTheDocument()
    expect(screen.getByText('Showing 121–125 of 125')).toBeInTheDocument()
    expect(scrollIntoView).not.toHaveBeenCalled()

    fireEvent.click(screen.getByText('R123').closest('button') as HTMLButtonElement)
    expect(onSelect).toHaveBeenCalledWith({ type: 'relation', index: 122 })

    rerender(<StructureView report={report} selection={{ type: 'change', index: 0 }} onSelect={onSelect} searchIndex={buildEvidenceSearchIndex(report, 'R123')} />)
    expect(await screen.findByText('R123')).toBeInTheDocument()
    expect(screen.queryByText('R122')).not.toBeInTheDocument()
  })

  it('opens the page containing an externally selected relation', async () => {
    const fixture = sessionFixture()
    const template = fixture.report.relations[0]
    if (template === undefined) throw new Error('Fixture relation is missing.')
    const report = {
      ...fixture.report,
      relations: Array.from({ length: 125 }, () => ({ ...template })),
    }

    render(<StructureView report={report} selection={{ type: 'relation', index: 122 }} onSelect={vi.fn()} searchIndex={buildEvidenceSearchIndex(report, '')} />)

    expect(await screen.findByText('R123')).toBeInTheDocument()
    expect(screen.getByText('R123').closest('button')).toHaveAttribute('aria-current', 'true')
  })

  it('re-centers a selected relation when search results change on the same page', () => {
    const fixture = sessionFixture()
    const template = fixture.report.relations[0]
    if (template === undefined) throw new Error('Fixture relation is missing.')
    const report = {
      ...fixture.report,
      relations: Array.from({ length: 80 }, () => ({ ...template })),
    }
    const selection = { type: 'relation', index: 50 } as const
    const { rerender } = render(<StructureView report={report} selection={selection} onSelect={vi.fn()} searchIndex={buildEvidenceSearchIndex(report, '')} />)
    scrollIntoView.mockClear()

    rerender(<StructureView report={report} selection={selection} onSelect={vi.fn()} searchIndex={buildEvidenceSearchIndex(report, 'R51')} />)

    expect(scrollIntoView).toHaveBeenCalledTimes(1)
  })

  it('makes later ambiguities and structural events reachable', async () => {
    const fixture = sessionFixture()
    const ambiguity = fixture.report.ambiguities[0]
    const change = fixture.report.changes[0]
    if (ambiguity === undefined || change === undefined) throw new Error('Fixture evidence is missing.')
    const report = {
      ...fixture.report,
      ambiguities: Array.from({ length: 81 }, (_, index) => ({ ...ambiguity, reason: `Ambiguity reason ${index + 1}` })),
      changes: Array.from({ length: 161 }, (_, index) => ({ ...change, detail: `Structural detail ${index + 1}` })),
    }

    const { rerender } = render(<StructureView report={report} selection={{ type: 'change', index: 160 }} onSelect={vi.fn()} searchIndex={buildEvidenceSearchIndex(report, '')} />)
    expect(await screen.findByText('Structural detail 161')).toBeInTheDocument()
    expect(screen.queryByText('Structural detail 1')).not.toBeInTheDocument()
    expect(screen.getByText('Showing 161–161 of 161')).toBeInTheDocument()

    rerender(<StructureView report={report} selection={{ type: 'ambiguity', index: 80 }} onSelect={vi.fn()} searchIndex={buildEvidenceSearchIndex(report, '')} />)
    expect(await screen.findByText('Ambiguity reason 81')).toBeInTheDocument()
    expect(screen.queryByText('Ambiguity reason 1')).not.toBeInTheDocument()
    expect(screen.getByText('Showing 81–81 of 81')).toBeInTheDocument()
  })
})

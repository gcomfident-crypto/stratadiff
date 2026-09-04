import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { buildEvidenceSearchIndex } from '../lib/evidenceNavigation'
import { sessionFixture } from '../test/fixture'
import { Sidebar } from './Sidebar'

describe('Sidebar relation search results', () => {
  it('shows relation-only matches and selects the clicked relation', () => {
    const report = sessionFixture().report
    const onSelect = vi.fn()

    render(
      <Sidebar
        report={report}
        selection={{ type: 'change', index: 0 }}
        onSelect={onSelect}
        query="R1"
        onQueryChange={vi.fn()}
        showFilters={false}
        onToggleFilters={vi.fn()}
        filters={{ changes: true, ambiguities: true, edits: true }}
        onFiltersChange={vi.fn()}
        searchIndex={buildEvidenceSearchIndex(report, 'R1')}
        drawer={false}
        open={false}
        onClose={vi.fn()}
      />,
    )

    expect(screen.getByText('Relations')).toBeInTheDocument()
    const relation = screen.getByText('R1').closest('button')
    if (relation === null) throw new Error('Relation result button is missing.')
    expect(relation).toHaveTextContent('Input pair')
    expect(screen.queryByText('No evidence matches this search and filter.')).not.toBeInTheDocument()

    fireEvent.click(relation)

    expect(onSelect).toHaveBeenCalledOnce()
    expect(onSelect).toHaveBeenCalledWith({ type: 'relation', index: 0 })
  })

  it('keeps relations out of the sidebar without a query or a matching result', () => {
    const report = sessionFixture().report
    const props = {
      report,
      selection: { type: 'change', index: 0 } as const,
      onSelect: vi.fn(),
      onQueryChange: vi.fn(),
      showFilters: false,
      onToggleFilters: vi.fn(),
      filters: { changes: true, ambiguities: true, edits: true },
      onFiltersChange: vi.fn(),
      drawer: false,
      open: false,
      onClose: vi.fn(),
    }
    const { rerender } = render(
      <Sidebar {...props} query="" searchIndex={buildEvidenceSearchIndex(report, '')} />,
    )

    expect(screen.queryByText('Relations')).not.toBeInTheDocument()

    rerender(
      <Sidebar {...props} query="R1" searchIndex={buildEvidenceSearchIndex(report, '')} />,
    )

    expect(screen.queryByText('Relations')).not.toBeInTheDocument()

    rerender(
      <Sidebar {...props} query="missing relation" searchIndex={buildEvidenceSearchIndex(report, 'missing relation')} />,
    )

    expect(screen.queryByText('Relations')).not.toBeInTheDocument()
    expect(screen.getByText('No evidence matches this search and filter.')).toBeInTheDocument()
  })
})

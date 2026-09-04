import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { sessionFixture } from './test/fixture'

vi.mock('@pierre/diffs/react', () => ({
  MultiFileDiff: ({ oldFile, newFile }: { oldFile: { contents: string }; newFile: { contents: string } }) => (
    <div data-testid="rendered-diff">{oldFile.contents} → {newFile.contents}</div>
  ),
}))

function mediaQueryResult(query: string, matches: boolean) {
  return {
    matches,
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }
}

describe('Evidence Workbench', () => {
  beforeEach(() => {
    vi.mocked(window.matchMedia).mockImplementation((query: string) => mediaQueryResult(query, false))
    window.history.replaceState({}, '', '/?token=test-token')
    const payload = sessionFixture()
    vi.stubGlobal('fetch', vi.fn().mockImplementation((url: string) => {
      if (url.startsWith('/api/session')) return Promise.resolve(new Response(JSON.stringify(payload), { status: 200 }))
      const source = url.includes('/before') ? 'const before = 1\n' : 'const after = 2\n'
      return Promise.resolve(new Response(new TextEncoder().encode(source), { status: 200 }))
    }))
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    delete (HTMLElement.prototype as { scrollIntoView?: typeof HTMLElement.prototype.scrollIntoView }).scrollIntoView
  })

  it('opens a verified session and renders the real source diff', async () => {
    render(<App />)
    expect(screen.getByText('Opening evidence workbench')).toBeInTheDocument()
    expect(await screen.findByText('Evidence Workbench')).toBeInTheDocument()
    expect(screen.getByTestId('rendered-diff')).toHaveTextContent('const before = 1')
    expect(screen.getAllByText('Verified')).toHaveLength(2)
  })

  it('switches all three evidence layers from the keyboard', async () => {
    render(<App />)
    await screen.findByText('Evidence Workbench')

    fireEvent.keyDown(window, { key: '2' })
    expect(await screen.findByText('Relations')).toBeInTheDocument()
    expect(screen.getByText('pair_claims: none')).toBeInTheDocument()

    fireEvent.keyDown(window, { key: '3' })
    expect(await screen.findByText('LOSSLESS REPLAY LAYER')).toBeInTheDocument()

    fireEvent.keyDown(window, { key: '1' })
    await waitFor(() => expect(screen.getByTestId('rendered-diff')).toBeInTheDocument())
  })

  it('keeps the active structure evidence in view after switching layers', async () => {
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
      configurable: true,
      value: vi.fn(function (this: HTMLElement) {
        const stage = this.closest<HTMLElement>('.view-stage')
        if (stage !== null) stage.scrollTop = 500
      }),
    })
    render(<App />)
    await screen.findByText('Evidence Workbench')
    const stage = screen.getByRole('tabpanel')
    stage.scrollTop = 250

    fireEvent.keyDown(window, { key: '2' })

    await waitFor(() => expect(stage.scrollTop).toBe(500))
  })

  it('keeps Code visible while navigating every evidence type', async () => {
    render(<App />)
    await screen.findByText('Evidence Workbench')

    fireEvent.keyDown(window, { key: 'j' })
    fireEvent.keyDown(window, { key: 'j' })
    fireEvent.keyDown(window, { key: 'j' })

    expect(screen.getByTestId('rendered-diff')).toBeInTheDocument()
    expect(screen.getByText('Relation R1')).toBeInTheDocument()

    fireEvent.keyDown(window, { key: 'k' })
    expect(screen.getByTestId('rendered-diff')).toBeInTheDocument()
    expect(screen.getByText('Byte edit 01')).toBeInTheDocument()
  })

  it('preserves modified browser shortcuts while the evidence drawer is open', async () => {
    vi.mocked(window.matchMedia).mockImplementation((query: string) => mediaQueryResult(query, query === '(max-width: 1279px)'))
    render(<App />)
    await screen.findByText('Evidence Workbench')

    fireEvent.click(screen.getByRole('button', { name: 'Open evidence search and navigation' }))
    const filterButton = screen.getByRole('button', { name: 'Toggle filters' })
    expect(filterButton).toHaveAttribute('aria-expanded', 'false')

    expect(fireEvent.keyDown(window, { key: 'f', ctrlKey: true })).toBe(true)
    expect(filterButton).toHaveAttribute('aria-expanded', 'false')

    expect(fireEvent.keyDown(window, { key: 'f' })).toBe(false)
    expect(filterButton).toHaveAttribute('aria-expanded', 'true')

    filterButton.focus()
    expect(fireEvent.keyDown(window, { key: 'j' })).toBe(false)
    expect(screen.queryByRole('dialog', { name: 'Evidence inspector' })).not.toBeInTheDocument()
    expect(screen.getByTestId('rendered-diff')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Open evidence search and navigation' })).toHaveFocus()
  })

  it('moves focus out of a drawer before hiding it', async () => {
    vi.mocked(window.matchMedia).mockImplementation((query: string) => mediaQueryResult(query, query === '(max-width: 1279px)'))
    render(<App />)
    await screen.findByText('Evidence Workbench')

    const inspectorTrigger = screen.getByRole('button', { name: 'Open evidence inspector' })
    fireEvent.click(inspectorTrigger)
    const inspector = screen.getByRole('dialog', { name: 'Evidence inspector' })
    const close = within(inspector).getByRole('button', { name: 'Close evidence inspector' })
    close.focus()
    fireEvent.click(close)

    expect(inspectorTrigger).toHaveFocus()
    expect(screen.queryByRole('dialog', { name: 'Evidence inspector' })).not.toBeInTheDocument()
  })

  it('clears a desktop search on the first Escape press', async () => {
    render(<App />)
    await screen.findByText('Evidence Workbench')

    fireEvent.keyDown(window, { key: '/' })
    const search = screen.getByRole('textbox', { name: 'Search report' })
    fireEvent.change(search, { target: { value: 'identifier' } })
    expect(search).toHaveValue('identifier')

    fireEvent.keyDown(window, { key: 'Escape' })
    expect(search).toHaveValue('')
  })

  it('shows a clear error when the token is missing', async () => {
    window.history.replaceState({}, '', '/')
    render(<App />)
    expect(await screen.findByText('Could not open this report')).toBeInTheDocument()
    expect(screen.getByText(/Missing viewer session token/)).toBeInTheDocument()
  })
})

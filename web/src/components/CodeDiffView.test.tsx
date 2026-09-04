import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { sessionFixture } from '../test/fixture'
import type { DecodedArtifact, EvidenceSelection } from '../types'
import { CodeDiffView } from './CodeDiffView'

const multiFileDiffSpy = vi.hoisted(() => vi.fn())

vi.mock('@pierre/diffs/react', () => ({
  MultiFileDiff: (props: unknown) => {
    multiFileDiffSpy(props)
    return <div data-testid="mock-multi-file-diff" />
  },
}))

interface MockDiffProps {
  oldFile: { name: string; contents: string }
  newFile: { name: string; contents: string }
  options: {
    enableLineSelection: boolean
    expandUnchanged: boolean
    overflow: 'scroll' | 'wrap'
    lineHoverHighlight: 'disabled' | 'both' | 'number' | 'line'
    onPostRender: (node: HTMLElement, instance: unknown, phase: 'mount' | 'update' | 'unmount') => void
  }
}

function artifact(path: string, text: string): DecodedArtifact {
  return { path, text, bytes: new TextEncoder().encode(text) }
}

function renderCodeDiff(before: DecodedArtifact, after: DecodedArtifact, selection: EvidenceSelection = { type: 'change', index: 0 }) {
  return render(
    <div className="view-stage">
      <CodeDiffView
        before={before}
        after={after}
        diffStyle="split"
        onDiffStyleChange={vi.fn()}
        compact={false}
        report={sessionFixture().report}
        selection={selection}
      />
    </div>,
  )
}

describe('CodeDiffView safety boundaries', () => {
  beforeEach(() => multiFileDiffSpy.mockClear())

  const boundaryPrefix = `${'a'.repeat(383)}😀`

  it.each([
    ['the 2 MiB byte limit', () => `${boundaryPrefix}${'a'.repeat(2 * 1024 * 1024)}`],
    ['the 5,000 line limit', () => `${boundaryPrefix}\n${'x\n'.repeat(4_999)}`],
  ])('uses a bounded byte-safe fallback above %s', (_label, makeSource) => {
    const before = artifact('before.ts', makeSource())
    const after = artifact('after.ts', 'const after = true\n')
    const originalText = before.text
    const originalBytes = before.bytes.slice()
    const originalBytesReference = before.bytes

    const { container } = renderCodeDiff(before, after)

    expect(multiFileDiffSpy).not.toHaveBeenCalled()
    expect(screen.queryByTestId('mock-multi-file-diff')).not.toBeInTheDocument()
    expect(screen.getByText('Source exceeds the interactive Code limit')).toBeInTheDocument()
    expect(screen.getByText(/This bounded preview is byte-safe/)).toBeInTheDocument()
    expect(screen.getByText('Preview limited to 384 bytes. Exact bytes remain available in view 3.')).toBeInTheDocument()
    const preview = container.querySelector('.binary-code-pane.before pre')?.textContent
    expect(preview).toBe(`${'a'.repeat(383)}\\xf0`)
    expect(preview).not.toContain('�')

    expect(before.text).toBe(originalText)
    expect(before.bytes).toBe(originalBytesReference)
    expect(before.bytes.every((byte, index) => byte === originalBytes[index])).toBe(true)
  })

  it('visualizes source and path controls before invoking MultiFileDiff without mutating the artifacts', () => {
    const beforeText = '\ufeffconst marker = "a\0b"\n// \u202e hidden\n'
    const afterText = '\ufeffconst marker = "ab"\n// visible\0\u202e\n'
    const beforePath = 'src/before\0\u202e.ts'
    const afterPath = 'src/after\ufeff\n.ts'
    const before = artifact(beforePath, beforeText)
    const after = artifact(afterPath, afterText)
    const beforeBytes = before.bytes.slice()
    const afterBytes = after.bytes.slice()
    const beforeBytesReference = before.bytes
    const afterBytesReference = after.bytes

    renderCodeDiff(before, after)

    expect(multiFileDiffSpy).toHaveBeenCalledOnce()
    expect(screen.getByTestId('mock-multi-file-diff')).toBeInTheDocument()
    const props = multiFileDiffSpy.mock.calls[0]?.[0] as MockDiffProps | undefined
    if (props === undefined) throw new Error('MultiFileDiff props were not captured.')

    expect(props.oldFile.contents).toBe(
      '⟦BOM U+FEFF⟧const marker = "a⟦NUL U+0000⟧b"\n// ⟦RLO U+202E⟧ hidden\n',
    )
    expect(props.newFile.contents).toBe(
      '⟦BOM U+FEFF⟧const marker = "ab"\n// visible⟦NUL U+0000⟧⟦RLO U+202E⟧\n',
    )
    expect(props.oldFile.name).toBe('src/before⟦NUL U+0000⟧⟦RLO U+202E⟧.ts')
    expect(props.newFile.name).toBe('src/after⟦BOM U+FEFF⟧⟦LF U+000A⟧.ts')
    expect(screen.getByRole('note')).toHaveTextContent('Source bytes remain unchanged')

    expect(before).toMatchObject({ path: beforePath, text: beforeText })
    expect(after).toMatchObject({ path: afterPath, text: afterText })
    expect(before.bytes).toBe(beforeBytesReference)
    expect(after.bytes).toBe(afterBytesReference)
    expect(before.bytes).toEqual(beforeBytes)
    expect(after.bytes).toEqual(afterBytes)
    const bomPreservingDecoder = new TextDecoder('utf-8', { ignoreBOM: true })
    expect(bomPreservingDecoder.decode(before.bytes)).toBe(beforeText)
    expect(bomPreservingDecoder.decode(after.bytes)).toBe(afterText)
  })

  it('keeps escaped binary previews distinct from literal escape text', () => {
    const before: DecodedArtifact = { path: 'nul.bin', text: null, bytes: Uint8Array.from([0]) }
    const after: DecodedArtifact = { path: 'literal.bin', text: null, bytes: new TextEncoder().encode('\\x00') }

    const { container } = renderCodeDiff(before, after)
    const previews = Array.from(container.querySelectorAll('.binary-code-pane pre'), (element) => element.textContent)

    expect(previews).toEqual(['\\x00', '\\\\x00'])
    expect(new Set(previews).size).toBe(2)
  })

  it('exposes focused-context and line-wrapping controls through Pierre options', () => {
    renderCodeDiff(artifact('before.ts', 'const before = 1\n'), artifact('after.ts', 'const after = 2\n'))

    let props = multiFileDiffSpy.mock.calls.at(-1)?.[0] as MockDiffProps | undefined
    if (props === undefined) throw new Error('MultiFileDiff props were not captured.')
    expect(props.options).toMatchObject({
      enableLineSelection: false,
      expandUnchanged: false,
      overflow: 'scroll',
      lineHoverHighlight: 'line',
    })

    fireEvent.click(screen.getByRole('button', { name: 'Expand all unchanged context' }))
    fireEvent.click(screen.getByRole('button', { name: 'Wrap long lines' }))

    props = multiFileDiffSpy.mock.calls.at(-1)?.[0] as MockDiffProps | undefined
    if (props === undefined) throw new Error('MultiFileDiff props were not captured after changing options.')
    expect(props.options).toMatchObject({ expandUnchanged: true, overflow: 'wrap' })
    expect(screen.getByRole('button', { name: 'Collapse unchanged context' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: 'Use horizontal scrolling' })).toHaveAttribute('aria-pressed', 'true')
  })

  it('waits for Pierre to apply controlled selection before revealing evidence', () => {
    const { container } = renderCodeDiff(artifact('before.ts', 'const before = 1\n'), artifact('after.ts', 'const after = 2\n'))
    const stage = container.querySelector<HTMLElement>('.view-stage')
    if (stage === null) throw new Error('View stage was not rendered.')
    vi.spyOn(stage, 'getBoundingClientRect').mockReturnValue({ top: 0, bottom: 100 } as DOMRect)

    let props = multiFileDiffSpy.mock.calls.at(-1)?.[0] as MockDiffProps | undefined
    if (props === undefined) throw new Error('MultiFileDiff props were not captured.')
    const renderedDiff = document.createElement('div')
    const shadowRoot = renderedDiff.attachShadow({ mode: 'open' })
    const staleLine = document.createElement('div')
    staleLine.dataset.selectedLine = 'stale'
    staleLine.scrollIntoView = vi.fn()
    const selectedLine = document.createElement('div')
    vi.spyOn(selectedLine, 'getBoundingClientRect').mockReturnValue({ top: 250, bottom: 271 } as DOMRect)
    selectedLine.scrollIntoView = vi.fn()
    shadowRoot.append(staleLine, selectedLine)
    const reveals: FrameRequestCallback[] = []
    const requestFrame = vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
      reveals.push(callback)
      return reveals.length
    })

    props.options.onPostRender(renderedDiff, {}, 'mount')
    delete staleLine.dataset.selectedLine
    selectedLine.dataset.selectedLine = 'current'
    const reveal = reveals[0]
    if (reveal === undefined) throw new Error('Evidence reveal was not scheduled.')
    reveal(0)

    expect(staleLine.scrollIntoView).not.toHaveBeenCalled()
    expect(selectedLine.scrollIntoView).toHaveBeenCalledWith({ block: 'center', inline: 'nearest' })
    requestFrame.mockRestore()
  })

  it('reveals relation evidence that may be outside collapsed hunks', () => {
    renderCodeDiff(
      artifact('before.ts', 'const before = 1\n'),
      artifact('after.ts', 'const after = 2\n'),
      { type: 'relation', index: 0 },
    )

    const props = multiFileDiffSpy.mock.calls.at(-1)?.[0] as MockDiffProps | undefined
    if (props === undefined) throw new Error('MultiFileDiff props were not captured.')
    expect(props.options.expandUnchanged).toBe(true)
  })
})

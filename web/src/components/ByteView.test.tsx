import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { sessionFixture } from '../test/fixture'
import type { DecodedArtifact } from '../types'
import { ByteView } from './ByteView'

const originalClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard')

afterEach(() => {
  if (originalClipboard === undefined) Reflect.deleteProperty(navigator, 'clipboard')
  else Object.defineProperty(navigator, 'clipboard', originalClipboard)
})

describe('ByteView replacement details', () => {
  it('renders a bounded Base64 preview but copies the complete replacement', async () => {
    const payload = sessionFixture()
    const longBase64 = 'QUJD'.repeat(100)
    const edit = payload.report.patch.edits[0]
    if (edit === undefined) throw new Error('Fixture byte edit is missing.')
    payload.report.patch.edits = [{ ...edit, replacement_base64: longBase64 }]
    const beforeText = 'const before = 1\n'
    const afterText = 'const after = 2\n'
    const before: DecodedArtifact = { path: payload.report.before.path, bytes: new TextEncoder().encode(beforeText), text: beforeText }
    const after: DecodedArtifact = { path: payload.report.after.path, bytes: new TextEncoder().encode(afterText), text: afterText }
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })

    const { container } = render(
      <ByteView
        report={payload.report}
        before={before}
        after={after}
        selection={{ type: 'edit', index: 0 }}
        onSelect={vi.fn()}
      />,
    )
    const preview = container.querySelector('.replacement-detail code')
    if (preview === null) throw new Error('Replacement preview was not rendered.')

    expect(preview.textContent).toBe(`${longBase64.slice(0, 160)}… (400 Base64 characters)`)
    expect(container.textContent).not.toContain(longBase64)

    fireEvent.click(screen.getByRole('button', { name: 'Copy Base64' }))
    await waitFor(() => expect(writeText).toHaveBeenCalledOnce())
    expect(writeText).toHaveBeenCalledWith(longBase64)
    expect(await screen.findByRole('button', { name: 'Copied' })).toBeInTheDocument()
  })
})

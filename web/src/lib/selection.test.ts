import { describe, expect, it } from 'vitest'
import { editAfterRanges } from './format'
import { buildByteLineIndex, evidenceByteRanges, evidenceLineSelection } from './selection'
import { sessionFixture } from '../test/fixture'

describe('cross-layer evidence selection', () => {
  it('maps a structural event to both source byte ranges and one-based lines', () => {
    const payload = sessionFixture()
    const ranges = evidenceByteRanges(
      payload.report,
      { type: 'change', index: 0 },
      editAfterRanges(payload.report.patch.edits),
    )
    expect(ranges).toEqual({ before: [6, 12], after: [6, 11] })
    expect(
      evidenceLineSelection(
        ranges,
        buildByteLineIndex(new TextEncoder().encode('const before = 1\n')),
        buildByteLineIndex(new TextEncoder().encode('const after = 2\n')),
      ),
    ).toEqual({ start: 1, side: 'deletions', end: 1, endSide: 'additions' })
  })

  it('maps an exact edit to its length-adjusted replay range', () => {
    const payload = sessionFixture()
    const ranges = evidenceByteRanges(
      payload.report,
      { type: 'edit', index: 0 },
      editAfterRanges(payload.report.patch.edits),
    )
    expect(ranges).toEqual({ before: [6, 16], after: [6, 15] })
  })
})

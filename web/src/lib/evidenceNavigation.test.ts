import { describe, expect, it } from 'vitest'
import { sessionFixture } from '../test/fixture'
import {
  buildEvidenceNavigation,
  buildEvidenceSearchIndex,
  matchCount,
  matchPosition,
  pageIndices,
  stepEvidence,
} from './evidenceNavigation'

describe('evidence navigation', () => {
  it('uses implicit ranges for an empty query and wraps across sections', () => {
    const report = sessionFixture().report
    const searchIndex = buildEvidenceSearchIndex(report, '')
    const navigation = buildEvidenceNavigation(report, searchIndex, { changes: true, ambiguities: true, edits: true })

    expect(Object.values(searchIndex).every((indices) => indices === null)).toBe(true)
    expect(navigation.segments.every((segment) => segment.indices === null)).toBe(true)
    expect(navigation.total).toBe(report.changes.length + report.ambiguities.length + report.patch.edits.length + report.relations.length)
    expect(stepEvidence(navigation, { type: 'change', index: 0 }, 1)).toEqual({ type: 'ambiguity', index: 0 })
    expect(stepEvidence(navigation, { type: 'change', index: 0 }, -1)).toEqual({ type: 'relation', index: report.relations.length - 1 })
  })

  it('honors filters and searches sparse indices', () => {
    const report = sessionFixture().report
    const relation = report.relations[0]
    const change = report.changes[0]
    if (relation === undefined || change === undefined) throw new Error('Fixture evidence is missing.')
    report.relations = [relation, { ...relation, evidence: ['needle'] }, relation]
    report.changes = [change, change, { ...change, detail: 'needle' }]
    const searchIndex = buildEvidenceSearchIndex(report, 'needle')
    const navigation = buildEvidenceNavigation(report, searchIndex, { changes: false, ambiguities: false, edits: false })

    expect(searchIndex.relations).toEqual([1])
    expect(searchIndex.changes).toEqual([2])
    expect(navigation.total).toBe(1)
    expect(stepEvidence(navigation, { type: 'change', index: 2 }, 1)).toEqual({ type: 'relation', index: 1 })
    expect(stepEvidence(navigation, { type: 'relation', index: 1 }, 1)).toEqual({ type: 'relation', index: 1 })

    const fullNavigation = buildEvidenceNavigation(report, searchIndex, { changes: true, ambiguities: true, edits: true })
    expect(stepEvidence(fullNavigation, { type: 'ambiguity', index: 0 }, 1)).toEqual({ type: 'change', index: 2 })
    expect(stepEvidence(fullNavigation, { type: 'ambiguity', index: 0 }, -1)).toEqual({ type: 'relation', index: 1 })
  })

  it('pages implicit and sparse matches without materializing the full range', () => {
    expect(matchCount(null, 11)).toBe(11)
    expect(pageIndices(null, 11, 0, 10)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
    expect(pageIndices(null, 11, 10, 10)).toEqual([10])
    expect(pageIndices(null, 11, 11, 10)).toEqual([])
    expect(pageIndices([1, 3, 10], 11, 2, 10)).toEqual([10])
    expect(matchPosition(null, 11, 10)).toBe(10)
    expect(matchPosition([1, 3, 10], 11, 3)).toBe(1)
    expect(matchPosition([1, 3, 10], 11, 2)).toBe(-1)
  })
})

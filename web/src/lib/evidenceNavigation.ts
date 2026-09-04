import type { DiffReport, EvidenceSelection } from '../types'
import { relationMatchesNormalizedQuery } from './evidenceSearch'

export interface EvidenceFilters {
  changes: boolean
  ambiguities: boolean
  edits: boolean
}

export type MatchingIndices = number[] | null

export interface EvidenceSearchIndex {
  changes: MatchingIndices
  ambiguities: MatchingIndices
  edits: MatchingIndices
  relations: MatchingIndices
}

interface NavigationSegment {
  type: EvidenceSelection['type']
  indices: MatchingIndices
  sourceLength: number
}

export interface EvidenceNavigation {
  segments: NavigationSegment[]
  total: number
}

function matchingIndices(length: number, normalizedQuery: string, matches: (index: number) => boolean): MatchingIndices {
  if (normalizedQuery.length === 0) return null
  const output: number[] = []
  for (let index = 0; index < length; index += 1) {
    if (matches(index)) output.push(index)
  }
  return output
}

export function matchCount(indices: MatchingIndices, sourceLength: number): number {
  return indices === null ? sourceLength : indices.length
}

export function pageIndices(indices: MatchingIndices, sourceLength: number, start: number, limit: number): number[] {
  if (indices !== null) return indices.slice(start, start + limit)
  const end = Math.min(sourceLength, start + limit)
  return Array.from({ length: Math.max(0, end - start) }, (_, offset) => start + offset)
}

function segmentLength(segment: NavigationSegment): number {
  return matchCount(segment.indices, segment.sourceLength)
}

function itemIndexAt(segment: NavigationSegment, position: number): number {
  if (segment.indices === null) return position
  const index = segment.indices[position]
  if (index === undefined) throw new Error(`Evidence index ${position} is missing from ${segment.type}.`)
  return index
}

function findSortedIndex(indices: number[], target: number): number {
  let low = 0
  let high = indices.length
  while (low < high) {
    const middle = low + Math.floor((high - low) / 2)
    const value = indices[middle]
    if (value === undefined) throw new Error(`Evidence index ${middle} is missing.`)
    if (value < target) low = middle + 1
    else high = middle
  }
  return indices[low] === target ? low : -1
}

export function matchPosition(indices: MatchingIndices, sourceLength: number, target: number): number {
  if (indices === null) return target >= 0 && target < sourceLength ? target : -1
  return findSortedIndex(indices, target)
}

export function buildEvidenceSearchIndex(report: DiffReport, query: string): EvidenceSearchIndex {
  const normalizedQuery = query.trim().toLocaleLowerCase()
  const changes = matchingIndices(report.changes.length, normalizedQuery, (index) => {
    const change = report.changes[index]
    if (change === undefined) throw new Error(`Structural change ${index} is missing.`)
    return [change.kind, change.detail, change.before?.kind ?? '', change.after?.kind ?? ''].join(' ').toLocaleLowerCase().includes(normalizedQuery)
  })
  const ambiguities = matchingIndices(report.ambiguities.length, normalizedQuery, (index) => {
    const ambiguity = report.ambiguities[index]
    if (ambiguity === undefined) throw new Error(`Ambiguity ${index} is missing.`)
    return [ambiguity.reason, ambiguity.constraint.kind].join(' ').toLocaleLowerCase().includes(normalizedQuery)
  })
  const edits = matchingIndices(report.patch.edits.length, normalizedQuery, (index) => {
    const edit = report.patch.edits[index]
    if (edit === undefined) throw new Error(`Byte edit ${index} is missing.`)
    return [`edit ${index + 1}`, edit.old_start, edit.old_end].join(' ').toLocaleLowerCase().includes(normalizedQuery)
  })
  const relations = matchingIndices(report.relations.length, normalizedQuery, (index) => {
    const relation = report.relations[index]
    if (relation === undefined) throw new Error(`Relation ${index} is missing.`)
    return relationMatchesNormalizedQuery(relation, index, normalizedQuery)
  })
  return { changes, ambiguities, edits, relations }
}

export function buildEvidenceNavigation(report: DiffReport, searchIndex: EvidenceSearchIndex, filters: EvidenceFilters): EvidenceNavigation {
  const segments: NavigationSegment[] = [
    { type: 'change', indices: filters.changes ? searchIndex.changes : [], sourceLength: report.changes.length },
    { type: 'ambiguity', indices: filters.ambiguities ? searchIndex.ambiguities : [], sourceLength: report.ambiguities.length },
    { type: 'edit', indices: filters.edits ? searchIndex.edits : [], sourceLength: report.patch.edits.length },
    { type: 'relation', indices: searchIndex.relations, sourceLength: report.relations.length },
  ]
  return { segments, total: segments.reduce((total, segment) => total + segmentLength(segment), 0) }
}

export function stepEvidence(navigation: EvidenceNavigation, current: EvidenceSelection, direction: 1 | -1): EvidenceSelection | null {
  if (navigation.total === 0) return null
  let offset = 0
  let currentPosition = -1
  for (const segment of navigation.segments) {
    if (segment.type === current.type) {
      const withinSegment = matchPosition(segment.indices, segment.sourceLength, current.index)
      if (withinSegment >= 0) currentPosition = offset + withinSegment
      break
    }
    offset += segmentLength(segment)
  }
  let nextPosition: number
  if (currentPosition < 0) nextPosition = direction === 1 ? 0 : navigation.total - 1
  else nextPosition = (currentPosition + direction + navigation.total) % navigation.total
  let remaining = nextPosition
  for (const segment of navigation.segments) {
    const length = segmentLength(segment)
    if (remaining < length) return { type: segment.type, index: itemIndexAt(segment, remaining) } as EvidenceSelection
    remaining -= length
  }
  throw new Error(`Evidence position ${nextPosition} is outside the navigation index.`)
}

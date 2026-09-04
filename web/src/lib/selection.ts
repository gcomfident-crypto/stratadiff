import type { SelectedLineRange } from '@pierre/diffs/react'
import type { DiffReport, EvidenceSelection, NodeRef } from '../types'

export type ByteRange = [number, number]

export interface EvidenceByteRanges {
  before: ByteRange | null
  after: ByteRange | null
}

export interface ByteLineIndex {
  byteLength: number
  lineStarts: number[]
}

function nodeRange(node: NodeRef | undefined): ByteRange | null {
  return node === undefined ? null : [node.span.start_byte, node.span.end_byte]
}

function coveringRange(nodes: NodeRef[]): ByteRange | null {
  const first = nodes[0]
  if (first === undefined) return null
  let start = first.span.start_byte
  let end = first.span.end_byte
  for (let index = 1; index < nodes.length; index += 1) {
    const node = nodes[index]
    if (node === undefined) throw new Error(`Endpoint ${index} is missing.`)
    start = Math.min(start, node.span.start_byte)
    end = Math.max(end, node.span.end_byte)
  }
  return [start, end]
}

export function evidenceByteRanges(
  report: DiffReport,
  selection: EvidenceSelection,
  editAfterRanges: ByteRange[],
): EvidenceByteRanges {
  if (selection.type === 'relation') {
    const relation = report.relations[selection.index]
    if (relation === undefined) throw new Error(`Relation ${selection.index} is missing.`)
    return { before: nodeRange(relation.before), after: nodeRange(relation.after) }
  }
  if (selection.type === 'change') {
    const change = report.changes[selection.index]
    if (change === undefined) throw new Error(`Structural change ${selection.index} is missing.`)
    return { before: nodeRange(change.before), after: nodeRange(change.after) }
  }
  if (selection.type === 'ambiguity') {
    const ambiguity = report.ambiguities[selection.index]
    if (ambiguity === undefined) throw new Error(`Ambiguity ${selection.index} is missing.`)
    return { before: coveringRange(ambiguity.before), after: coveringRange(ambiguity.after) }
  }

  const edit = report.patch.edits[selection.index]
  const after = editAfterRanges[selection.index]
  if (edit === undefined || after === undefined) throw new Error(`Byte edit ${selection.index} is missing.`)
  return {
    before: edit.old_start === edit.old_end ? null : [edit.old_start, edit.old_end],
    after: after[0] === after[1] ? null : after,
  }
}

export function buildByteLineIndex(bytes: Uint8Array): ByteLineIndex {
  const lineStarts = [0]
  for (let offset = 0; offset < bytes.byteLength; offset += 1) {
    if (bytes[offset] === 0x0a) lineStarts.push(offset + 1)
  }
  return { byteLength: bytes.byteLength, lineStarts }
}

function lineAtOffset(index: ByteLineIndex, offset: number): number {
  let low = 0
  let high = index.lineStarts.length
  while (low + 1 < high) {
    const middle = low + Math.floor((high - low) / 2)
    const candidate = index.lineStarts[middle]
    if (candidate === undefined) throw new Error(`Line index ${middle} is missing.`)
    if (candidate <= offset) low = middle
    else high = middle
  }
  return low + 1
}

function byteRangeToLines(index: ByteLineIndex, range: ByteRange): [number, number] {
  const [start, end] = range
  if (start > end || end > index.byteLength) {
    throw new Error(`Evidence byte range [${start}, ${end}) is outside a ${index.byteLength}-byte source.`)
  }
  const lastByte = end > start ? end - 1 : start
  return [lineAtOffset(index, start), lineAtOffset(index, lastByte)]
}

export function evidenceLineSelection(
  ranges: EvidenceByteRanges,
  before: ByteLineIndex,
  after: ByteLineIndex,
): SelectedLineRange | null {
  const beforeLines = ranges.before === null ? null : byteRangeToLines(before, ranges.before)
  const afterLines = ranges.after === null ? null : byteRangeToLines(after, ranges.after)
  if (beforeLines !== null && afterLines !== null) {
    return {
      start: beforeLines[0],
      side: 'deletions',
      end: afterLines[1],
      endSide: 'additions',
    }
  }
  if (beforeLines !== null) {
    return { start: beforeLines[0], end: beforeLines[1], side: 'deletions' }
  }
  if (afterLines !== null) {
    return { start: afterLines[0], end: afterLines[1], side: 'additions' }
  }
  return null
}

import type { ByteEdit, NodeRef, Predicate } from '../types'

export function compactNumber(value: number): string {
  return new Intl.NumberFormat('en', { notation: value > 9_999 ? 'compact' : 'standard' }).format(value)
}

export function titleCase(value: string): string {
  return value
    .split('_')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ')
}

export function predicateLabel(predicate: Predicate): string {
  const labels: Record<Predicate, string> = {
    input_pair: 'Input pair',
    byte_equal: 'Byte equal',
    syntax_equal: 'Syntax equal',
    shape_equal: 'Shape equal',
  }
  return labels[predicate]
}

export function nodeLabel(node: NodeRef): string {
  return `${node.kind} #${node.id}`
}

export function nodeLocation(node: NodeRef): string {
  const start = node.span.start
  const end = node.span.end
  return `L${start.row + 1} byte-col ${start.column}–L${end.row + 1} byte-col ${end.column}`
}

export function byteRange(start: number, end: number): string {
  return `[${start}, ${end})`
}

export function shortHash(value: string): string {
  return `${value.slice(0, 8)}…${value.slice(-6)}`
}

export function base64DecodedLength(value: string): number {
  if (value.length === 0) return 0
  if (value.length % 4 !== 0) throw new Error('Verified replacement Base64 has an invalid length.')
  const padding = value.endsWith('==') ? 2 : value.endsWith('=') ? 1 : 0
  return (value.length / 4) * 3 - padding
}

export function base64Preview(value: string, maxCharacters = 160): string {
  if (value.length === 0) return '∅'
  if (value.length <= maxCharacters) return value
  return `${value.slice(0, maxCharacters)}… (${value.length.toLocaleString('en')} Base64 characters)`
}

export function editAfterRanges(edits: ByteEdit[]): Array<[number, number]> {
  let delta = 0
  return edits.map((edit) => {
    const replacementLength = base64DecodedLength(edit.replacement_base64)
    const start = edit.old_start + delta
    delta += replacementLength - (edit.old_end - edit.old_start)
    return [start, start + replacementLength]
  })
}

export function editAfterRange(edits: ByteEdit[], index: number): [number, number] {
  const range = editAfterRanges(edits)[index]
  if (range === undefined) throw new Error(`Byte edit ${index} is missing.`)
  return range
}

export function formatBytes(value: number): string {
  if (value < 1_024) return `${value} B`
  if (value < 1_048_576) return `${(value / 1_024).toFixed(1)} KiB`
  return `${(value / 1_048_576).toFixed(1)} MiB`
}
